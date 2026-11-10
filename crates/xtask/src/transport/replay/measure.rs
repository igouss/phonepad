use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arrival {
    pub seq: u32,
    pub id: u32,
    pub sent_us: u64,
    pub bytes: usize,
}

#[derive(Debug, Default)]
pub struct Reorder {
    pub next: u32,
    pub held: BTreeMap<u32, Arrival>,
}

impl Reorder {
    pub fn arrive(&mut self, arrival: Arrival) -> Vec<Arrival> {
        self.held.insert(arrival.seq, arrival);
        let mut released: Vec<Arrival> = Vec::new();
        while let Some(ready) = self.held.remove(&self.next) {
            released.push(ready);
            self.next += 1;
        }
        released
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Released {
    pub at_us: u64,
    pub sent_us: u64,
    pub bytes: usize,
}

#[derive(Debug, Default)]
pub struct Log {
    pub released: Vec<Released>,
    pub acked: Vec<(u64, u64)>,
    pub pings: Vec<(u64, Option<u64>)>,
    pub keyframes: usize,
    pub desyncs: usize,
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub units_per_s: f64,
    pub mbit_per_s: f64,
    pub stalls_250: usize,
    pub stalls_500: usize,
    pub longest_gap_ms: Option<f64>,
    pub send_draw: Option<Spread>,
    pub send_ack: Option<Spread>,
    pub ping: Option<Spread>,
    pub pings_unanswered: usize,
    pub keyframes: usize,
    pub desyncs: usize,
}

pub fn us(epoch: Instant) -> u64 {
    u64::try_from(epoch.elapsed().as_micros()).unwrap_or(u64::MAX)
}

pub fn ms(micros: u64) -> f64 {
    micros as f64 / 1e3
}

pub fn outcome(log: &Log, from_us: u64, to_us: u64) -> Outcome {
    let seconds: f64 = ms(to_us - from_us) / 1e3;
    let inside = |at: u64| at >= from_us && at < to_us;
    let released: Vec<&Released> = log
        .released
        .iter()
        .filter(|released: &&Released| inside(released.at_us))
        .collect();
    let gaps: Vec<f64> = released
        .windows(2)
        .map(|pair: &[&Released]| ms(pair[1].at_us - pair[0].at_us))
        .collect();
    let pings: Vec<&(u64, Option<u64>)> = log
        .pings
        .iter()
        .filter(|(sent, _): &&(u64, Option<u64>)| inside(*sent))
        .collect();
    Outcome {
        units_per_s: released.len() as f64 / seconds,
        mbit_per_s: released
            .iter()
            .map(|released: &&Released| released.bytes)
            .sum::<usize>() as f64
            * 8.0
            / seconds
            / 1e6,
        stalls_250: gaps.iter().filter(|gap: &&f64| **gap > 250.0).count(),
        stalls_500: gaps.iter().filter(|gap: &&f64| **gap > 500.0).count(),
        longest_gap_ms: gaps.iter().copied().reduce(f64::max),
        send_draw: stats::spread(
            released
                .iter()
                .map(|released: &&Released| ms(released.at_us.saturating_sub(released.sent_us)))
                .collect(),
        ),
        send_ack: stats::spread(
            log.acked
                .iter()
                .filter(|(sent, _): &&(u64, u64)| inside(*sent))
                .map(|(sent, acked): &(u64, u64)| ms(acked.saturating_sub(*sent)))
                .collect(),
        ),
        ping: stats::spread(
            pings
                .iter()
                .filter_map(|(sent, echoed): &&(u64, Option<u64>)| {
                    echoed.map(|echoed: u64| ms(echoed.saturating_sub(*sent)))
                })
                .collect(),
        ),
        pings_unanswered: pings
            .iter()
            .filter(|(_, echoed): &&&(u64, Option<u64>)| echoed.is_none())
            .count(),
        keyframes: log.keyframes,
        desyncs: log.desyncs,
    }
}
