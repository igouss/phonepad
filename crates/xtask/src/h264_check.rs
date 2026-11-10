use crate::report::Report;
use phonepad_h264::{AccessUnit, CaptureSurface, Cropped, Cropper, EncodeError, H264, Size, nal};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::time::{Duration, Instant};

const RENDER_NODE: &str = "/dev/dri/renderD128";
const SOURCE: Size = Size {
    width: 2560,
    height: 1440,
};
const OUTPUT: Size = Size {
    width: 648,
    height: 1072,
};
const QP: u8 = 24;
const SURFACES: usize = 2;
const DELTAS: usize = 30;
const UNITS: usize = DELTAS + 2;
const STEP_X: u32 = 8;
const STEP_Y: u32 = 4;
const BAND: usize = 268;
const CHROMA_BLOCK: usize = 66;
const MIN_PSNR_DB: f64 = 35.0;
const AR24: u32 = u32::from_le_bytes(*b"AR24");
const LINEAR: u64 = 0;
const TIMED_ENCODES: usize = 300;
const MAX_CPU_PER_FRAME: Duration = Duration::from_millis(5);
const LATENCY_LIMIT: Duration = Duration::from_millis(17);
const PIPELINE_DEPTH: usize = 3;

struct Stages {
    cropper: Cropper,
    h264: H264,
}

impl Stages {
    fn open() -> Result<Stages, EncodeError> {
        let (cropper, h264): (Cropper, H264) =
            phonepad_h264::open(Path::new(RENDER_NODE), SOURCE, OUTPUT, QP)?;
        Ok(Stages { cropper, h264 })
    }

    fn encode(
        &mut self,
        surface: &Arc<CaptureSurface>,
        origin: (u32, u32),
        keyframe: bool,
    ) -> Result<AccessUnit, EncodeError> {
        let cropped: Cropped = self.cropper.crop(surface, origin, OUTPUT)?;
        self.h264.encode(cropped, keyframe)
    }
}

fn screen() -> Vec<u8> {
    let width: usize = SOURCE.width as usize;
    let height: usize = SOURCE.height as usize;
    let mut bgra: Vec<u8> = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let pixel: [u8; 3] = screen_pixel(x, y, width);
            bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 0xFF]);
        }
    }
    bgra
}

fn origin(index: usize) -> (u32, u32) {
    let step: u32 = u32::try_from(index).unwrap_or(u32::MAX);
    (
        step.wrapping_mul(STEP_X) % (SOURCE.width - OUTPUT.width),
        step.wrapping_mul(STEP_Y) % (SOURCE.height - OUTPUT.height),
    )
}

fn crop(screen_rgb: &[u8], (left, top): (u32, u32)) -> Vec<u8> {
    let source_row: usize = SOURCE.width as usize * 3;
    let row: usize = OUTPUT.width as usize * 3;
    let start: usize = left as usize * 3;
    screen_rgb
        .chunks_exact(source_row)
        .skip(top as usize)
        .take(OUTPUT.height as usize)
        .flat_map(|line: &[u8]| line[start..start + row].iter().copied())
        .collect()
}

fn screen_pixel(x: usize, y: usize, width: usize) -> [u8; 3] {
    let stroke: bool = (y % 24) < 14 && (x.is_multiple_of(9) || (y % 24) == 6) && (x / 54) % 3 != 2;
    let to_byte = |value: usize| u8::try_from(value % 256).unwrap_or(u8::MAX);
    match y / BAND {
        0 => [to_byte(x * 255 / width), to_byte(y * 255 / BAND), 0xC0],
        1 if stroke => [0x20, 0x20, 0x20],
        1 => [0xF2, 0xF2, 0xF2],
        2 => [
            [0xD0, 0x30, 0x30],
            [0x30, 0xA0, 0x40],
            [0x30, 0x50, 0xD0],
            [0xF0, 0xD0, 0x20],
        ][(x / CHROMA_BLOCK + y / CHROMA_BLOCK) % 4],
        _ if stroke => [0xE0, 0xE0, 0xE0],
        _ => [0x1E, 0x24, 0x30],
    }
}

fn rgb(bgra: &[u8]) -> Vec<u8> {
    bgra.as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p: &[u8; 4]| [p[2], p[1], p[0]])
        .collect()
}

fn psnr(expected: &[u8], actual: &[u8]) -> f64 {
    let squared: f64 = expected
        .iter()
        .zip(actual)
        .map(|(&e, &a)| (f64::from(e) - f64::from(a)).powi(2))
        .sum();
    let mse: f64 = squared / expected.len() as f64;
    if mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0 * 255.0 / mse).log10()
    }
}

fn encode_stream(stages: &mut Stages, bgra: &[u8]) -> Result<Vec<AccessUnit>, EncodeError> {
    let surfaces: Vec<Arc<CaptureSurface>> = filled_surfaces(&mut stages.cropper, bgra)?;
    let mut units: Vec<AccessUnit> = Vec::with_capacity(UNITS);
    for frame in 0..UNITS {
        units.push(stages.encode(
            &surfaces[frame % SURFACES],
            origin(frame),
            frame == UNITS - 1,
        )?);
    }
    Ok(units)
}

fn filled_surfaces(
    cropper: &mut Cropper,
    bgra: &[u8],
) -> Result<Vec<Arc<CaptureSurface>>, EncodeError> {
    let mut surfaces: Vec<Arc<CaptureSurface>> = Vec::with_capacity(SURFACES);
    for _ in 0..SURFACES {
        let mut surface: CaptureSurface = cropper.surface()?;
        surface.fill(bgra)?;
        surfaces.push(Arc::new(surface));
    }
    Ok(surfaces)
}

fn decode(stream: &Path) -> Result<Vec<u8>, String> {
    let output: Output = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(stream)
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .map_err(|e| format!("cannot run ffmpeg: {e}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(format!(
            "ffmpeg failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn idr_units(units: &[AccessUnit]) -> Vec<usize> {
    units
        .iter()
        .enumerate()
        .filter(|(_, unit)| nal::unit_types(&unit.data).contains(&nal::IDR))
        .map(|(index, _)| index)
        .collect()
}

fn parameter_sets_lead(unit: &AccessUnit) -> bool {
    let types: Vec<u8> = nal::unit_types(&unit.data);
    let first = |kind: u8| types.iter().position(|&t| t == kind);
    match (first(nal::SPS), first(nal::PPS), first(nal::IDR)) {
        (Some(sps), Some(pps), Some(idr)) => sps < idr && pps < idr,
        (None | Some(_), None | Some(_), None | Some(_)) => false,
    }
}

fn check_stream(report: &mut Report, units: &[AccessUnit], decoded: &[u8], screen_rgb: &[u8]) {
    let frame_bytes: usize = OUTPUT.width as usize * OUTPUT.height as usize * 3;
    let frames: Vec<&[u8]> = decoded.chunks(frame_bytes).collect();
    report.line(
        "frame-count",
        decoded.len() == UNITS * frame_bytes,
        format!(
            "{} frames decoded ({} bytes), expected {UNITS}",
            frames.len(),
            decoded.len()
        ),
    );
    let idr: Vec<usize> = idr_units(units);
    report.line(
        "idr-units",
        idr == [0, UNITS - 1],
        format!("NAL type 5 in units {idr:?}, expected [0, {}]", UNITS - 1),
    );
    let mismatched: Vec<usize> = units
        .iter()
        .enumerate()
        .filter(|(index, unit)| unit.key != idr.contains(index))
        .map(|(index, _)| index)
        .collect();
    report.line(
        "key-flags",
        mismatched.is_empty(),
        format!("units whose key flag disagrees with their NAL types: {mismatched:?}"),
    );
    let unled: Vec<usize> = idr
        .iter()
        .copied()
        .filter(|&index| !parameter_sets_lead(&units[index]))
        .collect();
    report.line(
        "sps-pps-before-idr",
        unled.is_empty() && !idr.is_empty(),
        format!("IDR units without SPS and PPS before the slice: {unled:?}"),
    );
    let scores: Vec<f64> = frames
        .iter()
        .enumerate()
        .map(|(index, frame)| psnr(&crop(screen_rgb, origin(index)), frame))
        .collect();
    let worst: f64 = scores.iter().copied().fold(f64::INFINITY, f64::min);
    report.line(
        "psnr",
        !scores.is_empty() && worst >= MIN_PSNR_DB,
        format!("worst frame {worst:.2} dB, threshold {MIN_PSNR_DB} dB"),
    );
}

fn process_cpu() -> Result<Duration, String> {
    let tasks: std::fs::ReadDir =
        std::fs::read_dir("/proc/self/task").map_err(|e| format!("/proc/self/task: {e}"))?;
    let mut total: u64 = 0;
    for task in tasks {
        let path: PathBuf = task
            .map_err(|e| format!("/proc/self/task: {e}"))?
            .path()
            .join("schedstat");
        let text: String =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let on_cpu: u64 = text
            .split_whitespace()
            .next()
            .and_then(|field: &str| field.parse().ok())
            .ok_or_else(|| format!("{}: unreadable", path.display()))?;
        total += on_cpu;
    }
    Ok(Duration::from_nanos(total))
}

struct Timing {
    cpu_per_frame: Duration,
    median: Duration,
    p95: Duration,
}

fn time_encodes(stages: &mut Stages, bgra: &[u8]) -> Result<Timing, String> {
    let surfaces: Vec<Arc<CaptureSurface>> =
        filled_surfaces(&mut stages.cropper, bgra).map_err(|e| e.to_string())?;
    let mut latencies: Vec<Duration> = Vec::with_capacity(TIMED_ENCODES);
    let cpu_before: Duration = process_cpu()?;
    for index in 0..TIMED_ENCODES {
        let start: Instant = Instant::now();
        stages
            .encode(&surfaces[index % SURFACES], origin(index), false)
            .map_err(|e| e.to_string())?;
        latencies.push(start.elapsed());
    }
    let cpu: Duration = process_cpu()?.saturating_sub(cpu_before);
    latencies.sort();
    let count: u32 = u32::try_from(TIMED_ENCODES).map_err(|e| e.to_string())?;
    Ok(Timing {
        cpu_per_frame: cpu / count,
        median: latencies[TIMED_ENCODES / 2],
        p95: latencies[TIMED_ENCODES * 95 / 100],
    })
}

fn pipelined_rate(stages: &mut Stages, bgra: &[u8]) -> Result<f64, String> {
    let surfaces: Vec<Arc<CaptureSurface>> =
        filled_surfaces(&mut stages.cropper, bgra).map_err(|e| e.to_string())?;
    let mut queued: VecDeque<Cropped> = VecDeque::with_capacity(PIPELINE_DEPTH);
    let start: Instant = Instant::now();
    for index in 0..TIMED_ENCODES {
        queued.push_back(
            stages
                .cropper
                .crop(&surfaces[index % SURFACES], origin(index), OUTPUT)
                .map_err(|e| e.to_string())?,
        );
        if queued.len() == PIPELINE_DEPTH
            && let Some(oldest) = queued.pop_front()
        {
            stages
                .h264
                .encode(oldest, false)
                .map_err(|e| e.to_string())?;
        }
    }
    while let Some(oldest) = queued.pop_front() {
        stages
            .h264
            .encode(oldest, false)
            .map_err(|e| e.to_string())?;
    }
    Ok(TIMED_ENCODES as f64 / start.elapsed().as_secs_f64())
}

fn check_surface(report: &mut Report, stages: &mut Stages) -> Result<(), EncodeError> {
    let surface: Arc<CaptureSurface> = Arc::new(stages.cropper.surface()?);
    let dmabuf: &phonepad_h264::Dmabuf = surface.dmabuf();
    report.line(
        "dmabuf-format",
        dmabuf.fourcc == AR24 && dmabuf.modifier == LINEAR,
        format!(
            "fourcc {:?}, modifier {:#x}, stride {}, offset {}",
            String::from_utf8_lossy(&dmabuf.fourcc.to_le_bytes()),
            dmabuf.modifier,
            dmabuf.stride,
            dmabuf.offset
        ),
    );
    let outside: (u32, u32) = (SOURCE.width - OUTPUT.width + 1, 0);
    let rejected: bool = matches!(
        stages.cropper.crop(&surface, outside, OUTPUT),
        Err(EncodeError::Crop { .. })
    );
    report.line(
        "crop-bounds",
        rejected,
        format!("a crop at {outside:?} that leaves the source is refused"),
    );
    let mut other: Stages = Stages::open()?;
    let refused: bool = matches!(
        other.cropper.crop(&surface, (0, 0), OUTPUT),
        Err(EncodeError::ForeignSurface)
    );
    report.line(
        "foreign-surface",
        refused,
        String::from("a surface from one encoder is refused by another"),
    );
    Ok(())
}

fn stream_file(units: &[AccessUnit]) -> Result<PathBuf, String> {
    let path: PathBuf =
        std::env::temp_dir().join(format!("phonepad-h264-check-{}.h264", std::process::id()));
    let bytes: Vec<u8> = units
        .iter()
        .flat_map(|unit| unit.data.iter().copied())
        .collect();
    std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

fn checks(report: &mut Report) -> Result<(), String> {
    let mut stages: Stages = Stages::open().map_err(|e| e.to_string())?;
    check_surface(report, &mut stages).map_err(|e| e.to_string())?;
    let bgra: Vec<u8> = screen();
    let units: Vec<AccessUnit> = encode_stream(&mut stages, &bgra).map_err(|e| e.to_string())?;
    let stream: PathBuf = stream_file(&units)?;
    let decoded: Result<Vec<u8>, String> = decode(&stream);
    std::fs::remove_file(&stream).map_err(|e| format!("{}: {e}", stream.display()))?;
    check_stream(report, &units, &decoded?, &rgb(&bgra));
    let bytes: usize = units.iter().map(|unit| unit.data.len()).sum();
    println!(
        "INFO stream: {bytes} bytes in {UNITS} units, first unit {} bytes",
        units.first().map_or(0, |unit| unit.data.len())
    );
    let timing: Timing = time_encodes(&mut stages, &bgra)?;
    report.line(
        "cpu-per-frame",
        timing.cpu_per_frame <= MAX_CPU_PER_FRAME,
        format!(
            "{:?} over {TIMED_ENCODES} encodes, ADR 0002 limit {MAX_CPU_PER_FRAME:?}",
            timing.cpu_per_frame
        ),
    );
    println!(
        "INFO latency: median {:?}, p95 {:?} over {TIMED_ENCODES} encodes; ADR 0002 limit {LATENCY_LIMIT:?}; load-dependent on this shared desktop, so not a gate",
        timing.median, timing.p95
    );
    let rate: f64 = pipelined_rate(&mut stages, &bgra)?;
    println!(
        "INFO pipelined: {rate:.0} units/s with {PIPELINE_DEPTH} crops in flight ahead of the encoder; load-dependent, so not a gate"
    );
    Ok(())
}

pub fn run() -> bool {
    let mut report: Report = Report::default();
    if let Err(error) = checks(&mut report) {
        report.line("h264-check", false, error);
    }
    report.passed()
}
