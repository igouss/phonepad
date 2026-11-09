use anyhow::Context;
use jpeg_encoder::{ColorType, Encoder};

use crate::capture::{ChannelOrder, Frame};

const QUALITY: u8 = 60;
const BYTES_PER_PIXEL: usize = 4;

pub fn encode(frame: &Frame) -> anyhow::Result<Vec<u8>> {
    let width: u16 = u16::try_from(frame.width).context("frame too wide for JPEG")?;
    let height: u16 = u16::try_from(frame.height).context("frame too tall for JPEG")?;
    let color: ColorType = match frame.order {
        ChannelOrder::Bgra => ColorType::Bgra,
        ChannelOrder::Rgba => ColorType::Rgba,
    };
    let mut out: Vec<u8> = Vec::new();
    Encoder::new(&mut out, QUALITY).encode(&packed_rows(frame), width, height, color)?;
    Ok(out)
}

fn packed_rows(frame: &Frame) -> Vec<u8> {
    let row: usize = frame.width as usize * BYTES_PER_PIXEL;
    let mut rows: Vec<&[u8]> = frame
        .pixels
        .chunks_exact(frame.stride as usize)
        .take(frame.height as usize)
        .map(|line: &[u8]| &line[..row])
        .collect();
    if frame.y_invert {
        rows.reverse();
    }
    rows.concat()
}

#[cfg(test)]
mod tests {
    use phonepad_core::geometry::Point;

    use super::*;

    fn frame(height: u32, y_invert: bool, pixels: Vec<u8>) -> Frame {
        Frame {
            origin: Point { x: 0, y: 0 },
            width: 1,
            height,
            stride: 6,
            order: ChannelOrder::Bgra,
            y_invert,
            pixels,
        }
    }

    #[test]
    fn zero_rows_pack_to_nothing() {
        assert_eq!(packed_rows(&frame(0, false, vec![])), Vec::<u8>::new());
    }

    #[test]
    fn one_row_drops_stride_padding() {
        assert_eq!(
            packed_rows(&frame(1, false, vec![1, 2, 3, 4, 0, 0])),
            vec![1, 2, 3, 4]
        );
    }

    #[test]
    fn many_rows_keep_order() {
        assert_eq!(
            packed_rows(&frame(2, false, vec![1, 2, 3, 4, 0, 0, 5, 6, 7, 8, 0, 0])),
            vec![1, 2, 3, 4, 5, 6, 7, 8]
        );
    }

    #[test]
    fn y_invert_flips_rows_not_pixels() {
        assert_eq!(
            packed_rows(&frame(2, true, vec![1, 2, 3, 4, 0, 0, 5, 6, 7, 8, 0, 0])),
            vec![5, 6, 7, 8, 1, 2, 3, 4]
        );
    }

    #[test]
    fn encodes_a_jpeg() {
        let jpeg: Vec<u8> = encode(&frame(2, false, vec![9; 12])).unwrap();
        assert_eq!(jpeg[..2], [0xFF, 0xD8]);
    }
}
