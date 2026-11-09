use wayland_client::protocol::wl_shm;

use super::ChannelOrder;

const SUPPORTED: [(wl_shm::Format, ChannelOrder); 4] = [
    (wl_shm::Format::Argb8888, ChannelOrder::Bgra),
    (wl_shm::Format::Xrgb8888, ChannelOrder::Bgra),
    (wl_shm::Format::Abgr8888, ChannelOrder::Rgba),
    (wl_shm::Format::Xbgr8888, ChannelOrder::Rgba),
];

pub(super) fn channel_order(format: wl_shm::Format) -> Option<ChannelOrder> {
    SUPPORTED
        .iter()
        .find(|(supported, _): &&(wl_shm::Format, ChannelOrder)| *supported == format)
        .map(|(_, order): &(wl_shm::Format, ChannelOrder)| *order)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xrgb_is_bgra_in_memory() {
        assert_eq!(
            channel_order(wl_shm::Format::Xrgb8888),
            Some(ChannelOrder::Bgra)
        );
    }

    #[test]
    fn xbgr_is_rgba_in_memory() {
        assert_eq!(
            channel_order(wl_shm::Format::Xbgr8888),
            Some(ChannelOrder::Rgba)
        );
    }

    #[test]
    fn packed_formats_are_unsupported() {
        assert_eq!(channel_order(wl_shm::Format::Rgb565), None);
    }
}
