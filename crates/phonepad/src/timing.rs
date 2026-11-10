use std::collections::VecDeque;

use phonepad_core::stream::{self, UnitKind};
use serde::Serialize;

pub const KEPT: usize = stream::MAX_UNITS;
const NANOS_PER_MILLI: f64 = 1_000_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sent {
    pub id: u32,
    pub kind: UnitKind,
    pub bytes: usize,
    pub captured_ns: u64,
    pub sent_ns: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drawn {
    pub id: u32,
    pub received_ms: f64,
    pub decoded_ms: f64,
    pub drawn_ms: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum Record {
    Unit {
        id: u32,
        key: bool,
        bytes: usize,
        capture_send_ms: f64,
        send_ack_ms: f64,
    },
    Drawn {
        id: u32,
        decode_ms: f64,
        draw_ms: f64,
        gap_ms: Option<f64>,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ledger {
    sent: VecDeque<Sent>,
    last_drawn_ms: Option<f64>,
}

fn millis(from_ns: u64, to_ns: u64) -> f64 {
    to_ns.saturating_sub(from_ns) as f64 / NANOS_PER_MILLI
}

impl Ledger {
    pub fn sent(&mut self, sent: Sent) {
        if self.sent.len() == KEPT {
            self.sent.pop_front();
        }
        self.sent.push_back(sent);
    }

    pub fn acked(&mut self, id: u32, at_ns: u64) -> Option<Record> {
        let index: usize = self.sent.iter().position(|sent: &Sent| sent.id == id)?;
        let sent: Sent = self.sent.remove(index)?;
        Some(Record::Unit {
            id,
            key: sent.kind == UnitKind::Keyframe,
            bytes: sent.bytes,
            capture_send_ms: millis(sent.captured_ns, sent.sent_ns),
            send_ack_ms: millis(sent.sent_ns, at_ns),
        })
    }

    pub fn drawn(&mut self, drawn: Drawn) -> Record {
        let gap_ms: Option<f64> = self.last_drawn_ms.map(|last: f64| drawn.drawn_ms - last);
        self.last_drawn_ms = Some(drawn.drawn_ms);
        Record::Drawn {
            id: drawn.id,
            decode_ms: drawn.decoded_ms - drawn.received_ms,
            draw_ms: drawn.drawn_ms - drawn.decoded_ms,
            gap_ms,
        }
    }
}

#[cfg(test)]
mod tests;
