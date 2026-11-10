use super::link::{self, Lane, Profile, Rng, SEGMENT};
use std::io::{ErrorKind, Read, Write};
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

pub const QUEUE: usize = 64;

type Segment = (Duration, Vec<u8>);

#[derive(Clone)]
pub struct Emulator {
    pub port: u16,
    profile: Arc<Mutex<Profile>>,
}

struct Pipe {
    profile: Arc<Mutex<Profile>>,
    epoch: Instant,
    seed: u64,
}

impl Emulator {
    pub fn start(upstream: u16, profile: Profile) -> Result<Emulator, String> {
        let listener: TcpListener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|e| format!("emulator bind: {e}"))?;
        let port: u16 = listener
            .local_addr()
            .map_err(|e| format!("emulator address: {e}"))?
            .port();
        let emulator: Emulator = Emulator {
            port,
            profile: Arc::new(Mutex::new(profile)),
        };
        let shared: Arc<Mutex<Profile>> = Arc::clone(&emulator.profile);
        thread::spawn(move || accept_all(&listener, upstream, &shared));
        Ok(emulator)
    }

    pub fn set(&self, profile: Profile) {
        *self.profile.lock().unwrap_or_else(PoisonError::into_inner) = profile;
    }
}

fn accept_all(listener: &TcpListener, upstream: u16, profile: &Arc<Mutex<Profile>>) {
    let seeds: AtomicU64 = AtomicU64::new(1);
    let epoch: Instant = Instant::now();
    for client in listener.incoming() {
        let joined: Result<(), String> =
            client
                .map_err(|e| format!("accept: {e}"))
                .and_then(|client: TcpStream| {
                    let server: TcpStream = TcpStream::connect((Ipv4Addr::LOCALHOST, upstream))
                        .map_err(|e| format!("connect 127.0.0.1:{upstream}: {e}"))?;
                    [&client, &server]
                        .iter()
                        .try_for_each(|stream: &&TcpStream| stream.set_nodelay(true))
                        .map_err(|e| format!("TCP_NODELAY: {e}"))?;
                    let pipe = |seed: u64| Pipe {
                        profile: Arc::clone(profile),
                        epoch,
                        seed,
                    };
                    let up: Pipe = pipe(seeds.fetch_add(1, Ordering::Relaxed));
                    let down: Pipe = pipe(seeds.fetch_add(1, Ordering::Relaxed));
                    let (client_read, server_read): (TcpStream, TcpStream) = (
                        client.try_clone().map_err(|e| e.to_string())?,
                        server.try_clone().map_err(|e| e.to_string())?,
                    );
                    thread::spawn(move || up.carry(client_read, server));
                    thread::spawn(move || down.carry(server_read, client));
                    Ok(())
                });
        if let Err(error) = joined {
            eprintln!("emulator: {error}");
        }
    }
}

impl Pipe {
    fn carry(self, from: TcpStream, to: TcpStream) {
        let (segments, queue): (SyncSender<Segment>, Receiver<Segment>) = mpsc::sync_channel(QUEUE);
        let epoch: Instant = self.epoch;
        let writer: thread::JoinHandle<()> = thread::spawn(move || deliver(&queue, to, epoch));
        self.schedule(from, &segments);
        drop(segments);
        if writer.join().is_err() {
            eprintln!("emulator: a writer panicked");
        }
    }

    fn schedule(&self, mut from: TcpStream, segments: &SyncSender<Segment>) {
        let mut rng: Rng = Rng::seeded(self.seed);
        let mut lane: Lane = Lane::default();
        let mut buffer: [u8; SEGMENT] = [0; SEGMENT];
        loop {
            let read: usize = match from.read(&mut buffer) {
                Ok(0) => return,
                Ok(read) => read,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(_) => return,
            };
            let profile: Profile = self
                .profile
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            lane = link::schedule(
                lane,
                &profile,
                self.epoch.elapsed(),
                read,
                rng.draw(profile.loss_ppm),
            );
            if segments
                .send((lane.last_out, buffer[..read].to_vec()))
                .is_err()
            {
                return;
            }
        }
    }
}

fn deliver(queue: &Receiver<Segment>, mut to: TcpStream, epoch: Instant) {
    for (out, bytes) in queue {
        thread::sleep(out.saturating_sub(epoch.elapsed()));
        if to.write_all(&bytes).is_err() {
            break;
        }
    }
    if let Err(error) = to.shutdown(Shutdown::Write)
        && error.kind() != ErrorKind::NotConnected
    {
        eprintln!("emulator: shutdown: {error}");
    }
}

fn drain(sink: &TcpListener) -> Result<(Duration, usize), String> {
    let (mut stream, _): (TcpStream, _) = sink.accept().map_err(|e| format!("sink accept: {e}"))?;
    let mut buffer: Vec<u8> = vec![0; SEGMENT * QUEUE];
    let mut first: Option<(Instant, usize)> = None;
    let mut last: Instant = Instant::now();
    let mut total: usize = 0;
    loop {
        let read: usize = stream
            .read(&mut buffer)
            .map_err(|e| format!("sink read: {e}"))?;
        if read == 0 {
            break;
        }
        last = Instant::now();
        first.get_or_insert((last, read));
        total += read;
    }
    let (start, head): (Instant, usize) = first.ok_or("the sink received nothing")?;
    Ok((last.duration_since(start), total - head))
}

pub fn throughput(profile: Profile, bytes: usize) -> Result<f64, String> {
    let sink: TcpListener =
        TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| format!("sink bind: {e}"))?;
    let sink_port: u16 = sink.local_addr().map_err(|e| e.to_string())?.port();
    let emulator: Emulator = Emulator::start(sink_port, profile)?;
    let receiver: thread::JoinHandle<Result<(Duration, usize), String>> =
        thread::spawn(move || drain(&sink));
    let mut client: TcpStream = TcpStream::connect((Ipv4Addr::LOCALHOST, emulator.port))
        .map_err(|e| format!("connect the emulator: {e}"))?;
    client
        .write_all(&vec![0; bytes])
        .and_then(|()| client.shutdown(Shutdown::Write))
        .map_err(|e| format!("send through the emulator: {e}"))?;
    let (span, counted): (Duration, usize) = receiver
        .join()
        .map_err(|_| String::from("the sink panicked"))??;
    Ok(counted as f64 * 8.0 / span.as_secs_f64())
}

#[cfg(test)]
mod tests;
