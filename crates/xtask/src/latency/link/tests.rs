use super::*;

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

const STEADY: Draw = Draw {
    jitter: 0.0,
    lost: false,
};

fn custom(text: &str) -> Result<(Duration, Duration, Option<u64>, u64), String> {
    parse(text).map(|p: Profile| (p.rtt, p.jitter, p.rate.map(NonZeroU64::get), p.loss_ppm))
}

#[test]
fn lan_adds_nothing() {
    assert_eq!(
        named("lan"),
        Some(Profile {
            name: String::from("lan"),
            ..profile(0, 0, None)
        })
    );
}

#[test]
fn wifi_is_10_ms_at_50_mbit() {
    assert_eq!(
        named("wifi"),
        Some(Profile {
            name: String::from("wifi"),
            ..profile(10, 0, Some(50_000_000))
        })
    );
}

#[test]
fn lte_is_60_ms_with_20_ms_jitter_at_10_mbit() {
    assert_eq!(
        named("lte"),
        Some(Profile {
            name: String::from("lte"),
            ..profile(60, 20, Some(10_000_000))
        })
    );
}

#[test]
fn bad_lte_is_150_ms_with_50_ms_jitter_at_3_mbit_and_1_percent_loss() {
    assert_eq!(
        named("bad-lte"),
        Some(Profile {
            name: String::from("bad-lte"),
            loss_ppm: 10_000,
            ..profile(150, 50, Some(3_000_000))
        })
    );
}

#[test]
fn an_unknown_name_is_no_profile() {
    assert_eq!(named("5g"), None);
}

#[test]
fn a_named_profile_parses_by_name() {
    assert_eq!(parse("lte"), named("lte").ok_or_else(String::new));
}

#[test]
fn one_setting_leaves_the_rest_unemulated() {
    assert_eq!(custom("rtt=100"), Ok((ms(100), ms(0), None, 0)));
}

#[test]
fn many_settings_combine() {
    assert_eq!(
        custom("rtt=150,jitter=12.5,rate=2.5,loss=0.5"),
        Ok((
            ms(150),
            Duration::from_micros(12_500),
            Some(2_500_000),
            5_000
        ))
    );
}

#[test]
fn a_custom_profile_is_named_by_its_text() {
    assert_eq!(
        parse("rtt=1").map(|p: Profile| p.name),
        Ok(String::from("rtt=1"))
    );
}

#[test]
fn full_loss_is_allowed() {
    assert_eq!(custom("loss=100"), Ok((ms(0), ms(0), None, 1_000_000)));
}

#[test]
fn loss_above_100_percent_is_rejected() {
    assert!(parse("loss=100.5").is_err());
}

#[test]
fn a_zero_rate_is_rejected() {
    assert!(parse("rate=0").is_err());
}

#[test]
fn a_negative_delay_is_rejected() {
    assert!(parse("rtt=-1").is_err());
}

#[test]
fn a_non_numeric_value_is_rejected() {
    assert!(parse("rtt=abc").is_err());
}

#[test]
fn an_unknown_setting_is_rejected() {
    assert!(parse("speed=1").is_err());
}

#[test]
fn a_setting_without_a_value_is_rejected() {
    assert!(parse("rtt=1,jitter").is_err());
}

#[test]
fn an_unknown_word_is_named_as_an_unknown_profile() {
    assert!(
        parse("fast")
            .err()
            .is_some_and(|error: String| error.starts_with("unknown profile `fast`"))
    );
}

#[test]
fn a_sample_below_the_loss_rate_is_lost() {
    assert!(lost(1_009_999, 10_000));
}

#[test]
fn a_sample_at_the_loss_rate_is_kept() {
    assert!(!lost(10_000, 10_000));
}

#[test]
fn no_loss_keeps_a_zero_sample() {
    assert!(!lost(0, 0));
}

#[test]
fn an_uncapped_link_serializes_instantly() {
    assert_eq!(serialization(1_000_000, None), Duration::ZERO);
}

#[test]
fn a_segment_at_one_megabit_takes_its_bits_in_microseconds() {
    assert_eq!(
        serialization(SEGMENT, NonZeroU64::new(1_000_000)),
        Duration::from_micros(11_584)
    );
}

#[test]
fn no_bytes_take_no_time() {
    assert_eq!(serialization(0, NonZeroU64::new(1)), Duration::ZERO);
}

#[test]
fn one_way_without_jitter_is_half_the_round_trip() {
    assert_eq!(one_way(&profile(100, 20, None), 0.0), ms(50));
}

#[test]
fn full_positive_jitter_adds_half_its_amplitude() {
    assert_eq!(one_way(&profile(100, 20, None), 1.0), ms(60));
}

#[test]
fn full_negative_jitter_removes_half_its_amplitude() {
    assert_eq!(one_way(&profile(100, 20, None), -1.0), ms(40));
}

#[test]
fn jitter_beyond_its_amplitude_is_clamped() {
    assert_eq!(
        [
            one_way(&profile(100, 20, None), 3.0),
            one_way(&profile(100, 20, None), -3.0)
        ],
        [ms(60), ms(40)]
    );
}

#[test]
fn jitter_larger_than_the_delay_never_goes_negative() {
    assert_eq!(one_way(&profile(10, 50, None), -1.0), Duration::ZERO);
}

#[test]
fn a_short_round_trip_retransmits_after_the_minimum_timeout() {
    assert_eq!(retransmission(&profile(0, 0, None)), MIN_RTO);
}

#[test]
fn a_long_round_trip_retransmits_after_two_round_trips() {
    assert_eq!(retransmission(&profile(150, 0, None)), ms(300));
}

#[test]
fn a_segment_on_an_idle_lan_leaves_when_it_arrives() {
    assert_eq!(
        schedule(Lane::default(), &profile(0, 0, None), ms(5), 1000, STEADY),
        Lane {
            free_at: ms(5),
            last_out: ms(5)
        }
    );
}

#[test]
fn a_segment_is_delayed_by_half_the_round_trip() {
    assert_eq!(
        schedule(Lane::default(), &profile(100, 0, None), ms(5), 1000, STEADY).last_out,
        ms(55)
    );
}

#[test]
fn a_capped_segment_waits_for_the_link_to_free() {
    let busy: Lane = Lane {
        free_at: ms(20),
        last_out: ms(20),
    };
    assert_eq!(
        schedule(busy, &profile(0, 0, Some(1_000_000)), ms(5), 1000, STEADY),
        Lane {
            free_at: ms(28),
            last_out: ms(28)
        }
    );
}

#[test]
fn a_capped_segment_after_the_link_freed_starts_on_arrival() {
    let idle: Lane = Lane {
        free_at: ms(2),
        last_out: ms(2),
    };
    assert_eq!(
        schedule(idle, &profile(0, 0, Some(1_000_000)), ms(5), 1000, STEADY).free_at,
        ms(13)
    );
}

#[test]
fn jitter_never_lets_a_segment_overtake_the_one_before() {
    let ahead: Lane = Lane {
        free_at: ms(0),
        last_out: ms(100),
    };
    assert_eq!(
        schedule(
            ahead,
            &profile(100, 40, None),
            ms(10),
            1,
            Draw {
                jitter: -1.0,
                lost: false
            }
        )
        .last_out,
        ms(100)
    );
}

#[test]
fn a_lost_segment_arrives_one_retransmission_later() {
    assert_eq!(
        schedule(
            Lane::default(),
            &profile(100, 0, None),
            ms(0),
            1,
            Draw {
                jitter: 0.0,
                lost: true
            }
        )
        .last_out,
        ms(250)
    );
}

#[test]
fn the_same_seed_draws_the_same_sequence() {
    let mut rng: Rng = Rng::seeded(1);
    assert_eq!(
        [rng.draw(500_000), rng.draw(500_000)],
        [
            Draw {
                jitter: -0.43832989989928106,
                lost: false
            },
            Draw {
                jitter: 0.45169229056673355,
                lost: true
            }
        ]
    );
}

#[test]
fn a_zero_seed_still_draws() {
    assert_ne!(Rng::seeded(0).draw(0).jitter, -1.0);
}

#[test]
fn jitter_draws_stay_within_one() {
    let mut rng: Rng = Rng::seeded(7);
    assert!(
        (0..10_000)
            .map(|_| rng.draw(0).jitter)
            .all(|j: f64| (-1.0..1.0).contains(&j))
    );
}

#[test]
fn jitter_draws_centre_on_zero() {
    let mut rng: Rng = Rng::seeded(7);
    let mean: f64 = (0..10_000).map(|_| rng.draw(0).jitter).sum::<f64>() / 10_000.0;
    assert!(mean.abs() < 0.03, "mean {mean}");
}

#[test]
fn no_loss_loses_nothing() {
    let mut rng: Rng = Rng::seeded(7);
    assert_eq!((0..10_000).filter(|_| rng.draw(0).lost).count(), 0);
}

#[test]
fn full_loss_loses_everything() {
    let mut rng: Rng = Rng::seeded(7);
    assert_eq!(
        (0..10_000).filter(|_| rng.draw(1_000_000).lost).count(),
        10_000
    );
}

#[test]
fn one_percent_loss_loses_about_one_in_a_hundred() {
    let mut rng: Rng = Rng::seeded(7);
    let lost: usize = (0..100_000).filter(|_| rng.draw(10_000).lost).count();
    assert!((900..1100).contains(&lost), "lost {lost}");
}
