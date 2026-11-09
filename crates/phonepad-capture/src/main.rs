use std::fs::File;
use std::io::{self, BufRead, BufWriter, Stdout, Write};
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use rustix::fs::{MemfdFlags, memfd_create};
use wayland_client::protocol::wl_buffer::{self, WlBuffer};
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

const IN_FLIGHT: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Region {
    output: String,
    global_x: i32,
    global_y: i32,
    local_x: i32,
    local_y: i32,
    width: i32,
    height: i32,
}

impl Region {
    fn parse(line: &str) -> Option<Region> {
        let mut parts = line.split_whitespace();
        let output: String = parts.next()?.to_owned();
        let numbers: Vec<i32> = parts.map(str::parse).collect::<Result<_, _>>().ok()?;
        let [global_x, global_y, local_x, local_y, width, height]: [i32; 6] =
            numbers.try_into().ok()?;
        (width > 0 && height > 0).then_some(Region {
            output,
            global_x,
            global_y,
            local_x,
            local_y,
            width,
            height,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Layout {
    format: wl_shm::Format,
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

struct Capture {
    region: Region,
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
    wanted: Arc<Mutex<Region>>,
    buffers: Vec<Option<Buffer>>,
    captures: Vec<Option<Capture>>,
    out: BufWriter<Stdout>,
    fatal: Option<String>,
}

impl State {
    fn issue(&mut self, slot: usize, qh: &QueueHandle<State>) {
        let region: Region = self
            .wanted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let Some(output) = self
            .outputs
            .iter()
            .find(|o| o.name.as_deref() == Some(region.output.as_str()))
        else {
            self.fatal = Some(format!("no output named {}", region.output));
            return;
        };
        let Some(manager) = self.manager.as_ref() else {
            self.fatal = Some("screencopy manager not bound".to_owned());
            return;
        };
        manager.capture_output_region(
            0,
            &output.proxy,
            region.local_x,
            region.local_y,
            region.width,
            region.height,
            qh,
            slot,
        );
        self.captures[slot] = Some(Capture {
            region,
            layout: None,
            y_invert: false,
        });
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
        let file: File = File::from(memfd_create("phonepad-capture", MemfdFlags::CLOEXEC)?);
        file.set_len(layout.bytes() as u64)?;
        let shm: &WlShm = self
            .shm
            .as_ref()
            .ok_or_else(|| io::Error::other("wl_shm not bound"))?;
        let pool: WlShmPool = shm.create_pool(file.as_fd(), layout.bytes() as i32, qh, ());
        let buffer: WlBuffer = pool.create_buffer(
            0,
            layout.width as i32,
            layout.height as i32,
            layout.stride as i32,
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

    fn emit(&mut self, slot: usize) -> io::Result<()> {
        let capture: Capture = self.captures[slot]
            .take()
            .ok_or_else(|| io::Error::other("no capture in flight"))?;
        let buffer: &Buffer = self.buffers[slot]
            .as_ref()
            .ok_or_else(|| io::Error::other("no buffer attached"))?;
        let layout: Layout = buffer.layout;
        let mut pixels: Vec<u8> = vec![0; layout.bytes()];
        buffer.file.read_exact_at(&mut pixels, 0)?;
        let header: [u32; 7] = [
            capture.region.global_x as u32,
            capture.region.global_y as u32,
            layout.width,
            layout.height,
            layout.stride,
            layout.format as u32,
            u32::from(capture.y_invert),
        ];
        for word in header {
            self.out.write_all(&word.to_le_bytes())?;
        }
        self.out.write_all(&pixels)?;
        self.out.flush()
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
                if let Some(capture) = state.captures[slot].as_mut() {
                    capture.layout = Some(Layout {
                        format,
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
                let layout: Option<Layout> = state.captures[slot].as_ref().and_then(|c| c.layout);
                let Some(layout) = layout else {
                    state.fatal = Some("compositor offered no shm buffer".to_owned());
                    return;
                };
                match state.buffer_for(slot, layout, qh) {
                    Ok(buffer) => frame.copy(&buffer),
                    Err(err) => state.fatal = Some(format!("shm buffer: {err}")),
                }
            }
            zwlr_screencopy_frame_v1::Event::Ready { .. } => {
                frame.destroy();
                if let Err(err) = state.emit(slot) {
                    state.fatal = Some(format!("stdout: {err}"));
                    return;
                }
                state.issue(slot, qh);
            }
            zwlr_screencopy_frame_v1::Event::Failed => {
                frame.destroy();
                state.captures[slot] = None;
                state.issue(slot, qh);
            }
            zwlr_screencopy_frame_v1::Event::Buffer { .. }
            | zwlr_screencopy_frame_v1::Event::Flags { .. }
            | zwlr_screencopy_frame_v1::Event::Damage { .. }
            | zwlr_screencopy_frame_v1::Event::LinuxDmabuf { .. }
            | _ => {}
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

fn read_region(line: io::Result<String>) -> Result<Region, String> {
    let line: String = line.map_err(|err| format!("stdin: {err}"))?;
    Region::parse(&line).ok_or_else(|| format!("bad region line: {line:?}"))
}

fn run() -> Result<(), String> {
    let mut lines = io::stdin().lock().lines();
    let first: Region = read_region(lines.next().ok_or("stdin closed before first region")?)?;
    let wanted: Arc<Mutex<Region>> = Arc::new(Mutex::new(first));
    let updater: Arc<Mutex<Region>> = Arc::clone(&wanted);
    thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            match read_region(line) {
                Ok(region) => *updater.lock().unwrap_or_else(PoisonError::into_inner) = region,
                Err(err) => eprintln!("phonepad-capture: {err}"),
            }
        }
        std::process::exit(0);
    });

    let conn: Connection =
        Connection::connect_to_env().map_err(|err| format!("wayland connect: {err}"))?;
    let mut queue: EventQueue<State> = conn.new_event_queue();
    let qh: QueueHandle<State> = queue.handle();
    conn.display().get_registry(&qh, ());
    let mut state: State = State {
        shm: None,
        manager: None,
        outputs: Vec::new(),
        wanted,
        buffers: (0..IN_FLIGHT).map(|_| None).collect(),
        captures: (0..IN_FLIGHT).map(|_| None).collect(),
        out: BufWriter::with_capacity(1 << 22, io::stdout()),
        fatal: None,
    };
    queue
        .roundtrip(&mut state)
        .map_err(|err| format!("roundtrip: {err}"))?;
    queue
        .roundtrip(&mut state)
        .map_err(|err| format!("roundtrip: {err}"))?;
    if state.shm.is_none() || state.manager.is_none() {
        return Err("compositor lacks wl_shm or zwlr_screencopy_manager_v1".to_owned());
    }
    for slot in 0..IN_FLIGHT {
        state.issue(slot, &qh);
    }
    loop {
        if let Some(fatal) = state.fatal.take() {
            return Err(fatal);
        }
        queue
            .blocking_dispatch(&mut state)
            .map_err(|err| format!("dispatch: {err}"))?;
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("phonepad-capture: {err}");
            ExitCode::FAILURE
        }
    }
}
