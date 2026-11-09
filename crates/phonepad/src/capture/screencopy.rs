use std::fs::File;
use std::io;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use phonepad_core::geometry::{Monitor, Point, Region, Size, region_around};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fs::{MemfdFlags, memfd_create};
use rustix::io::Errno;
use tokio::sync::watch;
use wayland_client::backend::WaylandError;
use wayland_client::protocol::wl_buffer::{self, WlBuffer};
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::{self, WlShmPool};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::{
    self, ZwlrScreencopyFrameV1,
};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::{
    self, ZwlrScreencopyManagerV1,
};

use crate::hypr;

use super::format::channel_order;
use super::plan::{CopyMode, IN_FLIGHT, IssuePlan, is_new_image, plan_issue};
use super::{ChannelOrder, Frame, LatestFrame};

const RETRY: Duration = Duration::from_millis(100);
const QUIET_RECHECK: Duration = Duration::from_millis(100);
const MONITORS_MAX_AGE: Duration = Duration::from_secs(1);
const WAKE: Timespec = Timespec {
    tv_sec: 0,
    tv_nsec: 16_000_000,
};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Layout {
    format: wl_shm::Format,
    order: ChannelOrder,
    width: u32,
    height: u32,
    stride: u32,
}

impl Layout {
    fn bytes(self) -> usize {
        self.stride as usize * self.height as usize
    }
}

struct Buffer {
    file: File,
    layout: Layout,
    pool: WlShmPool,
    buffer: WlBuffer,
}

struct InFlight {
    region: Region,
    mode: CopyMode,
    frame: ZwlrScreencopyFrameV1,
    layout: Option<Layout>,
    y_invert: bool,
}

struct NamedOutput {
    proxy: WlOutput,
    name: Option<String>,
}

struct State {
    shm: Option<WlShm>,
    manager: Option<ZwlrScreencopyManagerV1>,
    outputs: Vec<NamedOutput>,
    view: watch::Receiver<Option<Size>>,
    cursor: watch::Receiver<Option<Point>>,
    frames: watch::Sender<LatestFrame>,
    buffers: [Option<Buffer>; IN_FLIGHT],
    captures: [Option<InFlight>; IN_FLIGHT],
    wanted: Option<Region>,
    issued: Option<Region>,
    monitors: Vec<Monitor>,
    monitors_at: Option<Instant>,
    published: LatestFrame,
    unchanged_at: Option<Instant>,
    scratch: Vec<u8>,
    syncs: u32,
    failed: bool,
    fatal: Option<String>,
}

pub(super) fn run(
    stop: &AtomicBool,
    view: watch::Receiver<Option<Size>>,
    cursor: watch::Receiver<Option<Point>>,
    frames: watch::Sender<LatestFrame>,
) -> anyhow::Result<()> {
    let conn: Connection = Connection::connect_to_env().context("wayland connect")?;
    let mut queue: EventQueue<State> = conn.new_event_queue();
    let qh: QueueHandle<State> = queue.handle();
    conn.display().get_registry(&qh, ());
    let mut state: State = State {
        shm: None,
        manager: None,
        outputs: Vec::new(),
        view,
        cursor,
        frames,
        buffers: [const { None }; IN_FLIGHT],
        captures: [const { None }; IN_FLIGHT],
        wanted: None,
        issued: None,
        monitors: Vec::new(),
        monitors_at: None,
        published: None,
        unchanged_at: None,
        scratch: Vec::new(),
        syncs: 0,
        failed: false,
        fatal: None,
    };
    for _ in 0..2 {
        conn.display().sync(&qh, ());
        let target: u32 = state.syncs + 1;
        while state.syncs < target {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            wait(&mut queue, &mut state)?;
        }
    }
    if state.shm.is_none() || state.manager.is_none() {
        bail!("compositor lacks wl_shm or zwlr_screencopy_manager_v1");
    }
    while !stop.load(Ordering::Relaxed) {
        if let Some(fatal) = state.fatal.take() {
            bail!(fatal);
        }
        if std::mem::take(&mut state.failed) {
            thread::sleep(RETRY);
            continue;
        }
        state.fill(&qh);
        wait(&mut queue, &mut state)?;
    }
    Ok(())
}

fn wait(queue: &mut EventQueue<State>, state: &mut State) -> anyhow::Result<()> {
    queue.flush().context("wayland flush")?;
    if let Some(guard) = queue.prepare_read() {
        let ready: usize = {
            let mut fds: [PollFd<'_>; 1] = [PollFd::from_borrowed_fd(
                guard.connection_fd(),
                PollFlags::IN,
            )];
            match poll(&mut fds, Some(&WAKE)) {
                Ok(ready) => ready,
                Err(Errno::INTR) => 0,
                Err(err) => return Err(err).context("poll wayland socket"),
            }
        };
        if ready > 0 {
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
    fn in_flight(&self) -> usize {
        self.captures
            .iter()
            .filter(|c: &&Option<InFlight>| c.is_some())
            .count()
    }

    fn fill(&mut self, qh: &QueueHandle<State>) {
        if let Err(err) = self.refresh_wanted() {
            eprintln!("capture: {err:#}");
            return;
        }
        let Some(wanted) = self.wanted.clone() else {
            return;
        };
        let moved: bool = self.issued.as_ref() != Some(&wanted);
        if moved {
            self.cancel_on_damage();
        }
        let recently_unchanged: bool = self
            .unchanged_at
            .is_some_and(|at: Instant| at.elapsed() < QUIET_RECHECK);
        let plan: IssuePlan = plan_issue(moved, self.in_flight(), recently_unchanged);
        let idle: Vec<usize> = (0..IN_FLIGHT)
            .filter(|slot: &usize| self.captures[*slot].is_none())
            .take(plan.count)
            .collect();
        if idle.is_empty() {
            return;
        }
        match idle
            .into_iter()
            .try_for_each(|slot: usize| self.issue(slot, &wanted, plan.mode, qh))
        {
            Ok(()) => self.issued = Some(wanted),
            Err(err) => eprintln!("capture: {err:#}"),
        }
    }

    fn refresh_wanted(&mut self) -> anyhow::Result<()> {
        let changed: bool =
            self.view.has_changed().unwrap_or(false) || self.cursor.has_changed().unwrap_or(false);
        if !changed && self.wanted.is_some() {
            return Ok(());
        }
        let view: Option<Size> = *self.view.borrow_and_update();
        let cursor: Option<Point> = *self.cursor.borrow_and_update();
        let (Some(view), Some(cursor)) = (view, cursor) else {
            self.wanted = None;
            return Ok(());
        };
        if self
            .monitors_at
            .is_none_or(|at: Instant| at.elapsed() > MONITORS_MAX_AGE)
        {
            self.monitors = hypr::monitors()?;
            self.monitors_at = Some(Instant::now());
        }
        self.wanted = region_around(cursor, view, &self.monitors);
        Ok(())
    }

    fn cancel_on_damage(&mut self) {
        for slot in 0..IN_FLIGHT {
            if self.captures[slot]
                .as_ref()
                .is_some_and(|c: &InFlight| c.mode == CopyMode::OnDamage)
                && let Some(cancelled) = self.captures[slot].take()
            {
                cancelled.frame.destroy();
                if let Some(buffer) = self.buffers[slot].take() {
                    buffer.buffer.destroy();
                    buffer.pool.destroy();
                }
            }
        }
    }

    fn issue(
        &mut self,
        slot: usize,
        region: &Region,
        mode: CopyMode,
        qh: &QueueHandle<State>,
    ) -> anyhow::Result<()> {
        let output: &WlOutput = &self
            .outputs
            .iter()
            .find(|o: &&NamedOutput| o.name.as_deref() == Some(region.output.as_str()))
            .with_context(|| format!("no wayland output named {}", region.output))?
            .proxy;
        let manager: &ZwlrScreencopyManagerV1 = self
            .manager
            .as_ref()
            .context("screencopy manager not bound")?;
        let frame: ZwlrScreencopyFrameV1 = manager.capture_output_region(
            0,
            output,
            region.local.x,
            region.local.y,
            i32::try_from(region.size.w.get())?,
            i32::try_from(region.size.h.get())?,
            qh,
            slot,
        );
        self.captures[slot] = Some(InFlight {
            region: region.clone(),
            mode,
            frame,
            layout: None,
            y_invert: false,
        });
        Ok(())
    }

    fn buffer_for(
        &mut self,
        slot: usize,
        layout: Layout,
        qh: &QueueHandle<State>,
    ) -> io::Result<WlBuffer> {
        if let Some(existing) = &self.buffers[slot]
            && existing.layout == layout
        {
            return Ok(existing.buffer.clone());
        }
        if let Some(old) = self.buffers[slot].take() {
            old.buffer.destroy();
            old.pool.destroy();
        }
        let bytes: i32 = i32::try_from(layout.bytes()).map_err(io::Error::other)?;
        let file: File = File::from(memfd_create("phonepad-capture", MemfdFlags::CLOEXEC)?);
        file.set_len(layout.bytes() as u64)?;
        let shm: &WlShm = self
            .shm
            .as_ref()
            .ok_or_else(|| io::Error::other("wl_shm not bound"))?;
        let pool: WlShmPool = shm.create_pool(file.as_fd(), bytes, qh, ());
        let buffer: WlBuffer = pool.create_buffer(
            0,
            i32::try_from(layout.width).map_err(io::Error::other)?,
            i32::try_from(layout.height).map_err(io::Error::other)?,
            i32::try_from(layout.stride).map_err(io::Error::other)?,
            layout.format,
            qh,
            (),
        );
        self.buffers[slot] = Some(Buffer {
            file,
            layout,
            pool,
            buffer: buffer.clone(),
        });
        Ok(buffer)
    }

    fn publish(&mut self, slot: usize) -> io::Result<()> {
        let capture: InFlight = self.captures[slot]
            .take()
            .ok_or_else(|| io::Error::other("no capture in flight"))?;
        let buffer: &Buffer = self.buffers[slot]
            .as_ref()
            .ok_or_else(|| io::Error::other("no buffer attached"))?;
        let layout: Layout = buffer.layout;
        let mut pixels: Vec<u8> = std::mem::take(&mut self.scratch);
        pixels.resize(layout.bytes(), 0);
        buffer.file.read_exact_at(&mut pixels, 0)?;
        let captured: Frame = Frame {
            origin: capture.region.global,
            width: layout.width,
            height: layout.height,
            stride: layout.stride,
            order: layout.order,
            y_invert: capture.y_invert,
            pixels,
        };
        if is_new_image(self.published.as_deref(), &captured) {
            let frame: Arc<Frame> = Arc::new(captured);
            self.published = Some(Arc::clone(&frame));
            self.frames.send_replace(Some(frame));
            self.unchanged_at = None;
        } else {
            self.scratch = captured.pixels;
            self.unchanged_at = Some(Instant::now());
        }
        Ok(())
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
                "wl_shm" => state.shm = Some(registry.bind(name, 1, qh, ())),
                "wl_output" => {
                    let index: usize = state.outputs.len();
                    let proxy: WlOutput = registry.bind(name, version.min(4), qh, index);
                    state.outputs.push(NamedOutput { proxy, name: None });
                }
                "zwlr_screencopy_manager_v1" => {
                    state.manager = Some(registry.bind(name, version.min(3), qh, ()))
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
        slot: &usize,
        _: &Connection,
        qh: &QueueHandle<State>,
    ) {
        let slot: usize = *slot;
        match event {
            zwlr_screencopy_frame_v1::Event::Buffer {
                format: WEnum::Value(format),
                width,
                height,
                stride,
            } => {
                if let Some(capture) = state.captures[slot].as_mut()
                    && capture.layout.is_none()
                    && let Some(order) = channel_order(format)
                {
                    capture.layout = Some(Layout {
                        format,
                        order,
                        width,
                        height,
                        stride,
                    });
                }
            }
            zwlr_screencopy_frame_v1::Event::Flags {
                flags: WEnum::Value(flags),
            } => {
                if let Some(capture) = state.captures[slot].as_mut() {
                    capture.y_invert = flags.contains(zwlr_screencopy_frame_v1::Flags::YInvert);
                }
            }
            zwlr_screencopy_frame_v1::Event::BufferDone => {
                let pending: Option<(Option<Layout>, CopyMode)> = state.captures[slot]
                    .as_ref()
                    .map(|c: &InFlight| (c.layout, c.mode));
                let Some((layout, mode)) = pending else {
                    return;
                };
                let Some(layout) = layout else {
                    state.fatal = Some("compositor offered no supported shm format".to_owned());
                    return;
                };
                match state.buffer_for(slot, layout, qh) {
                    Ok(buffer) => match mode {
                        CopyMode::Immediate => frame.copy(&buffer),
                        CopyMode::OnDamage => frame.copy_with_damage(&buffer),
                    },
                    Err(err) => state.fatal = Some(format!("shm buffer: {err}")),
                }
            }
            zwlr_screencopy_frame_v1::Event::Ready { .. } => {
                frame.destroy();
                if let Err(err) = state.publish(slot) {
                    state.fatal = Some(format!("read frame: {err}"));
                }
            }
            zwlr_screencopy_frame_v1::Event::Failed => {
                frame.destroy();
                state.captures[slot] = None;
                state.failed = true;
            }
            zwlr_screencopy_frame_v1::Event::Buffer { .. }
            | zwlr_screencopy_frame_v1::Event::Flags { .. }
            | zwlr_screencopy_frame_v1::Event::Damage { .. }
            | zwlr_screencopy_frame_v1::Event::LinuxDmabuf { .. }
            | _ => {}
        }
    }
}

impl Dispatch<WlCallback, ()> for State {
    fn event(
        state: &mut State,
        _: &WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.syncs += 1;
        }
    }
}

impl Dispatch<WlShm, ()> for State {
    fn event(
        _: &mut State,
        _: &WlShm,
        _: wl_shm::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}

impl Dispatch<WlShmPool, ()> for State {
    fn event(
        _: &mut State,
        _: &WlShmPool,
        _: wl_shm_pool::Event,
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
