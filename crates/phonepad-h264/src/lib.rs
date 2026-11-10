mod ffi;
mod geometry;
pub mod nal;

use ffmpeg_next::format::Pixel;
use ffmpeg_next::util::color;
use ffmpeg_next::{Dictionary, Packet, Rational, encoder, filter, frame, picture};
use std::ffi::CString;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub use ffi::MapError;
pub use geometry::Size;
use geometry::{checked_geometry, checked_qp, crop_edges};

const COLOUR: &str = "format=nv12:out_color_matrix=bt709:out_color_primaries=bt709:out_color_transfer=bt709:out_range=limited";
const FRAME_RATE: i32 = 60;
const BGRA_BYTES: usize = 4;
const AGAIN: ffmpeg_next::Error = ffmpeg_next::Error::Other {
    errno: ffmpeg_sys_next::EAGAIN,
};

static NEXT_ENCODER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("output {0}: width and height must be even, non-zero and fit an i32")]
    OutputSize(Size),
    #[error("source {0}: width and height must be non-zero and fit an i32")]
    SourceSize(Size),
    #[error("output {output} does not fit inside source {screen}")]
    OutputExceedsSource { screen: Size, output: Size },
    #[error("crop at {origin:?} of size {output} leaves source {screen}")]
    Crop {
        origin: (u32, u32),
        screen: Size,
        output: Size,
    },
    #[error("qp {0} is outside 1..=51")]
    Qp(u8),
    #[error("render node path {0} contains a NUL byte")]
    RenderNode(PathBuf),
    #[error("FFmpeg has no h264_vaapi encoder")]
    NoEncoder,
    #[error("{step}: {source}")]
    Ffmpeg {
        step: &'static str,
        source: ffmpeg_next::Error,
    },
    #[error("export surface as dmabuf: {0}")]
    Export(MapError),
    #[error("the surface belongs to another encoder")]
    ForeignSurface,
    #[error("{0} produced nothing for one input frame")]
    Stalled(&'static str),
    #[error("expected {expected} bytes of BGRA, got {actual}")]
    Pixels { expected: usize, actual: usize },
}

fn step(step: &'static str) -> impl FnOnce(ffmpeg_next::Error) -> EncodeError {
    move |source: ffmpeg_next::Error| EncodeError::Ffmpeg { step, source }
}

fn drained(step: &'static str) -> impl FnOnce(ffmpeg_next::Error) -> EncodeError {
    move |source: ffmpeg_next::Error| {
        if source == AGAIN {
            EncodeError::Stalled(step)
        } else {
            EncodeError::Ffmpeg { step, source }
        }
    }
}

#[derive(Debug)]
pub struct Dmabuf {
    pub fd: OwnedFd,
    pub offset: u32,
    pub stride: u32,
    pub modifier: u64,
    pub fourcc: u32,
    pub width: u32,
    pub height: u32,
}

pub struct CaptureSurface {
    encoder: u64,
    frame: frame::Video,
    dmabuf: Dmabuf,
}

impl CaptureSurface {
    pub fn dmabuf(&self) -> &Dmabuf {
        &self.dmabuf
    }

    pub fn fill(&mut self, bgra: &[u8]) -> Result<(), EncodeError> {
        let width: u32 = self.dmabuf.width;
        let height: u32 = self.dmabuf.height;
        let row: usize = width as usize * BGRA_BYTES;
        let expected: usize = row * height as usize;
        if bgra.len() != expected {
            return Err(EncodeError::Pixels {
                expected,
                actual: bgra.len(),
            });
        }
        let mut pixels: frame::Video = frame::Video::new(Pixel::BGRA, width, height);
        let stride: usize = pixels.stride(0);
        for (destination, source) in pixels
            .data_mut(0)
            .chunks_mut(stride)
            .zip(bgra.chunks_exact(row))
        {
            destination[..row].copy_from_slice(source);
        }
        ffi::upload(&mut self.frame, &pixels).map_err(step("upload BGRA pixels"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessUnit {
    pub key: bool,
    pub data: Vec<u8>,
}

pub struct Cropper {
    id: u64,
    source: Size,
    frames: ffi::HwFrames,
    graph: filter::Graph,
}

pub struct Cropped {
    frame: frame::Video,
    source: Arc<CaptureSurface>,
}

pub struct H264 {
    next_pts: i64,
    encoder: encoder::video::Encoder,
}

fn convert_graph(
    frames: &ffi::HwFrames,
    output: Size,
    time_base: Rational,
) -> Result<filter::Graph, EncodeError> {
    let mut graph: filter::Graph = filter::Graph::new();
    frames
        .feed(&mut graph, c"in", time_base)
        .map_err(step("buffer source"))?;
    let sink: filter::Filter = filter::find("buffersink")
        .ok_or_else(|| step("find buffersink")(ffmpeg_next::Error::FilterNotFound))?;
    graph.add(&sink, "out", "").map_err(step("buffer sink"))?;
    graph
        .output("in", 0)
        .and_then(|parser| parser.input("out", 0))
        .and_then(|parser| {
            parser.parse(&format!(
                "scale_vaapi=w={}:h={}:{COLOUR}",
                output.width, output.height
            ))
        })
        .map_err(step("parse filter graph"))?;
    graph.validate().map_err(step("configure filter graph"))?;
    Ok(graph)
}

fn h264_encoder(
    graph: &mut filter::Graph,
    output: Size,
    qp: u8,
    time_base: Rational,
) -> Result<encoder::video::Encoder, EncodeError> {
    let codec: ffmpeg_next::Codec =
        encoder::find_by_name("h264_vaapi").ok_or(EncodeError::NoEncoder)?;
    let mut context: encoder::video::Video =
        ffi::encoder_context(codec).map_err(step("encoder context"))?;
    context.set_width(output.width);
    context.set_height(output.height);
    context.set_format(Pixel::VAAPI);
    context.set_time_base(time_base);
    context.set_frame_rate(Some(Rational::new(FRAME_RATE, 1)));
    context.set_gop(i32::MAX as u32);
    context.set_max_b_frames(0);
    context.set_color_range(color::Range::MPEG);
    context.set_colorspace(color::Space::BT709);
    context.set_color_primaries(color::Primaries::BT709);
    context.set_color_transfer_characteristic(color::TransferCharacteristic::BT709);
    let mut sink: filter::Context = graph
        .get("out")
        .ok_or_else(|| step("buffer sink")(ffmpeg_next::Error::FilterNotFound))?;
    ffi::adopt_sink_frames(&mut context, &mut sink).map_err(step("encoder frames"))?;
    let mut options: Dictionary = Dictionary::new();
    options.set("low_power", "1");
    options.set("rc_mode", "CQP");
    options.set("qp", &qp.to_string());
    options.set("async_depth", "1");
    context.open_with(options).map_err(step("open h264_vaapi"))
}

pub fn open(
    render_node: &Path,
    source: Size,
    output: Size,
    qp: u8,
) -> Result<(Cropper, H264), EncodeError> {
    let (w, h): (i32, i32) = checked_geometry(source, output)?;
    let qp: u8 = checked_qp(qp)?;
    let node: CString = CString::new(render_node.as_os_str().as_bytes())
        .map_err(|_| EncodeError::RenderNode(render_node.to_path_buf()))?;
    let time_base: Rational = Rational::new(1, FRAME_RATE);
    let frames: ffi::HwFrames =
        ffi::HwFrames::vaapi(&node, Pixel::BGRA, w, h).map_err(step("VA-API frames"))?;
    let mut graph: filter::Graph = convert_graph(&frames, output, time_base)?;
    let encoder: encoder::video::Encoder = h264_encoder(&mut graph, output, qp, time_base)?;
    Ok((
        Cropper {
            id: NEXT_ENCODER.fetch_add(1, Ordering::Relaxed),
            source,
            frames,
            graph,
        },
        H264 {
            next_pts: 0,
            encoder,
        },
    ))
}

impl Cropper {
    pub fn surface(&mut self) -> Result<CaptureSurface, EncodeError> {
        let frame: frame::Video = self.frames.surface().map_err(step("allocate surface"))?;
        let plane: ffi::DrmPlane = ffi::export_drm(&frame).map_err(EncodeError::Export)?;
        Ok(CaptureSurface {
            encoder: self.id,
            frame,
            dmabuf: Dmabuf {
                fd: plane.fd,
                offset: plane.offset,
                stride: plane.pitch,
                modifier: plane.modifier,
                fourcc: plane.fourcc,
                width: self.source.width,
                height: self.source.height,
            },
        })
    }

    pub fn crop(
        &mut self,
        surface: &Arc<CaptureSurface>,
        crop_origin: (u32, u32),
        crop_size: Size,
    ) -> Result<Cropped, EncodeError> {
        if surface.encoder != self.id {
            return Err(EncodeError::ForeignSurface);
        }
        let crop: ffi::Crop = crop_edges(self.source, crop_size, crop_origin)?;
        let mut source: filter::Context = self
            .graph
            .get("in")
            .ok_or_else(|| step("buffer source")(ffmpeg_next::Error::FilterNotFound))?;
        ffi::add_cropped(&mut source, &surface.frame, crop).map_err(step("feed surface"))?;
        let mut nv12: frame::Video = frame::Video::empty();
        self.graph
            .get("out")
            .ok_or_else(|| step("buffer sink")(ffmpeg_next::Error::FilterNotFound))?
            .sink()
            .frame(&mut nv12)
            .map_err(drained("scale_vaapi"))?;
        Ok(Cropped {
            frame: nv12,
            source: Arc::clone(surface),
        })
    }
}

impl H264 {
    pub fn encode(&mut self, cropped: Cropped, keyframe: bool) -> Result<AccessUnit, EncodeError> {
        let Cropped {
            frame: mut nv12,
            source: read_by_the_gpu,
        } = cropped;
        nv12.set_pts(Some(self.next_pts));
        if keyframe || self.next_pts == 0 {
            nv12.set_kind(picture::Type::I);
        }
        self.next_pts += 1;
        self.encoder.send_frame(&nv12).map_err(step("send frame"))?;
        let mut packet: Packet = Packet::empty();
        self.encoder
            .receive_packet(&mut packet)
            .map_err(drained("h264_vaapi"))?;
        drop(read_by_the_gpu);
        let data: Vec<u8> = packet.data().map(<[u8]>::to_vec).unwrap_or_default();
        Ok(AccessUnit {
            key: nal::contains_idr(&data),
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{AGAIN, EncodeError, drained};

    #[test]
    fn eagain_means_the_stage_stalled() {
        assert!(matches!(
            drained("scale_vaapi")(AGAIN),
            EncodeError::Stalled("scale_vaapi")
        ));
    }

    #[test]
    fn other_errors_keep_their_cause() {
        assert!(matches!(
            drained("h264_vaapi")(ffmpeg_next::Error::InvalidData),
            EncodeError::Ffmpeg {
                step: "h264_vaapi",
                source: ffmpeg_next::Error::InvalidData
            }
        ));
    }
}
