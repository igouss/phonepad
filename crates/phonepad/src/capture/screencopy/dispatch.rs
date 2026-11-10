use wayland_client::protocol::wl_buffer::{self, WlBuffer};
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::{
    self, ExtImageCaptureSourceV1,
};
use wayland_protocols::ext::image_capture_source::v1::client::ext_output_image_capture_source_manager_v1::{
    self, ExtOutputImageCaptureSourceManagerV1,
};
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
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::{
    self, ZwpLinuxDmabufV1,
};

use super::{NamedOutput, State};

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
                "ext_output_image_capture_source_manager_v1" => {
                    state.sources = Some(registry.bind(name, version.min(1), qh, ()))
                }
                "ext_image_copy_capture_manager_v1" => {
                    state.manager = Some(registry.bind(name, version.min(1), qh, ()))
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

impl Dispatch<ExtImageCopyCaptureFrameV1, usize> for State {
    fn event(
        state: &mut State,
        frame: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        state.frame_event(*index, frame, event);
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, usize> for State {
    fn event(
        state: &mut State,
        _: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        state.session_event(*index, event);
    }
}

impl Dispatch<ExtImageCaptureSourceV1, ()> for State {
    fn event(
        _: &mut State,
        _: &ExtImageCaptureSourceV1,
        _: ext_image_capture_source_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}

impl Dispatch<ExtOutputImageCaptureSourceManagerV1, ()> for State {
    fn event(
        _: &mut State,
        _: &ExtOutputImageCaptureSourceManagerV1,
        _: ext_output_image_capture_source_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
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

impl Dispatch<ExtImageCopyCaptureManagerV1, ()> for State {
    fn event(
        _: &mut State,
        _: &ExtImageCopyCaptureManagerV1,
        _: ext_image_copy_capture_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}
