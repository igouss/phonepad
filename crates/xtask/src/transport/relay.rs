use crate::latency::link::{self, Lane, Profile, Rng};
use std::collections::VecDeque;
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

pub const QUEUE: usize = 64;
pub const DATAGRAM: usize = 65_536;
const POLL: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bottleneck {
    pub lane: Lane,
    pub queued: VecDeque<Duration>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fate {
    Overflow,
    Lost,
    Deliver(Duration),
}

pub fn admit(
    mut bottleneck: Bottleneck,
    profile: &Profile,
    arrival: Duration,
    bytes: usize,
    draw: link::Draw,
) -> (Bottleneck, Fate) {
    while bottleneck
        .queued
        .front()
        .is_some_and(|free_at: &Duration| *free_at <= arrival)
    {
        bottleneck.queued.pop_front();
    }
    if bottleneck.queued.len() >= QUEUE {
        return (bottleneck, Fate::Overflow);
    }
    let free_at: Duration =
        arrival.max(bottleneck.lane.free_at) + link::serialization(bytes, profile.rate);
    bottleneck.queued.push_back(free_at);
    if draw.lost {
        bottleneck.lane.free_at = free_at;
        return (bottleneck, Fate::Lost);
    }
    let out: Duration =
        (free_at + link::one_way(profile, draw.jitter)).max(bottleneck.lane.last_out);
    bottleneck.lane = Lane {
        free_at,
        last_out: out,
    };
    (bottleneck, Fate::Deliver(out))
}

#[derive(Debug, Default)]
pub struct Counts {
    pub carried: AtomicU64,
    pub lost: AtomicU64,
    pub overflowed: AtomicU64,
}

impl Counts {
    pub fn snapshot(&self) -> (u64, u64, u64) {
        (
            self.carried.load(Ordering::Relaxed),
            self.lost.load(Ordering::Relaxed),
            self.overflowed.load(Ordering::Relaxed),
        )
    }
}

pub struct Relay {
    pub port: u16,
    profile: Arc<Mutex<Profile>>,
    pub up: Arc<Counts>,
    pub down: Arc<Counts>,
    stop: Arc<AtomicBool>,
}

struct Direction {
    profile: Arc<Mutex<Profile>>,
    epoch: Instant,
    seed: u64,
    counts: Arc<Counts>,
    stop: Arc<AtomicBool>,
}

type Datagram = (Duration, Vec<u8>);

impl Relay {
    pub fn start(upstream: u16, profile: Profile) -> Result<Relay, String> {
        let front: UdpSocket =
            UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| format!("relay bind: {e}"))?;
        let back: UdpSocket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|e| format!("relay upstream bind: {e}"))?;
        back.connect((Ipv4Addr::LOCALHOST, upstream))
            .map_err(|e| format!("relay connect 127.0.0.1:{upstream}: {e}"))?;
        [&front, &back]
            .iter()
            .try_for_each(|socket: &&UdpSocket| socket.set_read_timeout(Some(POLL)))
            .map_err(|e| format!("relay timeout: {e}"))?;
        let port: u16 = front.local_addr().map_err(|e| e.to_string())?.port();
        let relay: Relay = Relay {
            port,
            profile: Arc::new(Mutex::new(profile)),
            up: Arc::new(Counts::default()),
            down: Arc::new(Counts::default()),
            stop: Arc::new(AtomicBool::new(false)),
        };
        let epoch: Instant = Instant::now();
        let direction = |seed: u64, counts: &Arc<Counts>| Direction {
            profile: Arc::clone(&relay.profile),
            epoch,
            seed,
            counts: Arc::clone(counts),
            stop: Arc::clone(&relay.stop),
        };
        let peer: Arc<Mutex<Option<SocketAddr>>> = Arc::new(Mutex::new(None));
        let clone = |socket: &UdpSocket| socket.try_clone().map_err(|e| e.to_string());
        let (front_in, front_out, back_in, back_out): (UdpSocket, UdpSocket, UdpSocket, UdpSocket) =
            (clone(&front)?, front, clone(&back)?, back);
        let up: Direction = direction(1, &relay.up);
        let down: Direction = direction(2, &relay.down);
        let heard: Arc<Mutex<Option<SocketAddr>>> = Arc::clone(&peer);
        thread::spawn(move || {
            up.carry(
                |buffer: &mut [u8]| {
                    front_in
                        .recv_from(buffer)
                        .map(|(read, from): (usize, SocketAddr)| {
                            *heard.lock().unwrap_or_else(PoisonError::into_inner) = Some(from);
                            read
                        })
                },
                move |bytes: &[u8]| back_out.send(bytes).map(|_| ()),
            )
        });
        thread::spawn(move || {
            down.carry(
                |buffer: &mut [u8]| back_in.recv(buffer),
                move |bytes: &[u8]| {
                    let to: Option<SocketAddr> =
                        *peer.lock().unwrap_or_else(PoisonError::into_inner);
                    to.map_or(Ok(()), |to: SocketAddr| {
                        front_out.send_to(bytes, to).map(|_| ())
                    })
                },
            )
        });
        Ok(relay)
    }

    pub fn set(&self, profile: Profile) {
        *self.profile.lock().unwrap_or_else(PoisonError::into_inner) = profile;
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn quiet(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
    )
}

impl Direction {
    fn carry<R, W>(self, mut receive: R, send: W)
    where
        R: FnMut(&mut [u8]) -> std::io::Result<usize>,
        W: FnMut(&[u8]) -> std::io::Result<()> + Send + 'static,
    {
        let (datagrams, queue): (Sender<Datagram>, Receiver<Datagram>) = mpsc::channel();
        let epoch: Instant = self.epoch;
        let writer: thread::JoinHandle<()> = thread::spawn(move || deliver(&queue, send, epoch));
        let mut rng: Rng = Rng::seeded(self.seed);
        let mut bottleneck: Bottleneck = Bottleneck::default();
        let mut buffer: Vec<u8> = vec![0; DATAGRAM];
        while !self.stop.load(Ordering::Relaxed) {
            let read: usize = match receive(&mut buffer) {
                Ok(read) => read,
                Err(error) if quiet(&error) => continue,
                Err(_) => break,
            };
            let profile: Profile = self
                .profile
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            let fate: Fate;
            (bottleneck, fate) = admit(
                bottleneck,
                &profile,
                epoch.elapsed(),
                read,
                rng.draw(profile.loss_ppm),
            );
            match fate {
                Fate::Overflow => {
                    self.counts.overflowed.fetch_add(1, Ordering::Relaxed);
                }
                Fate::Lost => {
                    self.counts.lost.fetch_add(1, Ordering::Relaxed);
                }
                Fate::Deliver(out) => {
                    self.counts.carried.fetch_add(1, Ordering::Relaxed);
                    if datagrams.send((out, buffer[..read].to_vec())).is_err() {
                        break;
                    }
                }
            }
        }
        drop(datagrams);
        if writer.join().is_err() {
            eprintln!("relay: a writer panicked");
        }
    }
}

fn deliver<W>(queue: &Receiver<Datagram>, mut send: W, epoch: Instant)
where
    W: FnMut(&[u8]) -> std::io::Result<()>,
{
    for (out, bytes) in queue {
        thread::sleep(out.saturating_sub(epoch.elapsed()));
        if let Err(error) = send(&bytes)
            && !quiet(&error)
            && error.kind() != ErrorKind::ConnectionRefused
        {
            eprintln!("relay: send: {error}");
        }
    }
}

#[cfg(test)]
mod tests;
