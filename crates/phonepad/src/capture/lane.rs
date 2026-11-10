use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};

use anyhow::{Context, anyhow};
use axum::body::Bytes;
use phonepad_core::stream::UnitKind;
use phonepad_h264::{AccessUnit, Cropped, H264};
use rustix::event::{EventfdFlags, eventfd};
use rustix::io::Errno;
use tokio::sync::broadcast;

use crate::wire;

use super::Unit;
use super::plan::Shot;

pub(super) const DEPTH: usize = 6;

pub(super) struct Job {
    pub(super) cropped: Cropped,
    pub(super) keyframe: bool,
    pub(super) shot: Shot,
}

pub(super) struct Lane {
    jobs: SyncSender<Job>,
    outstanding: Arc<AtomicUsize>,
    done: Arc<OwnedFd>,
    thread: JoinHandle<anyhow::Result<()>>,
}

impl Lane {
    pub(super) fn start(h264: H264, units: broadcast::Sender<Unit>) -> anyhow::Result<Lane> {
        let (jobs, queue): (SyncSender<Job>, Receiver<Job>) = mpsc::sync_channel(DEPTH);
        let outstanding: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
        let done: Arc<OwnedFd> = Arc::new(
            eventfd(0, EventfdFlags::CLOEXEC | EventfdFlags::NONBLOCK).context("eventfd")?,
        );
        let counted: Arc<AtomicUsize> = Arc::clone(&outstanding);
        let signal: Arc<OwnedFd> = Arc::clone(&done);
        let thread: JoinHandle<anyhow::Result<()>> = thread::Builder::new()
            .name("encode".to_owned())
            .spawn(move || encode_all(h264, &queue, &units, &counted, &signal))
            .context("spawn the encode thread")?;
        Ok(Lane {
            jobs,
            outstanding,
            done,
            thread,
        })
    }

    pub(super) fn busy(&self) -> bool {
        self.outstanding.load(Ordering::Acquire) >= DEPTH
    }

    pub(super) fn stopped(&self) -> bool {
        self.thread.is_finished()
    }

    pub(super) fn submit(&self, job: Job) -> anyhow::Result<()> {
        self.outstanding.fetch_add(1, Ordering::AcqRel);
        match self.jobs.try_send(job) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                self.outstanding.fetch_sub(1, Ordering::AcqRel);
                Err(anyhow!("encode queue full although it reported credit"))
            }
            Err(TrySendError::Disconnected(_)) => Err(anyhow!("the encode thread stopped")),
        }
    }

    pub(super) fn done_fd(&self) -> BorrowedFd<'_> {
        self.done.as_fd()
    }

    pub(super) fn clear_done(&self) {
        let mut count: [u8; 8] = [0; 8];
        match rustix::io::read(self.done.as_fd(), &mut count) {
            Ok(_) | Err(Errno::AGAIN) => {}
            Err(err) => eprintln!("capture: clear encode wakeup: {err}"),
        }
    }

    pub(super) fn finish(self) -> anyhow::Result<()> {
        drop(self.jobs);
        self.thread
            .join()
            .map_err(|_| anyhow!("the encode thread panicked"))?
    }
}

fn encode_all(
    mut h264: H264,
    queue: &Receiver<Job>,
    units: &broadcast::Sender<Unit>,
    outstanding: &AtomicUsize,
    done: &OwnedFd,
) -> anyhow::Result<()> {
    while let Ok(job) = queue.recv() {
        let unit: AccessUnit = h264.encode(job.cropped, job.keyframe).context("encode")?;
        outstanding.fetch_sub(1, Ordering::AcqRel);
        rustix::io::write(done, &1u64.to_ne_bytes()).context("signal encode done")?;
        let kind: UnitKind = if unit.key {
            UnitKind::Keyframe
        } else {
            UnitKind::Delta
        };
        let message: Bytes = Bytes::from(wire::frame_message(
            job.shot.region,
            job.shot.cursor,
            &unit.data,
        ));
        let _no_viewer_subscribed: Result<usize, broadcast::error::SendError<Unit>> =
            units.send(Unit { kind, message });
    }
    Ok(())
}
