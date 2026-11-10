use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use phonepad_core::geometry::{Monitor, Point, Size};
use phonepad_h264::{CaptureSurface, Cropped, Cropper, Dmabuf, H264};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::io::Errno;
use tokio::sync::{broadcast, watch};
use wayland_client::backend::WaylandError;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::{Connection, EventQueue, QueueHandle};
use wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::ExtImageCaptureSourceV1;
use wayland_protocols::ext::image_capture_source::v1::client::ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1;
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_frame_v1::{
    self, ExtImageCopyCaptureFrameV1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_manager_v1::{
    self, ExtImageCopyCaptureManagerV1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_session_v1::{
    self, ExtImageCopyCaptureSessionV1,
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1::{
    self, ZwpLinuxBufferParamsV1,
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1;

use crate::hypr;

mod dispatch;

use super::cursor::Sampler;
use super::lane::{Job, Lane};
use super::plan::{
    self, Buffer, Constraints, Pending, Pixels, Schedule, Shape, Shot, Slot, Target, schedule,
};
use super::{Feed, KeyframeRequest, Unit};

const POOL: usize = 4;
const IN_FLIGHT: usize = 2;
const QP: u8 = 24;
const LINEAR: u64 = 0;
const RETRY: Duration = Duration::from_millis(100);
const MONITORS_MAX_AGE: Duration = Duration::from_secs(1);
const WAKE: Duration = Duration::from_millis(16);

struct Surface {
    surface: Arc<CaptureSurface>,
    buffer: WlBuffer,
}

struct Session {
    proxy: ExtImageCopyCaptureSessionV1,
    constraints: Constraints,
    ready: bool,
}

struct Pool {
    shape: Shape,
    cropper: Cropper,
    lane: Lane,
    source: ExtImageCaptureSourceV1,
    sessions: Vec<Session>,
    surfaces: Vec<Surface>,
    slots: Vec<Slot<ExtImageCopyCaptureFrameV1>>,
    presented: Vec<u64>,
    new_frame: bool,
    shown: Option<Shot>,
    encoded_at: Option<Instant>,
    keyframe_at: Option<Instant>,
}

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
    cursor: watch::Sender<Option<Point>>,
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
        if let Slot::Capturing { frame, .. } = slot {
            frame.destroy();
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

fn source_size(pixels: Pixels) -> phonepad_h264::Size {
    phonepad_h264::Size {
        width: pixels.width,
        height: pixels.height,
    }
}

impl Pool {
    fn open(
        render_node: &Path,
        shape: &Shape,
        state: &State,
        qh: &QueueHandle<State>,
    ) -> anyhow::Result<Pool> {
        let dmabuf: &ZwpLinuxDmabufV1 = state.dmabuf.as_ref().context("linux-dmabuf not bound")?;
        let output: &WlOutput = &state
            .outputs
            .iter()
            .find(|o: &&NamedOutput| o.name.as_deref() == Some(shape.output.as_str()))
            .with_context(|| format!("no wayland output named {}", shape.output))?
            .proxy;
        let source: ExtImageCaptureSourceV1 = state
            .sources
            .as_ref()
            .context("capture source manager not bound")?
            .create_source(output, qh, ());
        let manager: &ExtImageCopyCaptureManagerV1 = state
            .manager
            .as_ref()
            .context("copy capture manager not bound")?;
        let sessions: Vec<Session> = (0..IN_FLIGHT)
            .map(|index: usize| Session {
                proxy: manager.create_session(
                    &source,
                    ext_image_copy_capture_manager_v1::Options::empty(),
                    qh,
                    index,
                ),
                constraints: Constraints::default(),
                ready: false,
            })
            .collect();
        let (mut cropper, h264): (Cropper, H264) = phonepad_h264::open(
            render_node,
            source_size(shape.source),
            source_size(shape.window),
            QP,
        )
        .with_context(|| format!("open the H.264 encoder for {shape:?}"))?;
        let surfaces: Vec<Surface> = (0..POOL)
            .map(|_| -> anyhow::Result<Surface> {
                let surface: CaptureSurface = cropper.surface().context("encoder surface")?;
                let buffer: WlBuffer = wl_buffer(dmabuf, surface.dmabuf(), qh)?;
                Ok(Surface {
                    surface: Arc::new(surface),
                    buffer,
                })
            })
            .collect::<anyhow::Result<Vec<Surface>>>()?;
        Ok(Pool {
            shape: shape.clone(),
            cropper,
            lane: Lane::start(h264, state.units.clone())?,
            source,
            sessions,
            surfaces,
            slots: (0..POOL).map(|_| Slot::Free).collect(),
            presented: vec![0; POOL],
            new_frame: false,
            shown: None,
            encoded_at: None,
            keyframe_at: None,
        })
    }
}

fn wl_buffer(
    dmabuf: &ZwpLinuxDmabufV1,
    plane: &Dmabuf,
    qh: &QueueHandle<State>,
) -> anyhow::Result<WlBuffer> {
    if plane.modifier != LINEAR {
        bail!(
            "encoder surface has DRM modifier {:#x}; Hyprland screencopy takes only linear dmabufs",
            plane.modifier
        );
    }
    let params: ZwpLinuxBufferParamsV1 = dmabuf.create_params(qh, ());
    params.add(plane.fd.as_fd(), 0, plane.offset, plane.stride, 0, 0);
    let buffer: WlBuffer = params.create_immed(
        i32::try_from(plane.width)?,
        i32::try_from(plane.height)?,
        plane.fourcc,
        zwp_linux_buffer_params_v1::Flags::empty(),
        qh,
        (),
    );
    params.destroy();
    Ok(buffer)
}

impl State {
    fn refresh_target(&mut self) -> anyhow::Result<()> {
        let moved: bool = self.sampler.sample_if_due(Instant::now());
        if moved {
            self.cursor.send_replace(self.sampler.at());
        }
        let changed: bool = self.view.has_changed().unwrap_or(false) || moved;
        if !changed && self.target.is_some() {
            return Ok(());
        }
        let view: Option<Size> = *self.view.borrow_and_update();
        let cursor: Option<Point> = self.sampler.at();
        let (Some(view), Some(cursor)) = (view, cursor) else {
            self.target = None;
            return Ok(());
        };
        if self
            .monitors_at
            .is_none_or(|at: Instant| at.elapsed() > MONITORS_MAX_AGE)
        {
            self.monitors = hypr::monitors()?;
            self.monitors_at = Some(Instant::now());
        }
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
        let busy: Vec<usize> = pool
            .slots
            .iter()
            .filter_map(|slot: &Slot<ExtImageCopyCaptureFrameV1>| match slot {
                Slot::Capturing { session, .. } => Some(*session),
                Slot::Free | Slot::Held { .. } => None,
            })
            .collect();
        let idle: Vec<usize> = (0..pool.sessions.len())
            .filter(|index: &usize| pool.sessions[*index].ready && !busy.contains(index))
            .collect();
        let pinned: Vec<bool> = pool
            .surfaces
            .iter()
            .map(|surface: &Surface| Arc::strong_count(&surface.surface) > 1)
            .collect();
        for (index, session) in plan::to_issue(&pool.slots, &pinned, idle.len())
            .into_iter()
            .zip(idle)
        {
            let plane: &Dmabuf = pool.surfaces[index].surface.dmabuf();
            let (width, height): (i32, i32) =
                (i32::try_from(plane.width)?, i32::try_from(plane.height)?);
            let frame: ExtImageCopyCaptureFrameV1 =
                pool.sessions[session].proxy.create_frame(qh, index);
            frame.attach_buffer(&pool.surfaces[index].buffer);
            frame.damage_buffer(0, 0, width, height);
            frame.capture();
            pool.slots[index] = Slot::Capturing { frame, session };
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
            new_frame: pool.new_frame,
            keyframe: plan::keyframe_due(
                self.keyframe.pending(),
                pool.keyframe_at.map(|at: Instant| at.elapsed()),
            ),
            encoder_busy: pool.lane.busy(),
        };
        let since_last: Option<Duration> = pool.encoded_at.map(|at: Instant| at.elapsed());
        match schedule(pending, pool.shown, wanted, since_last) {
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
                pool.new_frame = false;
                pool.shown = Some(wanted);
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
                if let Some(presented) = pool.presented.get_mut(index) {
                    *presented = plan::stamp(tv_sec_hi, tv_sec_lo, tv_nsec);
                }
            }
            ext_image_copy_capture_frame_v1::Event::Ready => {
                frame.destroy();
                let stamp: u64 = pool.presented.get(index).copied().unwrap_or(0);
                if plan::arrive(&mut pool.slots, index, stamp) {
                    pool.new_frame = true;
                }
            }
            ext_image_copy_capture_frame_v1::Event::Failed { .. } => {
                frame.destroy();
                if let Some(slot) = pool.slots.get_mut(index) {
                    *slot = Slot::Free;
                }
                self.failed = true;
            }
            ext_image_copy_capture_frame_v1::Event::Transform { .. }
            | ext_image_copy_capture_frame_v1::Event::Damage { .. }
            | _ => {}
        }
    }
}
