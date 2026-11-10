use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhoneId(pub u64);

#[derive(Clone, Debug, Default)]
pub struct PhoneIds(Arc<AtomicU64>);

impl PhoneIds {
    pub fn next(&self) -> PhoneId {
        PhoneId(self.0.fetch_add(1, Ordering::Relaxed))
    }
}

pub type Applied = BTreeMap<PhoneId, u32>;

pub fn applied_seq(applied: &Applied, phone: PhoneId) -> u32 {
    applied.get(&phone).copied().unwrap_or(0)
}

#[derive(Debug, Default)]
pub struct Marks {
    generation: AtomicU64,
    applied: Mutex<Applied>,
}

impl Marks {
    fn applied(&self) -> MutexGuard<'_, Applied> {
        self.applied.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn moved<T>(&self, phone: PhoneId, seq: u32, emit: impl FnOnce() -> T) -> T {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let emitted: T = emit();
        self.applied().insert(phone, seq);
        self.generation.fetch_add(1, Ordering::SeqCst);
        emitted
    }

    pub fn forget(&self, phone: PhoneId) {
        self.applied().remove(&phone);
    }

    pub fn read<T>(&self, query: impl FnOnce() -> T) -> Option<(Applied, T)> {
        let before: u64 = self.generation.load(Ordering::SeqCst);
        if before % 2 == 1 {
            return None;
        }
        let applied: Applied = self.applied().clone();
        let answer: T = query();
        (self.generation.load(Ordering::SeqCst) == before).then_some((applied, answer))
    }
}

#[cfg(test)]
mod tests;
