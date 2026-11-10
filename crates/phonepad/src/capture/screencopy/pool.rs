use std::os::fd::AsFd;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, bail};
use phonepad_h264::{CaptureSurface, Cropper, Dmabuf, H264};
use wayland_client::QueueHandle;
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_output::WlOutput;
use wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::ExtImageCaptureSourceV1;
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_frame_v1::ExtImageCopyCaptureFrameV1;
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_manager_v1::{
    self, ExtImageCopyCaptureManagerV1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_session_v1::ExtImageCopyCaptureSessionV1;
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1::{
    self, ZwpLinuxBufferParamsV1,
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1;

use super::super::lane::Lane;
use super::super::plan::{Constraints, Pixels, Shape, Shown, Slot};
use super::{NamedOutput, State};

const POOL: usize = 4;
const IN_FLIGHT: usize = 2;
const QP: u8 = 24;
const LINEAR: u64 = 0;

pub(super) struct Surface {
    pub(super) surface: Arc<CaptureSurface>,
    pub(super) buffer: WlBuffer,
}

pub(super) struct Session {
    pub(super) proxy: ExtImageCopyCaptureSessionV1,
    pub(super) constraints: Constraints,
    pub(super) ready: bool,
}

pub(super) struct Pool {
    pub(super) shape: Shape,
    pub(super) cropper: Cropper,
    pub(super) lane: Lane,
    pub(super) source: ExtImageCaptureSourceV1,
    pub(super) sessions: Vec<Session>,
    pub(super) surfaces: Vec<Surface>,
    pub(super) slots: Vec<Slot<ExtImageCopyCaptureFrameV1>>,
    pub(super) presented: Vec<u64>,
    pub(super) changed: bool,
    pub(super) shown: Option<Shown>,
    pub(super) encoded_at: Option<Instant>,
    pub(super) keyframe_at: Option<Instant>,
}

fn source_size(pixels: Pixels) -> phonepad_h264::Size {
    phonepad_h264::Size {
        width: pixels.width,
        height: pixels.height,
    }
}

impl Pool {
    pub(super) fn open(
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
            changed: false,
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
