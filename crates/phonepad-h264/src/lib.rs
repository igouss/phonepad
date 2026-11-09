mod ffi;
pub mod nal;

use ffmpeg_next::format::Pixel;
use ffmpeg_next::util::color;
use ffmpeg_next::{Dictionary, Packet, Rational, encoder, filter, frame, picture};
use std::ffi::CString;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub use ffi::MapError;

const COLOUR: &str = "format=nv12:out_color_matrix=bt709:out_color_primaries=bt709:out_color_transfer=bt709:out_range=limited";
const FRAME_RATE: i32 = 60;
const MAX_QP: u8 = 51;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl std::fmt::Display for Size {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
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

pub struct Encoder {
    id: u64,
    source: Size,
    output: Size,
    next_pts: i64,
    frames: ffi::HwFrames,
    graph: filter::Graph,
    encoder: encoder::video::Encoder,
}

fn sides(size: Size) -> Option<(i32, i32)> {
    match (i32::try_from(size.width), i32::try_from(size.height)) {
        (Ok(width), Ok(height)) if width > 0 && height > 0 => Some((width, height)),
        (Ok(_) | Err(_), Ok(_) | Err(_)) => None,
    }
}

fn checked_geometry(source: Size, output: Size) -> Result<(i32, i32), EncodeError> {
    let source_sides: (i32, i32) = sides(source).ok_or(EncodeError::SourceSize(source))?;
    if output.width % 2 == 1 || output.height % 2 == 1 || sides(output).is_none() {
        return Err(EncodeError::OutputSize(output));
    }
    if output.width > source.width || output.height > source.height {
        return Err(EncodeError::OutputExceedsSource {
            screen: source,
            output,
        });
    }
    Ok(source_sides)
}

fn crop_edges(source: Size, output: Size, origin: (u32, u32)) -> Result<ffi::Crop, EncodeError> {
    let outside = || EncodeError::Crop {
        origin,
        screen: source,
        output,
    };
    let right: u32 = source
        .width
        .checked_sub(origin.0)
        .and_then(|room: u32| room.checked_sub(output.width))
        .ok_or_else(outside)?;
    let bottom: u32 = source
        .height
        .checked_sub(origin.1)
        .and_then(|room: u32| room.checked_sub(output.height))
        .ok_or_else(outside)?;
    Ok(ffi::Crop {
        left: origin.0,
        top: origin.1,
        right,
        bottom,
    })
}

fn checked_qp(qp: u8) -> Result<u8, EncodeError> {
    match qp {
        1..=MAX_QP => Ok(qp),
        other => Err(EncodeError::Qp(other)),
    }
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

impl Encoder {
    pub fn open(
        render_node: &Path,
        source: Size,
        output: Size,
        qp: u8,
    ) -> Result<Encoder, EncodeError> {
        let (w, h): (i32, i32) = checked_geometry(source, output)?;
        let qp: u8 = checked_qp(qp)?;
        let node: CString = CString::new(render_node.as_os_str().as_bytes())
            .map_err(|_| EncodeError::RenderNode(render_node.to_path_buf()))?;
        let time_base: Rational = Rational::new(1, FRAME_RATE);
        let frames: ffi::HwFrames =
            ffi::HwFrames::vaapi(&node, Pixel::BGRA, w, h).map_err(step("VA-API frames"))?;
        let mut graph: filter::Graph = convert_graph(&frames, output, time_base)?;
        let encoder: encoder::video::Encoder = h264_encoder(&mut graph, output, qp, time_base)?;
        Ok(Encoder {
            id: NEXT_ENCODER.fetch_add(1, Ordering::Relaxed),
            source,
            output,
            next_pts: 0,
            frames,
            graph,
            encoder,
        })
    }

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

    pub fn encode(
        &mut self,
        surface: &CaptureSurface,
        crop_origin: (u32, u32),
        keyframe: bool,
    ) -> Result<AccessUnit, EncodeError> {
        if surface.encoder != self.id {
            return Err(EncodeError::ForeignSurface);
        }
        let crop: ffi::Crop = crop_edges(self.source, self.output, crop_origin)?;
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
        let data: Vec<u8> = packet.data().map(<[u8]>::to_vec).unwrap_or_default();
        Ok(AccessUnit {
            key: nal::contains_idr(&data),
            data,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{AGAIN, EncodeError, Size, checked_geometry, checked_qp, crop_edges, drained, ffi};

    const SCREEN: Size = Size {
        width: 2560,
        height: 1440,
    };
    const VIEW: Size = Size {
        width: 648,
        height: 1072,
    };

    fn size(width: u32, height: u32) -> Size {
        Size { width, height }
    }

    #[test]
    fn even_output_inside_the_source_is_accepted() {
        assert!(matches!(checked_geometry(SCREEN, VIEW), Ok((2560, 1440))));
    }

    #[test]
    fn odd_source_holding_an_even_output_is_accepted() {
        assert!(matches!(
            checked_geometry(size(649, 1073), size(648, 1072)),
            Ok((649, 1073))
        ));
    }

    #[test]
    fn output_equal_to_the_source_is_accepted() {
        assert!(matches!(checked_geometry(VIEW, VIEW), Ok((648, 1072))));
    }

    #[test]
    fn size_displays_as_width_by_height() {
        assert_eq!(VIEW.to_string(), "648x1072");
    }

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

    #[test]
    fn odd_output_width_is_rejected() {
        assert!(matches!(
            checked_geometry(SCREEN, size(647, 1072)),
            Err(EncodeError::OutputSize(Size {
                width: 647,
                height: 1072
            }))
        ));
    }

    #[test]
    fn odd_output_height_is_rejected() {
        assert!(matches!(
            checked_geometry(SCREEN, size(648, 1071)),
            Err(EncodeError::OutputSize(_))
        ));
    }

    #[test]
    fn zero_output_is_rejected() {
        assert!(matches!(
            checked_geometry(SCREEN, size(0, 1072)),
            Err(EncodeError::OutputSize(_))
        ));
    }

    #[test]
    fn zero_source_is_rejected() {
        assert!(matches!(
            checked_geometry(size(2560, 0), VIEW),
            Err(EncodeError::SourceSize(_))
        ));
    }

    #[test]
    fn source_beyond_i32_is_rejected() {
        assert!(matches!(
            checked_geometry(size(1 << 31, 1440), VIEW),
            Err(EncodeError::SourceSize(_))
        ));
    }

    #[test]
    fn output_taller_than_the_source_is_rejected() {
        assert!(matches!(
            checked_geometry(size(2560, 1070), VIEW),
            Err(EncodeError::OutputExceedsSource { .. })
        ));
    }

    #[test]
    fn output_wider_than_the_source_is_rejected() {
        assert!(matches!(
            checked_geometry(size(646, 1440), VIEW),
            Err(EncodeError::OutputExceedsSource { .. })
        ));
    }

    #[test]
    fn crop_at_the_origin_leaves_the_rest_right_and_below() {
        assert_eq!(
            crop_edges(SCREEN, VIEW, (0, 0)).ok(),
            Some(ffi::Crop {
                left: 0,
                top: 0,
                right: 1912,
                bottom: 368
            })
        );
    }

    #[test]
    fn crop_touching_the_far_corner_is_accepted() {
        assert_eq!(
            crop_edges(SCREEN, VIEW, (1912, 368)).ok(),
            Some(ffi::Crop {
                left: 1912,
                top: 368,
                right: 0,
                bottom: 0
            })
        );
    }

    #[test]
    fn crop_one_pixel_past_the_right_edge_is_rejected() {
        assert!(matches!(
            crop_edges(SCREEN, VIEW, (1913, 0)),
            Err(EncodeError::Crop {
                origin: (1913, 0),
                ..
            })
        ));
    }

    #[test]
    fn crop_one_pixel_past_the_bottom_edge_is_rejected() {
        assert!(matches!(
            crop_edges(SCREEN, VIEW, (0, 369)),
            Err(EncodeError::Crop { .. })
        ));
    }

    #[test]
    fn crop_origin_outside_the_source_is_rejected() {
        assert!(matches!(
            crop_edges(SCREEN, VIEW, (u32::MAX, u32::MAX)),
            Err(EncodeError::Crop { .. })
        ));
    }

    #[test]
    fn qp_zero_is_rejected() {
        assert!(matches!(checked_qp(0), Err(EncodeError::Qp(0))));
    }

    #[test]
    fn qp_one_and_fifty_one_are_the_bounds() {
        assert!(matches!((checked_qp(1), checked_qp(51)), (Ok(1), Ok(51))));
    }

    #[test]
    fn qp_fifty_two_is_rejected() {
        assert!(matches!(checked_qp(52), Err(EncodeError::Qp(52))));
    }
}
