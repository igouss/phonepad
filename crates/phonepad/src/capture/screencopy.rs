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
use wayland_client::protocol::wl_buffer::{self, WlBuffer};
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1::{
    self, ZwpLinuxBufferParamsV1,
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::{
    self, ZwpLinuxDmabufV1,
};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::{
    self, ZwlrScreencopyFrameV1,
};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::{
    self, ZwlrScreencopyManagerV1,
};

use crate::hypr;

use super::cursor::Sampler;
use super::lane::{Job, Lane};
use super::plan::{
    self, CopyMode, Offer, Pending, Pixels, Schedule, Shape, Shot, Slot, Target, schedule,
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

struct Pool {
    shape: Shape,
    cropper: Cropper,
    lane: Lane,
    surfaces: Vec<Surface>,
    slots: Vec<Slot<ZwlrScreencopyFrameV1>>,
    new_frame: bool,
    shown: Option<Shot>,
    encoded_at: Option<Instant>,
}

struct NamedOutput {
    proxy: WlOutput,
    name: Option<String>,
}

struct State {
    dmabuf: Option<ZwpLinuxDmabufV1>,
    manager: Option<ZwlrScreencopyManagerV1>,
    outputs: Vec<NamedOutput>,
    view: watch::Receiver<Option<Size>>,
    sampler: Sampler,
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
        manager: None,
        outputs: Vec::new(),
        view: feed.view,
        sampler: Sampler::new(Instant::now()),
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
    if state.dmabuf.is_none() || state.manager.is_none() {
        bail!("compositor lacks zwp_linux_dmabuf_v1 or zwlr_screencopy_manager_v1");
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
        let (mut cropper, h264): (Cropper, H264) = phonepad_h264::open(
            render_node,
            source_size(shape.source),
            source_size(shape.view),
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
            surfaces,
            slots: (0..POOL).map(|_| Slot::Free).collect(),
            new_frame: false,
            shown: None,
            encoded_at: None,
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
        self.target = plan::target(cursor, view, &self.monitors);
        Ok(())
    }

    fn issue(&mut self, qh: &QueueHandle<State>) -> anyhow::Result<()> {
        let Some(pool) = self.pool.as_mut() else {
            return Ok(());
        };
        let output: &WlOutput = &self
            .outputs
            .iter()
            .find(|o: &&NamedOutput| o.name.as_deref() == Some(pool.shape.output.as_str()))
            .with_context(|| format!("no wayland output named {}", pool.shape.output))?
            .proxy;
        let manager: &ZwlrScreencopyManagerV1 = self
            .manager
            .as_ref()
            .context("screencopy manager not bound")?;
        let mode: CopyMode = plan::copy_mode(plan::held(&pool.slots).is_some());
        let pinned: Vec<bool> = pool
            .surfaces
            .iter()
            .map(|surface: &Surface| Arc::strong_count(&surface.surface) > 1)
            .collect();
        for index in plan::to_issue(&pool.slots, &pinned, IN_FLIGHT) {
            pool.slots[index] = Slot::Capturing {
                frame: manager.capture_output(0, output, qh, index),
                mode,
                offer: None,
            };
        }
        Ok(())
    }

    fn encode_if_due(&mut self, wanted: Shot) -> anyhow::Result<Duration> {
        let Some(pool) = self.pool.as_mut() else {
            return Ok(WAKE);
        };
        let Some((held, _)) = plan::held(&pool.slots) else {
            return Ok(WAKE);
        };
        let pending: Pending = Pending {
            new_frame: pool.new_frame,
            keyframe: self.keyframe.pending(),
            encoder_busy: pool.lane.busy(),
        };
        let since_last: Option<Duration> = pool.encoded_at.map(|at: Instant| at.elapsed());
        match schedule(pending, pool.shown, wanted, since_last) {
            Schedule::Idle | Schedule::Busy => Ok(WAKE),
            Schedule::WaitFor(left) => Ok(left.min(WAKE)),
            Schedule::EncodeNow => {
                let cropped: Cropped = pool
                    .cropper
                    .crop(&pool.surfaces[held].surface, wanted.crop)
                    .context("crop")?;
                pool.lane.submit(Job {
                    cropped,
                    keyframe: self.keyframe.take(),
                    shot: wanted,
                })?;
                pool.new_frame = false;
                pool.shown = Some(wanted);
                pool.encoded_at = Some(Instant::now());
                Ok(WAKE)
            }
        }
    }

    fn frame_event(
        &mut self,
        index: usize,
        frame: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
    ) {
        let Some(pool) = self.pool.as_mut() else {
            return;
        };
        let (Some(surface), Some(slot)) = (pool.surfaces.get(index), pool.slots.get_mut(index))
        else {
            return;
        };
        let Slot::Capturing { mode, offer, .. } = slot else {
            return;
        };
        match event {
            zwlr_screencopy_frame_v1::Event::LinuxDmabuf {
                format,
                width,
                height,
            } => {
                *offer = Some(Offer {
                    fourcc: format,
                    width,
                    height,
                });
            }
            zwlr_screencopy_frame_v1::Event::Flags {
                flags: WEnum::Value(flags),
            } => {
                if flags.contains(zwlr_screencopy_frame_v1::Flags::YInvert) {
                    self.fatal = Some(
                        "compositor sent a y-inverted frame; phonepad would stream it upside down"
                            .to_owned(),
                    );
                }
            }
            zwlr_screencopy_frame_v1::Event::BufferDone => {
                let plane: &Dmabuf = surface.surface.dmabuf();
                let fits: Offer = Offer {
                    fourcc: plane.fourcc,
                    width: plane.width,
                    height: plane.height,
                };
                match *offer {
                    Some(offered) if offered == fits => match mode {
                        CopyMode::Immediate => frame.copy(&surface.buffer),
                        CopyMode::OnDamage => frame.copy_with_damage(&surface.buffer),
                    },
                    Some(offered) => {
                        self.fatal = Some(format!(
                            "compositor offers dmabuf {offered:?} but the encoder surface is {fits:?}; scaled or rotated outputs are not supported"
                        ));
                    }
                    None => {
                        self.fatal = Some("compositor offered no dmabuf capture".to_owned());
                    }
                }
            }
            zwlr_screencopy_frame_v1::Event::Ready {
                tv_sec_hi,
                tv_sec_lo,
                tv_nsec,
            } => {
                frame.destroy();
                if plan::arrive(
                    &mut pool.slots,
                    index,
                    plan::stamp(tv_sec_hi, tv_sec_lo, tv_nsec),
                ) {
                    pool.new_frame = true;
                }
            }
            zwlr_screencopy_frame_v1::Event::Failed => {
                frame.destroy();
                *slot = Slot::Free;
                self.failed = true;
            }
            zwlr_screencopy_frame_v1::Event::Buffer { .. }
            | zwlr_screencopy_frame_v1::Event::Flags { .. }
            | zwlr_screencopy_frame_v1::Event::Damage { .. }
            | _ => {}
        }
    }
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut State,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<State>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_output" => {
                    let index: usize = state.outputs.len();
                    let proxy: WlOutput = registry.bind(name, version.min(4), qh, index);
                    state.outputs.push(NamedOutput { proxy, name: None });
                }
                "zwlr_screencopy_manager_v1" => {
                    state.manager = Some(registry.bind(name, version.min(3), qh, ()))
                }
                "zwp_linux_dmabuf_v1" => {
                    state.dmabuf = Some(registry.bind(name, version.min(3), qh, ()))
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<WlOutput, usize> for State {
    fn event(
        state: &mut State,
        _: &WlOutput,
        event: wl_output::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        if let wl_output::Event::Name { name } = event {
            state.outputs[*index].name = Some(name);
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, usize> for State {
    fn event(
        state: &mut State,
        frame: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        state.frame_event(*index, frame, event);
    }
}

impl Dispatch<ZwpLinuxBufferParamsV1, ()> for State {
    fn event(
        state: &mut State,
        _: &ZwpLinuxBufferParamsV1,
        event: zwp_linux_buffer_params_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        if let zwp_linux_buffer_params_v1::Event::Failed = event {
            state.fatal = Some("compositor rejected the encoder surface as a dmabuf".to_owned());
        }
    }
}

impl Dispatch<ZwpLinuxDmabufV1, ()> for State {
    fn event(
        _: &mut State,
        _: &ZwpLinuxDmabufV1,
        _: zwp_linux_dmabuf_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}

impl Dispatch<WlBuffer, ()> for State {
    fn event(
        _: &mut State,
        _: &WlBuffer,
        _: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for State {
    fn event(
        _: &mut State,
        _: &ZwlrScreencopyManagerV1,
        _: zwlr_screencopy_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}
