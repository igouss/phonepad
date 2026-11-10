use crate::latency::stats::{self, Spread};
use std::io::ErrorKind;
use std::net::{Ipv4Addr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

const PROBE_BYTES: usize = 200;
const SEQ: usize = 4;
const POLL: Duration = Duration::from_millis(50);
type Tally = (usize, usize, Option<(Instant, Instant)>);

const DRAIN: Duration = Duration::from_millis(800);

pub struct Echo {
    pub port: u16,
    stop: Arc<AtomicBool>,
}

impl Echo {
    pub fn start() -> Result<Echo, String> {
        let socket: UdpSocket =
            UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| format!("echo bind: {e}"))?;
        socket
            .set_read_timeout(Some(POLL))
            .map_err(|e| e.to_string())?;
        let port: u16 = socket.local_addr().map_err(|e| e.to_string())?.port();
        let stop: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
        let stopping: Arc<AtomicBool> = Arc::clone(&stop);
        thread::spawn(move || {
            let mut buffer: [u8; 2048] = [0; 2048];
            while !stopping.load(Ordering::Relaxed) {
                if let Ok((read, from)) = socket.recv_from(&mut buffer)
                    && socket.send_to(&buffer[..read], from).is_err()
                {
                    break;
                }
            }
        });
        Ok(Echo { port, stop })
    }
}

impl Drop for Echo {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub struct Echoed {
    pub rtt: Option<Spread>,
    pub p5: Option<f64>,
    pub min: Option<f64>,
    pub sent: usize,
    pub returned: usize,
}

fn quiet(error: &std::io::Error) -> bool {
    matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
}

pub fn echo(port: u16, count: usize, every: Duration) -> Result<Echoed, String> {
    let socket: UdpSocket =
        UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| format!("probe bind: {e}"))?;
    socket
        .connect((Ipv4Addr::LOCALHOST, port))
        .map_err(|e| e.to_string())?;
    socket
        .set_read_timeout(Some(POLL))
        .map_err(|e| e.to_string())?;
    let reader: UdpSocket = socket.try_clone().map_err(|e| e.to_string())?;
    let epoch: Instant = Instant::now();
    let sent_at: Arc<Mutex<Vec<Duration>>> = Arc::new(Mutex::new(Vec::with_capacity(count)));
    let done: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let listening: Arc<AtomicBool> = Arc::clone(&done);
    let stamps: Arc<Mutex<Vec<Duration>>> = Arc::clone(&sent_at);
    let listener: thread::JoinHandle<Vec<f64>> = thread::spawn(move || {
        let mut rtts: Vec<f64> = Vec::new();
        let mut buffer: [u8; PROBE_BYTES] = [0; PROBE_BYTES];
        while !listening.load(Ordering::Relaxed) {
            match reader.recv(&mut buffer) {
                Ok(read) if read >= SEQ => {
                    let at: Duration = epoch.elapsed();
                    let seq: usize =
                        u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) as usize;
                    let sent: Option<Duration> = stamps
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .get(seq)
                        .copied();
                    if let Some(sent) = sent {
                        rtts.push(at.saturating_sub(sent).as_secs_f64() * 1e3);
                    }
                }
                Ok(_) => {}
                Err(error) if quiet(&error) => {}
                Err(_) => break,
            }
        }
        rtts
    });
    let mut payload: [u8; PROBE_BYTES] = [0; PROBE_BYTES];
    for seq in 0..count {
        let index: u32 = u32::try_from(seq).map_err(|e| e.to_string())?;
        payload[..SEQ].copy_from_slice(&index.to_be_bytes());
        sent_at
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(epoch.elapsed());
        socket
            .send(&payload)
            .map_err(|e| format!("probe send: {e}"))?;
        thread::sleep(every);
    }
    thread::sleep(DRAIN);
    done.store(true, Ordering::Relaxed);
    let mut rtts: Vec<f64> = listener
        .join()
        .map_err(|_| String::from("the probe listener panicked"))?;
    rtts.sort_by(f64::total_cmp);
    let p5: Option<f64> = rtts.get(rtts.len() / 20).copied();
    Ok(Echoed {
        min: rtts.first().copied(),
        p5,
        returned: rtts.len(),
        rtt: stats::spread(rtts),
        sent: count,
    })
}

pub struct Sink {
    pub port: u16,
    received: thread::JoinHandle<Tally>,
    stop: Arc<AtomicBool>,
}

impl Sink {
    pub fn start() -> Result<Sink, String> {
        let socket: UdpSocket =
            UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| format!("sink bind: {e}"))?;
        socket
            .set_read_timeout(Some(POLL))
            .map_err(|e| e.to_string())?;
        let port: u16 = socket.local_addr().map_err(|e| e.to_string())?.port();
        let stop: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
        let stopping: Arc<AtomicBool> = Arc::clone(&stop);
        let received = thread::spawn(move || {
            let mut buffer: Vec<u8> = vec![0; 65_536];
            let mut count: usize = 0;
            let mut bytes: usize = 0;
            let mut span: Option<(Instant, Instant)> = None;
            while !stopping.load(Ordering::Relaxed) {
                match socket.recv(&mut buffer) {
                    Ok(read) => {
                        let now: Instant = Instant::now();
                        count += 1;
                        if span.is_some() {
                            bytes += read;
                        }
                        span = Some(span.map_or((now, now), |(first, _)| (first, now)));
                    }
                    Err(error) if quiet(&error) => {}
                    Err(_) => break,
                }
            }
            (count, bytes, span)
        });
        Ok(Sink {
            port,
            received,
            stop,
        })
    }

    pub fn finish(self) -> Result<(usize, f64), String> {
        thread::sleep(DRAIN);
        self.stop.store(true, Ordering::Relaxed);
        let (count, bytes, span) = self
            .received
            .join()
            .map_err(|_| String::from("the sink panicked"))?;
        let mbit: f64 = span.map_or(0.0, |(first, last): (Instant, Instant)| {
            bytes as f64 * 8.0 / last.duration_since(first).as_secs_f64().max(1e-9) / 1e6
        });
        Ok((count, mbit))
    }
}

pub fn blast(port: u16, count: usize, bytes: usize, every: Duration) -> Result<(), String> {
    let socket: UdpSocket =
        UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| format!("blast bind: {e}"))?;
    socket
        .connect((Ipv4Addr::LOCALHOST, port))
        .map_err(|e| e.to_string())?;
    let payload: Vec<u8> = vec![0; bytes];
    let start: Instant = Instant::now();
    for index in 0..count {
        let due: Duration = every * u32::try_from(index).map_err(|e| e.to_string())?;
        thread::sleep(due.saturating_sub(start.elapsed()));
        socket
            .send(&payload)
            .map_err(|e| format!("blast send: {e}"))?;
    }
    Ok(())
}
