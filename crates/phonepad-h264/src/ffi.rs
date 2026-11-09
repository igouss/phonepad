use ffmpeg_next::format::Pixel;
use ffmpeg_next::{Error, Rational, codec, encoder, filter, frame};
use ffmpeg_sys_next::{
    AV_HWFRAME_MAP_READ, AV_HWFRAME_MAP_WRITE, AVBufferRef, AVBufferSrcParameters,
    AVDRMFrameDescriptor, AVFilterContext, AVFrame, AVHWDeviceType, AVHWFramesContext,
    AVPixelFormat, av_buffer_ref, av_buffer_unref, av_buffersink_get_hw_frames_ctx,
    av_buffersrc_add_frame_flags, av_buffersrc_parameters_alloc, av_buffersrc_parameters_set,
    av_frame_clone, av_frame_free, av_free, av_hwdevice_ctx_create, av_hwframe_ctx_alloc,
    av_hwframe_ctx_init, av_hwframe_get_buffer, av_hwframe_map, av_hwframe_transfer_data,
    avfilter_graph_alloc_filter, avfilter_init_dict,
};
use std::ffi::{CStr, c_int};
use std::os::fd::{BorrowedFd, OwnedFd, RawFd};
use std::ptr::null_mut;

pub struct DrmPlane {
    pub fd: OwnedFd,
    pub offset: u32,
    pub pitch: u32,
    pub modifier: u64,
    pub fourcc: u32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MapError {
    #[error("av_hwframe_map: {0}")]
    Map(Error),
    #[error("expected 1 object, 1 layer and 1 plane, got {objects}, {layers} and {planes}")]
    Layout {
        objects: c_int,
        layers: c_int,
        planes: c_int,
    },
    #[error("negative offset or pitch in the DRM descriptor")]
    Geometry,
    #[error("dup of the dmabuf fd: {0}")]
    Dup(std::io::ErrorKind),
}

fn status(code: c_int) -> Result<(), Error> {
    match code {
        0.. => Ok(()),
        negative => Err(Error::from(negative)),
    }
}

fn out_of_memory() -> Error {
    Error::from(ffmpeg_sys_next::AVERROR(ffmpeg_sys_next::ENOMEM))
}

pub struct HwFrames {
    frames: *mut AVBufferRef,
}

unsafe impl Send for HwFrames {}

impl Drop for HwFrames {
    fn drop(&mut self) {
        unsafe { av_buffer_unref(&mut self.frames) }
    }
}

struct HwDevice {
    device: *mut AVBufferRef,
}

impl Drop for HwDevice {
    fn drop(&mut self) {
        unsafe { av_buffer_unref(&mut self.device) }
    }
}

fn vaapi_device(render_node: &CStr) -> Result<HwDevice, Error> {
    let mut options: ffmpeg_next::Dictionary = ffmpeg_next::Dictionary::new();
    options.set("connection_type", "drm");
    let mut device: *mut AVBufferRef = null_mut();
    let code: c_int = unsafe {
        av_hwdevice_ctx_create(
            &mut device,
            AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
            render_node.as_ptr(),
            options.as_mut_ptr(),
            0,
        )
    };
    status(code).map(|()| HwDevice { device })
}

impl HwFrames {
    pub fn vaapi(
        render_node: &CStr,
        sw_format: Pixel,
        width: c_int,
        height: c_int,
    ) -> Result<HwFrames, Error> {
        let device: HwDevice = vaapi_device(render_node)?;
        let allocated: *mut AVBufferRef = unsafe { av_hwframe_ctx_alloc(device.device) };
        if allocated.is_null() {
            return Err(out_of_memory());
        }
        let frames: HwFrames = HwFrames { frames: allocated };
        unsafe {
            let context: &mut AVHWFramesContext =
                &mut *((*frames.frames).data as *mut AVHWFramesContext);
            context.format = AVPixelFormat::AV_PIX_FMT_VAAPI;
            context.sw_format = sw_format.into();
            context.width = width;
            context.height = height;
            context.initial_pool_size = 0;
        }
        status(unsafe { av_hwframe_ctx_init(frames.frames) })?;
        Ok(frames)
    }

    pub fn surface(&self) -> Result<frame::Video, Error> {
        let mut surface: frame::Video = frame::Video::empty();
        status(unsafe { av_hwframe_get_buffer(self.frames, surface.as_mut_ptr(), 0) })?;
        Ok(surface)
    }

    pub fn feed(
        &self,
        graph: &mut filter::Graph,
        name: &CStr,
        time_base: Rational,
    ) -> Result<(), Error> {
        let buffer: filter::Filter = filter::find("buffer").ok_or(Error::FilterNotFound)?;
        unsafe {
            let source: *mut AVFilterContext =
                avfilter_graph_alloc_filter(graph.as_mut_ptr(), buffer.as_ptr(), name.as_ptr());
            if source.is_null() {
                return Err(out_of_memory());
            }
            let parameters: *mut AVBufferSrcParameters = av_buffersrc_parameters_alloc();
            if parameters.is_null() {
                return Err(out_of_memory());
            }
            let context: &AVHWFramesContext = &*((*self.frames).data as *const AVHWFramesContext);
            (*parameters).format = AVPixelFormat::AV_PIX_FMT_VAAPI as c_int;
            (*parameters).width = context.width;
            (*parameters).height = context.height;
            (*parameters).time_base = time_base.into();
            (*parameters).hw_frames_ctx = self.frames;
            let set: c_int = av_buffersrc_parameters_set(source, parameters);
            av_free(parameters.cast());
            status(set)?;
            status(avfilter_init_dict(source, null_mut()))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crop {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

pub fn add_cropped(
    source: &mut filter::Context,
    surface: &frame::Video,
    crop: Crop,
) -> Result<(), Error> {
    let mut reference: *mut AVFrame = unsafe { av_frame_clone(surface.as_ptr()) };
    if reference.is_null() {
        return Err(out_of_memory());
    }
    let added: c_int = unsafe {
        (*reference).crop_left = crop.left as usize;
        (*reference).crop_top = crop.top as usize;
        (*reference).crop_right = crop.right as usize;
        (*reference).crop_bottom = crop.bottom as usize;
        let added: c_int = av_buffersrc_add_frame_flags(source.as_mut_ptr(), reference, 0);
        av_frame_free(&mut reference);
        added
    };
    status(added)
}

pub fn adopt_sink_frames(
    context: &mut encoder::video::Video,
    sink: &mut filter::Context,
) -> Result<(), Error> {
    unsafe {
        let frames: *mut AVBufferRef = av_buffersink_get_hw_frames_ctx(sink.as_mut_ptr());
        if frames.is_null() {
            return Err(Error::InvalidData);
        }
        let reference: *mut AVBufferRef = av_buffer_ref(frames);
        if reference.is_null() {
            return Err(out_of_memory());
        }
        (*context.as_mut_ptr()).hw_frames_ctx = reference;
    }
    Ok(())
}

pub fn encoder_context(codec: ffmpeg_next::Codec) -> Result<encoder::video::Video, Error> {
    codec::context::Context::new_with_codec(codec)
        .encoder()
        .video()
}

pub fn upload(surface: &mut frame::Video, pixels: &frame::Video) -> Result<(), Error> {
    status(unsafe { av_hwframe_transfer_data(surface.as_mut_ptr(), pixels.as_ptr(), 0) })
}

pub fn export_drm(surface: &frame::Video) -> Result<DrmPlane, MapError> {
    let mut mapped: frame::Video = frame::Video::empty();
    mapped.set_format(Pixel::DRM_PRIME);
    status(unsafe {
        av_hwframe_map(
            mapped.as_mut_ptr(),
            surface.as_ptr(),
            AV_HWFRAME_MAP_READ as c_int | AV_HWFRAME_MAP_WRITE as c_int,
        )
    })
    .map_err(MapError::Map)?;
    let descriptor: &AVDRMFrameDescriptor =
        unsafe { &*((*mapped.as_ptr()).data[0] as *const AVDRMFrameDescriptor) };
    let layer_planes: c_int = descriptor.layers[0].nb_planes;
    let plane: &ffmpeg_sys_next::AVDRMPlaneDescriptor = &descriptor.layers[0].planes[0];
    if descriptor.nb_objects != 1
        || descriptor.nb_layers != 1
        || layer_planes != 1
        || plane.object_index != 0
    {
        return Err(MapError::Layout {
            objects: descriptor.nb_objects,
            layers: descriptor.nb_layers,
            planes: layer_planes,
        });
    }
    let object: &ffmpeg_sys_next::AVDRMObjectDescriptor = &descriptor.objects[0];
    let raw: RawFd = object.fd;
    let fd: OwnedFd = unsafe { BorrowedFd::borrow_raw(raw) }
        .try_clone_to_owned()
        .map_err(|e| MapError::Dup(e.kind()))?;
    Ok(DrmPlane {
        fd,
        offset: u32::try_from(plane.offset).map_err(|_| MapError::Geometry)?,
        pitch: u32::try_from(plane.pitch).map_err(|_| MapError::Geometry)?,
        modifier: object.format_modifier,
        fourcc: descriptor.layers[0].format,
    })
}
