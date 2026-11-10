use super::*;

pub struct Viewer {
    pub reorder: Reorder,
    pub released: Vec<Released>,
    pub pings: BTreeMap<u32, (u64, Option<u64>)>,
    pub epoch: Instant,
}

impl Viewer {
    pub fn new(epoch: Instant) -> Viewer {
        Viewer {
            reorder: Reorder::default(),
            released: Vec::new(),
            pings: BTreeMap::new(),
            epoch,
        }
    }

    pub fn arrive(&mut self, arrival: Arrival) -> Vec<u32> {
        let at_us: u64 = us(self.epoch);
        self.reorder
            .arrive(arrival)
            .into_iter()
            .map(|ready: Arrival| {
                self.released.push(Released {
                    at_us,
                    sent_us: ready.sent_us,
                    bytes: ready.bytes,
                });
                ready.id
            })
            .collect()
    }

    pub fn echoed(&mut self, record: &[u8]) {
        let at_us: u64 = us(self.epoch);
        if let Some(seq) = be32(record, 0)
            && let Some(entry) = self.pings.get_mut(&seq)
        {
            entry.1 = Some(at_us);
        }
    }

    pub fn log(self, server: Log) -> Log {
        Log {
            released: self.released,
            pings: self.pings.into_values().collect(),
            ..server
        }
    }
}

pub async fn pinger(events: mpsc::UnboundedSender<ViewerEvent>, epoch: Instant, until: Instant) {
    let mut ticks: tokio::time::Interval = tokio::time::interval(TICK);
    let mut seq: u32 = 0;
    while Instant::now() < until {
        ticks.tick().await;
        if events.send(ViewerEvent::Ping(seq, us(epoch))).is_err() {
            return;
        }
        seq += 1;
    }
}
