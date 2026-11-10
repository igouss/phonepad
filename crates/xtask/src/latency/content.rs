use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fs::{MemfdFlags, memfd_create};
use rustix::io::Errno;
use std::fs::File;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use wayland_client::backend::WaylandError;
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_buffer::{self, WlBuffer};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_shm::{Format, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::xdg_surface::{self, XdgSurface};
use wayland_protocols::xdg::shell::client::xdg_toplevel::{self, XdgToplevel};
use wayland_protocols::xdg::shell::client::xdg_wm_base::{self, XdgWmBase};

pub const LEVELS: [u8; 8] = [16, 48, 80, 112, 144, 176, 208, 240];
pub const BAND_ROWS: u32 = 128;
const BYTES_PER_PIXEL: u32 = 4;
const BUFFERS: usize = 2;
const IDLE: Duration = Duration::from_millis(50);
const MAP_TIMEOUT: Duration = Duration::from_secs(10);
const NANOS_PER_SECOND: u64 = 1_000_000_000;
const TITLE: &str = "phonepad latency content";

type Mapped = Result<(u32, u32), String>;

pub struct Content {
    stop: Arc<AtomicBool>,
    rate: Arc<AtomicU32>,
    commits: Arc<AtomicU64>,
    skipped: Arc<AtomicU64>,
    thread: JoinHandle<Result<(), String>>,
    pub size: (u32, u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Painted {
    pub commits: u64,
    pub skipped: u64,
}

pub fn level(frame: u64) -> u8 {
    LEVELS[(frame % LEVELS.len() as u64) as usize]
}

pub fn period(hz: u32) -> Option<Duration> {
    (hz > 0).then(|| Duration::from_nanos(NANOS_PER_SECOND / u64::from(hz)))
}

pub fn next_due(due: Duration, period: Duration, now: Duration) -> Duration {
    let missed: u128 = now.saturating_sub(due).as_nanos() / period.as_nanos().max(1);
    due + period * u32::try_from(missed + 1).unwrap_or(u32::MAX)
}

pub fn band(height: u32, centre: u32) -> (u32, u32) {
    let rows: u32 = BAND_ROWS.min(height);
    (
        centre
            .saturating_sub(rows / 2)
            .min(height.saturating_sub(rows)),
        rows,
    )
}

pub fn changes(levels: &[u8]) -> Vec<bool> {
    std::iter::once(false)
        .chain(
            levels
                .windows(2)
                .map(|pair: &[u8]| pair[0].abs_diff(pair[1]) > (LEVELS[1] - LEVELS[0]) / 2),
        )
        .take(levels.len())
        .collect()
}

struct State {
    size: Option<(i32, i32)>,
    configured: bool,
    closed: bool,
    free: [bool; BUFFERS],
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut State,
        _: &WlRegistry,
        _: <WlRegistry as wayland_client::Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}

impl Dispatch<XdgWmBase, ()> for State {
    fn event(
        _: &mut State,
        wm: &XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm.pong(serial);
        }
    }
}

impl Dispatch<XdgSurface, ()> for State {
    fn event(
        state: &mut State,
        surface: &XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
            state.configured = true;
        }
    }
}

impl Dispatch<XdgToplevel, ()> for State {
    fn event(
        state: &mut State,
        _: &XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        if let xdg_toplevel::Event::Configure { width, height, .. } = event {
            state.size = Some((width, height));
        } else if let xdg_toplevel::Event::Close = event {
            state.closed = true;
        }
    }
}

impl Dispatch<WlBuffer, usize> for State {
    fn event(
        state: &mut State,
        _: &WlBuffer,
        event: wl_buffer::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        if let wl_buffer::Event::Release = event {
            state.free[*index] = true;
        }
    }
}

delegate_noop!(State: ignore WlCompositor);
delegate_noop!(State: ignore WlShm);
delegate_noop!(State: ignore WlShmPool);
delegate_noop!(State: ignore WlSurface);

fn wait(queue: &mut EventQueue<State>, state: &mut State, timeout: Duration) -> Result<(), String> {
    queue.flush().map_err(|e| format!("wayland flush: {e}"))?;
    if let Some(guard) = queue.prepare_read() {
        let limit: Timespec = Timespec::try_from(timeout).map_err(|e| e.to_string())?;
        let ready: bool = {
            let mut fds: [PollFd<'_>; 1] = [PollFd::from_borrowed_fd(
                guard.connection_fd(),
                PollFlags::IN,
            )];
            match poll(&mut fds, Some(&limit)) {
                Ok(_) => !fds[0].revents().is_empty(),
                Err(Errno::INTR) => false,
                Err(error) => return Err(format!("poll wayland socket: {error}")),
            }
        };
        if ready {
            match guard.read() {
                Ok(_) => {}
                Err(WaylandError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(format!("wayland read: {error}")),
            }
        }
    }
    queue
        .dispatch_pending(state)
        .map(|_| ())
        .map_err(|e| format!("wayland dispatch: {e}"))
}

struct Surface {
    surface: WlSurface,
    buffers: [WlBuffer; BUFFERS],
    file: File,
    width: u32,
    height: u32,
}

impl Surface {
    fn paint(&self, state: &mut State, frame: u64) -> Result<bool, String> {
        let Some(index) = state.free.iter().position(|free: &bool| *free) else {
            return Ok(false);
        };
        let (first, rows): (u32, u32) = band(self.height, self.height / 2);
        let stride: u64 = u64::from(self.width * BYTES_PER_PIXEL);
        let shade: u8 = level(frame);
        let pixels: Vec<u8> = [shade, shade, shade, u8::MAX]
            .repeat(usize::try_from(self.width * rows).map_err(|e| e.to_string())?);
        let offset: u64 =
            index as u64 * stride * u64::from(self.height) + u64::from(first) * stride;
        self.file
            .write_at(&pixels, offset)
            .map_err(|e| format!("paint the content buffer: {e}"))?;
        state.free[index] = false;
        self.surface.attach(Some(&self.buffers[index]), 0, 0);
        self.surface.damage_buffer(
            0,
            i32::try_from(first).map_err(|e| e.to_string())?,
            i32::try_from(self.width).map_err(|e| e.to_string())?,
            i32::try_from(rows).map_err(|e| e.to_string())?,
        );
        self.surface.commit();
        Ok(true)
    }
}

fn map(
    queue: &mut EventQueue<State>,
    state: &mut State,
    globals: &GlobalList,
    fallback: (u32, u32),
) -> Result<Surface, String> {
    let qh: QueueHandle<State> = queue.handle();
    let compositor: WlCompositor = globals
        .bind(&qh, 4..=6, ())
        .map_err(|e| format!("wl_compositor: {e}"))?;
    let shm: WlShm = globals
        .bind(&qh, 1..=1, ())
        .map_err(|e| format!("wl_shm: {e}"))?;
    let wm: XdgWmBase = globals
        .bind(&qh, 1..=5, ())
        .map_err(|e| format!("xdg_wm_base: {e}"))?;
    let surface: WlSurface = compositor.create_surface(&qh, ());
    let xdg: XdgSurface = wm.get_xdg_surface(&surface, &qh, ());
    let toplevel: XdgToplevel = xdg.get_toplevel(&qh, ());
    toplevel.set_title(String::from(TITLE));
    surface.commit();
    let start: Instant = Instant::now();
    while !state.configured {
        if start.elapsed() > MAP_TIMEOUT {
            return Err(format!("no configure within {MAP_TIMEOUT:?}"));
        }
        wait(queue, state, IDLE)?;
    }
    let (width, height): (u32, u32) = match state.size {
        Some((width, height)) if width > 0 && height > 0 => (
            u32::try_from(width).map_err(|e| e.to_string())?,
            u32::try_from(height).map_err(|e| e.to_string())?,
        ),
        Some(_) | None => fallback,
    };
    let one: u32 = width * height * BYTES_PER_PIXEL;
    let file: File = File::from(
        memfd_create("phonepad-latency-content", MemfdFlags::CLOEXEC)
            .map_err(|e| format!("memfd: {e}"))?,
    );
    file.set_len(u64::from(one) * BUFFERS as u64)
        .map_err(|e| format!("size the content buffers: {e}"))?;
    let pool: WlShmPool = shm.create_pool(
        file.as_fd(),
        i32::try_from(one * BUFFERS as u32).map_err(|e| e.to_string())?,
        &qh,
        (),
    );
    let side = |value: u32| i32::try_from(value).map_err(|e| e.to_string());
    let buffer = |index: usize| -> Result<WlBuffer, String> {
        Ok(pool.create_buffer(
            side(one * u32::try_from(index).map_err(|e| e.to_string())?)?,
            side(width)?,
            side(height)?,
            side(width * BYTES_PER_PIXEL)?,
            Format::Xrgb8888,
            &qh,
            index,
        ))
    };
    let buffers: [WlBuffer; BUFFERS] = [buffer(0)?, buffer(1)?];
    pool.destroy();
    state.free = [true; BUFFERS];
    Ok(Surface {
        surface,
        buffers,
        file,
        width,
        height,
    })
}

struct Shared {
    stop: Arc<AtomicBool>,
    rate: Arc<AtomicU32>,
    commits: Arc<AtomicU64>,
    skipped: Arc<AtomicU64>,
}

fn run(
    socket: UnixStream,
    fallback: (u32, u32),
    shared: &Shared,
    mapped: &Sender<Mapped>,
) -> Result<(), String> {
    let connection: Connection =
        Connection::from_socket(socket).map_err(|e| format!("wayland connect: {e}"))?;
    let (globals, mut queue): (GlobalList, EventQueue<State>) =
        registry_queue_init(&connection).map_err(|e| format!("wayland registry: {e}"))?;
    let mut state: State = State {
        size: None,
        configured: false,
        closed: false,
        free: [false; BUFFERS],
    };
    let surface: Surface = map(&mut queue, &mut state, &globals, fallback)?;
    let start: Instant = Instant::now();
    let mut frame: u64 = 0;
    let mut due: Duration = Duration::ZERO;
    surface.paint(&mut state, frame)?;
    queue.flush().map_err(|e| format!("wayland flush: {e}"))?;
    let _receiver_gone: Result<(), mpsc::SendError<Mapped>> =
        mapped.send(Ok((surface.width, surface.height)));
    while !shared.stop.load(Ordering::Relaxed) && !state.closed {
        let Some(period) = period(shared.rate.load(Ordering::Relaxed)) else {
            wait(&mut queue, &mut state, IDLE)?;
            due = start.elapsed();
            continue;
        };
        let now: Duration = start.elapsed();
        if now >= due {
            frame += 1;
            let counter: &AtomicU64 = if surface.paint(&mut state, frame)? {
                &shared.commits
            } else {
                &shared.skipped
            };
            counter.fetch_add(1, Ordering::Relaxed);
            due = next_due(due, period, now);
        }
        wait(
            &mut queue,
            &mut state,
            due.saturating_sub(start.elapsed()).min(IDLE),
        )?;
    }
    Ok(())
}

impl Content {
    pub fn start(socket: &Path, fallback: (u32, u32), hz: u32) -> Result<Content, String> {
        let stream: UnixStream =
            UnixStream::connect(socket).map_err(|e| format!("{}: {e}", socket.display()))?;
        let shared: Shared = Shared {
            stop: Arc::new(AtomicBool::new(false)),
            rate: Arc::new(AtomicU32::new(hz)),
            commits: Arc::new(AtomicU64::new(0)),
            skipped: Arc::new(AtomicU64::new(0)),
        };
        let content: Shared = Shared {
            stop: Arc::clone(&shared.stop),
            rate: Arc::clone(&shared.rate),
            commits: Arc::clone(&shared.commits),
            skipped: Arc::clone(&shared.skipped),
        };
        let (mapped, ready): (Sender<Mapped>, Receiver<Mapped>) = mpsc::channel();
        let thread: JoinHandle<Result<(), String>> = thread::spawn(move || {
            let ran: Result<(), String> = run(stream, fallback, &content, &mapped);
            if let Err(error) = &ran {
                let _receiver_gone: Result<(), mpsc::SendError<Mapped>> =
                    mapped.send(Err(error.clone()));
            }
            ran
        });
        let size: (u32, u32) = ready
            .recv_timeout(MAP_TIMEOUT)
            .map_err(|e| format!("content window: {e}"))??;
        Ok(Content {
            stop: shared.stop,
            rate: shared.rate,
            commits: shared.commits,
            skipped: shared.skipped,
            thread,
            size,
        })
    }

    pub fn set(&self, hz: u32) {
        self.rate.store(hz, Ordering::Relaxed);
    }

    pub fn painted(&self) -> Painted {
        Painted {
            commits: self.commits.load(Ordering::Relaxed),
            skipped: self.skipped.load(Ordering::Relaxed),
        }
    }

    pub fn stop(self) -> Result<(), String> {
        self.stop.store(true, Ordering::Relaxed);
        self.thread
            .join()
            .map_err(|_| String::from("the content client panicked"))?
    }
}

#[cfg(test)]
mod tests;
