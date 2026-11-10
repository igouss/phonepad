use std::io;
use std::os::fd::BorrowedFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use phonepad_core::geometry::{Monitor, Point, Size};
use phonepad_h264::{Cropped, Dmabuf};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::io::Errno;
use tokio::sync::{broadcast, watch};
use wayland_client::backend::WaylandError;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::{Connection, EventQueue, QueueHandle};
use wayland_protocols::ext::image_capture_source::v1::client::ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1;
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_frame_v1::{
    self, ExtImageCopyCaptureFrameV1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_manager_v1::ExtImageCopyCaptureManagerV1;
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_session_v1::{
    self,
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1;

use crate::hypr;
use crate::marks::Marks;

mod dispatch;
mod pool;

use super::cursor::{Sample, Sampler};
use super::lane::Job;
use super::plan::{
    self, Buffer, Constraints, Pending, Schedule, Shot, Shown, Slot, Target, schedule,
};
use super::{Feed, KeyframeRequest, Pointer, Unit};
use pool::{Pool, Session, Surface};

const RETRY: Duration = Duration::from_millis(100);
const MONITORS_MAX_AGE: Duration = Duration::from_secs(1);
const WAKE: Duration = Duration::from_millis(16);

struct NamedOutput {
    proxy: WlOutput,
    name: Option<String>,
}

struct State {
    dmabuf: Option<ZwpLinuxDmabufV1>,
    sources: Option<ExtOutputImageCaptureSourceManagerV1>,
    manager: Option<ExtImageCopyCaptureManagerV1>,
    outputs: Vec<NamedOutput>,
    view: watch::Receiver<Option<Size>>,
    sampler: Sampler,
    marks: Arc<Marks>,
    cursor: watch::Sender<Option<Pointer>>,
    units: broadcast::Sender<Unit>,
    keyframe: KeyframeRequest,
    target: Option<Target>,
    monitors: Vec<Monitor>,
    monitors_at: Option<Instant>,
    pool: Option<Pool>,
    failed: bool,
    fatal: Option<String>,
}

pub(super) fn run(stop: &AtomicBool, feed: Feed) -> anyhow::Result<()> {
    let conn: Connection = Connection::connect_to_env().context("wayland connect")?;
    let mut queue: EventQueue<State> = conn.new_event_queue();
    let qh: QueueHandle<State> = queue.handle();
    conn.display().get_registry(&qh, ());
    let mut state: State = State {
        dmabuf: None,
        sources: None,
        manager: None,
        outputs: Vec::new(),
        view: feed.view,
        sampler: Sampler::new(Instant::now()),
        marks: feed.marks,
        cursor: feed.cursor,
        units: feed.units,
        keyframe: feed.keyframe,
        target: None,
        monitors: Vec::new(),
        monitors_at: None,
        pool: None,
        failed: false,
        fatal: None,
    };
    queue.roundtrip(&mut state).context("wayland registry")?;
    queue
        .roundtrip(&mut state)
        .context("wayland output names")?;
    if state.dmabuf.is_none() || state.sources.is_none() || state.manager.is_none() {
        bail!(
            "compositor lacks zwp_linux_dmabuf_v1, ext_output_image_capture_source_manager_v1 or ext_image_copy_capture_manager_v1"
        );
    }
    let mut wait_for: Duration = WAKE;
    while !stop.load(Ordering::Relaxed) {
        if let Some(fatal) = state.fatal.take() {
            bail!(fatal);
        }
        if std::mem::take(&mut state.failed) {
            thread::sleep(RETRY);
        }
        if let Err(err) = state.refresh_target() {
            eprintln!("capture: {err:#}");
        }
        if state
            .pool
            .as_ref()
            .is_some_and(|pool: &Pool| pool.lane.stopped())
        {
            close(&mut queue, &mut state)?;
            bail!("the encode thread stopped");
        }
        if let Some(target) = state.target.clone() {
            if state.pool.as_ref().map(|pool: &Pool| &pool.shape) != Some(&target.shape) {
                close(&mut queue, &mut state)?;
                state.pool = Some(Pool::open(&feed.render_node, &target.shape, &state, &qh)?);
            }
            state.issue(&qh)?;
            wait_for = state.encode_if_due(target.shot)?;
        }
        wait_for = wait_for.min(state.sampler.until_due(Instant::now()));
        wait(&mut queue, &mut state, wait_for)?;
    }
    close(&mut queue, &mut state)
}

fn close(queue: &mut EventQueue<State>, state: &mut State) -> anyhow::Result<()> {
    let Some(pool) = state.pool.take() else {
        return Ok(());
    };
    for slot in &pool.slots {
        match slot {
            Slot::Requested { frame, .. } | Slot::Copying { frame, .. } => frame.destroy(),
            Slot::Free | Slot::Held { .. } => {}
        }
    }
    for session in &pool.sessions {
        session.proxy.destroy();
    }
    pool.source.destroy();
    for surface in &pool.surfaces {
        surface.buffer.destroy();
    }
    queue.roundtrip(state).context("release capture buffers")?;
    pool.lane.finish()
}

fn wait(queue: &mut EventQueue<State>, state: &mut State, timeout: Duration) -> anyhow::Result<()> {
    queue.flush().context("wayland flush")?;
    if let Some(guard) = queue.prepare_read() {
        let limit: Timespec = Timespec::try_from(timeout).context("poll timeout")?;
        let done: Option<BorrowedFd<'_>> =
            state.pool.as_ref().map(|pool: &Pool| pool.lane.done_fd());
        let wayland_ready: bool = {
            let mut fds: Vec<PollFd<'_>> = std::iter::once(guard.connection_fd())
                .chain(done)
                .map(|fd: BorrowedFd<'_>| PollFd::from_borrowed_fd(fd, PollFlags::IN))
                .collect();
            match poll(&mut fds, Some(&limit)) {
                Ok(_) => !fds[0].revents().is_empty(),
                Err(Errno::INTR) => false,
                Err(err) => return Err(err).context("poll wayland socket"),
            }
        };
        if let Some(pool) = state.pool.as_ref() {
            pool.lane.clear_done();
        }
        if wayland_ready {
            match guard.read() {
                Ok(_) => {}
                Err(WaylandError::Io(err)) if err.kind() == io::ErrorKind::WouldBlock => {}
                Err(err) => return Err(err).context("wayland read"),
            }
        }
    }
    queue.dispatch_pending(state).context("dispatch")?;
    Ok(())
}

impl State {
    fn refresh_monitors(&mut self) -> anyhow::Result<()> {
        if self
            .monitors_at
            .is_none_or(|at: Instant| at.elapsed() > MONITORS_MAX_AGE)
        {
            self.monitors = hypr::monitors()?;
            self.monitors_at = Some(Instant::now());
        }
        Ok(())
    }

    fn refresh_target(&mut self) -> anyhow::Result<()> {
        let moved: bool = self.sampler.sample_if_due(Instant::now(), &self.marks);
        let changed: bool = self.view.has_changed().unwrap_or(false) || moved;
        if !changed && self.target.is_some() {
            return Ok(());
        }
        let monitors: anyhow::Result<()> = self.refresh_monitors();
        if moved {
            self.cursor.send_replace(
                self.sampler
                    .at()
                    .and_then(|sample: &Sample| sample.pointer(&self.monitors)),
            );
        }
        monitors?;
        let view: Option<Size> = *self.view.borrow_and_update();
        let cursor: Option<Point> = self.sampler.at().map(|sample: &Sample| sample.at);
        let (Some(view), Some(cursor)) = (view, cursor) else {
            self.target = None;
            return Ok(());
        };
        self.target = plan::target(
            cursor,
            view,
            &self.monitors,
            self.target.as_ref().map(|kept: &Target| &kept.window),
        );
        Ok(())
    }

    fn issue(&mut self, qh: &QueueHandle<State>) -> anyhow::Result<()> {
        let Some(pool) = self.pool.as_mut() else {
            return Ok(());
        };
        let pinned: Vec<bool> = pool
            .surfaces
            .iter()
            .map(|surface: &Surface| Arc::strong_count(&surface.surface) > 1)
            .collect();
        let ready: Vec<bool> = pool
            .sessions
            .iter()
            .map(|session: &Session| session.ready)
            .collect();
        if let Some((index, session)) = plan::to_issue(&pool.slots, &pinned, &ready) {
            let plane: &Dmabuf = pool.surfaces[index].surface.dmabuf();
            let (width, height): (i32, i32) =
                (i32::try_from(plane.width)?, i32::try_from(plane.height)?);
            let frame: ExtImageCopyCaptureFrameV1 =
                pool.sessions[session].proxy.create_frame(qh, index);
            frame.attach_buffer(&pool.surfaces[index].buffer);
            frame.damage_buffer(0, 0, width, height);
            frame.capture();
            pool.slots[index] = Slot::Requested {
                frame,
                session,
                damage: Vec::new(),
            };
        }
        Ok(())
    }

    fn encode_if_due(&mut self, wanted: Shot) -> anyhow::Result<Duration> {
        let Some(pool) = self.pool.as_mut() else {
            return Ok(WAKE);
        };
        let Some((held, captured_ns)) = plan::held(&pool.slots) else {
            return Ok(WAKE);
        };
        let pending: Pending = Pending {
            changed: pool.changed,
            keyframe: plan::keyframe_due(
                self.keyframe.pending(),
                pool.keyframe_at.map(|at: Instant| at.elapsed()),
            ),
            encoder_busy: pool.lane.busy(),
        };
        let since_last: Option<Duration> = pool.encoded_at.map(|at: Instant| at.elapsed());
        match schedule(
            pending,
            pool.shown.map(|shown: Shown| shown.shot),
            wanted,
            since_last,
        ) {
            Schedule::Idle | Schedule::Busy => Ok(WAKE),
            Schedule::WaitFor(left) => Ok(left.min(WAKE)),
            Schedule::EncodeNow => {
                let cropped: Cropped = pool
                    .cropper
                    .crop(
                        &pool.surfaces[held].surface,
                        (wanted.crop.x, wanted.crop.y),
                        phonepad_h264::Size {
                            width: wanted.crop.w.get(),
                            height: wanted.crop.h.get(),
                        },
                    )
                    .context("crop")?;
                pool.lane.submit(Job {
                    cropped,
                    transform: pool.shape.transform,
                    keyframe: pending.keyframe && self.keyframe.take(),
                    shot: wanted,
                    captured_ns,
                })?;
                pool.changed = false;
                pool.shown = Some(Shown {
                    shot: wanted,
                    stamp: captured_ns,
                });
                pool.encoded_at = Some(Instant::now());
                if pending.keyframe || pool.keyframe_at.is_none() {
                    pool.keyframe_at = pool.encoded_at;
                }
                Ok(WAKE)
            }
        }
    }

    fn session_event(&mut self, index: usize, event: ext_image_copy_capture_session_v1::Event) {
        let Some(pool) = self.pool.as_mut() else {
            return;
        };
        let Some(session) = pool.sessions.get_mut(index) else {
            return;
        };
        match event {
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => {
                session.constraints.size = Some((width, height));
                session.ready = false;
            }
            ext_image_copy_capture_session_v1::Event::DmabufFormat { format, modifiers } => {
                session.ready = false;
                session.constraints.dmabuf.push((
                    format,
                    modifiers
                        .as_chunks::<8>()
                        .0
                        .iter()
                        .map(|bytes: &[u8; 8]| u64::from_ne_bytes(*bytes))
                        .collect(),
                ));
            }
            ext_image_copy_capture_session_v1::Event::Done => {
                let plane: &Dmabuf = pool.surfaces[0].surface.dmabuf();
                let buffer: Buffer = Buffer {
                    fourcc: plane.fourcc,
                    modifier: plane.modifier,
                    width: plane.width,
                    height: plane.height,
                };
                let constraints: Constraints = std::mem::take(&mut session.constraints);
                match plan::fits(&constraints, buffer) {
                    Ok(()) => session.ready = true,
                    Err(misfit) => self.fatal = Some(misfit.to_string()),
                }
            }
            ext_image_copy_capture_session_v1::Event::Stopped => {
                self.fatal = Some("the compositor stopped the capture session".to_owned());
            }
            ext_image_copy_capture_session_v1::Event::ShmFormat { .. }
            | ext_image_copy_capture_session_v1::Event::DmabufDevice { .. }
            | _ => {}
        }
    }

    fn frame_event(
        &mut self,
        index: usize,
        frame: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
    ) {
        let Some(pool) = self.pool.as_mut() else {
            return;
        };
        match event {
            ext_image_copy_capture_frame_v1::Event::PresentationTime {
                tv_sec_hi,
                tv_sec_lo,
                tv_nsec,
            } => {
                plan::presented(
                    &mut pool.slots,
                    index,
                    plan::stamp(tv_sec_hi, tv_sec_lo, tv_nsec),
                );
            }
            ext_image_copy_capture_frame_v1::Event::Ready => {
                frame.destroy();
                pool.changed |=
                    pool.slots
                        .get(index)
                        .is_some_and(|slot: &Slot<ExtImageCopyCaptureFrameV1>| {
                            plan::changes_shown(slot, pool.shown)
                        });
                plan::arrive(&mut pool.slots, index);
            }
            ext_image_copy_capture_frame_v1::Event::Damage {
                x,
                y,
                width,
                height,
            } => {
                if let Some(rect) = plan::damage(x, y, width, height) {
                    plan::damaged(&mut pool.slots, index, rect);
                }
            }
            ext_image_copy_capture_frame_v1::Event::Failed { .. } => {
                frame.destroy();
                if let Some(slot) = pool.slots.get_mut(index) {
                    *slot = Slot::Free;
                }
                self.failed = true;
            }
            ext_image_copy_capture_frame_v1::Event::Transform { .. } | _ => {}
        }
    }
}
