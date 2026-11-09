use crate::{EncodeError, ffi};

const MAX_QP: u8 = 51;

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

fn sides(size: Size) -> Option<(i32, i32)> {
    match (i32::try_from(size.width), i32::try_from(size.height)) {
        (Ok(width), Ok(height)) if width > 0 && height > 0 => Some((width, height)),
        (Ok(_) | Err(_), Ok(_) | Err(_)) => None,
    }
}

pub(crate) fn checked_geometry(source: Size, output: Size) -> Result<(i32, i32), EncodeError> {
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

pub(crate) fn crop_edges(
    source: Size,
    output: Size,
    origin: (u32, u32),
) -> Result<ffi::Crop, EncodeError> {
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

pub(crate) fn checked_qp(qp: u8) -> Result<u8, EncodeError> {
    match qp {
        1..=MAX_QP => Ok(qp),
        other => Err(EncodeError::Qp(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::{Size, checked_geometry, checked_qp, crop_edges};
    use crate::{EncodeError, ffi};

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
