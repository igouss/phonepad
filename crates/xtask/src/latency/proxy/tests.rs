use super::*;
use std::num::NonZeroU64;

fn profile(rtt_ms: u64, rate: Option<u64>) -> Profile {
    Profile {
        name: String::from("t"),
        rtt: Duration::from_millis(rtt_ms),
        jitter: Duration::ZERO,
        rate: rate.and_then(NonZeroU64::new),
        loss_ppm: 0,
    }
}

fn echo() -> u16 {
    let listener: TcpListener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port: u16 = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let (mut stream, _): (TcpStream, _) = listener.accept().unwrap();
        let mut back: TcpStream = stream.try_clone().unwrap();
        std::io::copy(&mut stream, &mut back).unwrap();
    });
    port
}

fn round_trip(emulator: &Emulator, message: &[u8]) -> (Vec<u8>, Duration) {
    let mut client: TcpStream = TcpStream::connect((Ipv4Addr::LOCALHOST, emulator.port)).unwrap();
    let start: Instant = Instant::now();
    client.write_all(message).unwrap();
    let mut back: Vec<u8> = vec![0; message.len()];
    client.read_exact(&mut back).unwrap();
    (back, start.elapsed())
}

#[test]
fn bytes_cross_the_emulator_unchanged_both_ways() {
    let emulator: Emulator = Emulator::start(echo(), profile(0, None)).unwrap();
    let message: Vec<u8> = (0..=255).cycle().take(SEGMENT * 3 + 7).collect();
    assert_eq!(round_trip(&emulator, &message).0, message);
}

#[test]
fn a_round_trip_takes_at_least_the_emulated_rtt() {
    let emulator: Emulator = Emulator::start(echo(), profile(80, None)).unwrap();
    let elapsed: Duration = round_trip(&emulator, b"ping").1;
    assert!(
        (Duration::from_millis(80)..Duration::from_millis(160)).contains(&elapsed),
        "{elapsed:?}"
    );
}

#[test]
fn a_new_profile_applies_to_the_next_segment() {
    let emulator: Emulator = Emulator::start(echo(), profile(400, None)).unwrap();
    emulator.set(profile(0, None));
    let elapsed: Duration = round_trip(&emulator, b"ping").1;
    assert!(elapsed < Duration::from_millis(100), "{elapsed:?}");
}

#[test]
fn a_capped_link_delivers_no_faster_than_its_rate() {
    let rate: f64 = throughput(profile(0, Some(8_000_000)), 200_000).unwrap();
    assert!((4e6..8.4e6).contains(&rate), "{rate} bit/s");
}
