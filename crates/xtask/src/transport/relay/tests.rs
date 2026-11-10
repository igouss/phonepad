use super::*;
use std::num::NonZeroU64;

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

fn profile(rtt: u64, jitter: u64, rate: Option<u64>) -> Profile {
    Profile {
        name: String::from("t"),
        rtt: ms(rtt),
        jitter: ms(jitter),
        rate: rate.and_then(NonZeroU64::new),
        loss_ppm: 0,
    }
}

const STEADY: link::Draw = link::Draw {
    jitter: 0.0,
    lost: false,
};

const LOST: link::Draw = link::Draw {
    jitter: 0.0,
    lost: true,
};

#[test]
fn a_datagram_on_an_idle_link_arrives_half_a_round_trip_later() {
    let (_, fate): (Bottleneck, Fate) = admit(
        Bottleneck::default(),
        &profile(100, 0, None),
        ms(10),
        1200,
        STEADY,
    );
    assert_eq!(fate, Fate::Deliver(ms(60)));
}

#[test]
fn a_datagram_waits_for_the_link_and_takes_its_bits_at_the_cap() {
    let capped: Profile = profile(0, 0, Some(1_000_000));
    let (first, _): (Bottleneck, Fate) = admit(Bottleneck::default(), &capped, ms(0), 1000, STEADY);
    let (_, fate): (Bottleneck, Fate) = admit(first, &capped, ms(0), 1000, STEADY);
    assert_eq!(fate, Fate::Deliver(ms(16)));
}

#[test]
fn a_lost_datagram_is_not_delivered() {
    let (_, fate): (Bottleneck, Fate) = admit(
        Bottleneck::default(),
        &profile(100, 0, None),
        ms(0),
        1200,
        LOST,
    );
    assert_eq!(fate, Fate::Lost);
}

#[test]
fn a_lost_datagram_still_took_its_time_on_the_link() {
    let capped: Profile = profile(0, 0, Some(1_000_000));
    let (lost, _): (Bottleneck, Fate) = admit(Bottleneck::default(), &capped, ms(0), 1000, LOST);
    let (_, fate): (Bottleneck, Fate) = admit(lost, &capped, ms(0), 1000, STEADY);
    assert_eq!(fate, Fate::Deliver(ms(16)));
}

#[test]
fn a_datagram_never_arrives_before_the_one_ahead_of_it() {
    let jittery: Profile = profile(100, 100, None);
    let late: link::Draw = link::Draw {
        jitter: 1.0,
        lost: false,
    };
    let early: link::Draw = link::Draw {
        jitter: -1.0,
        lost: false,
    };
    let (first, _): (Bottleneck, Fate) = admit(Bottleneck::default(), &jittery, ms(0), 100, late);
    let (_, fate): (Bottleneck, Fate) = admit(first, &jittery, ms(1), 100, early);
    assert_eq!(fate, Fate::Deliver(ms(100)));
}

#[test]
fn a_full_queue_drops_the_arriving_datagram() {
    let capped: Profile = profile(0, 0, Some(1_000_000));
    let full: Bottleneck = (0..QUEUE).fold(Bottleneck::default(), |bottleneck, _| {
        admit(bottleneck, &capped, ms(0), 1000, STEADY).0
    });
    assert_eq!(admit(full, &capped, ms(0), 1000, STEADY).1, Fate::Overflow);
}

#[test]
fn a_queue_drains_as_its_datagrams_leave_the_link() {
    let capped: Profile = profile(0, 0, Some(1_000_000));
    let full: Bottleneck = (0..QUEUE).fold(Bottleneck::default(), |bottleneck, _| {
        admit(bottleneck, &capped, ms(0), 1000, STEADY).0
    });
    assert_eq!(
        admit(full, &capped, ms(8), 1000, STEADY).1,
        Fate::Deliver(ms(8 * (QUEUE as u64 + 1)))
    );
}
