pub const IDR: u8 = 5;
pub const SPS: u8 = 7;
pub const PPS: u8 = 8;

const NAL_TYPE_MASK: u8 = 0x1F;

pub fn unit_types(annex_b: &[u8]) -> Vec<u8> {
    let mut types: Vec<u8> = Vec::new();
    let mut zeros: usize = 0;
    let mut bytes: std::iter::Peekable<std::slice::Iter<u8>> = annex_b.iter().peekable();
    while let Some(&byte) = bytes.next() {
        if byte == 1
            && zeros >= 2
            && let Some(&&header) = bytes.peek()
        {
            types.push(header & NAL_TYPE_MASK);
        }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
    }
    types
}

pub fn contains_idr(annex_b: &[u8]) -> bool {
    unit_types(annex_b).contains(&IDR)
}

#[cfg(test)]
mod tests {
    use super::{contains_idr, unit_types};

    #[test]
    fn no_start_code_has_no_units() {
        assert_eq!(unit_types(&[0x65, 0x00, 0x01, 0x67]), Vec::<u8>::new());
    }

    #[test]
    fn empty_input_has_no_units() {
        assert_eq!(unit_types(&[]), Vec::<u8>::new());
    }

    #[test]
    fn three_byte_start_code_yields_one_unit() {
        assert_eq!(unit_types(&[0, 0, 1, 0x65, 0x88]), vec![5]);
    }

    #[test]
    fn four_byte_start_code_yields_one_unit() {
        assert_eq!(unit_types(&[0, 0, 0, 1, 0x41, 0x9a]), vec![1]);
    }

    #[test]
    fn many_units_keep_their_order_and_drop_the_ref_idc() {
        let stream: [u8; 22] = [
            0, 0, 0, 1, 0x67, 0x64, 0x00, 0, 0, 0, 1, 0x68, 0xee, 0, 0, 1, 0x06, 0x05, 0, 0, 1,
            0x65,
        ];
        assert_eq!(unit_types(&stream), vec![7, 8, 6, 5]);
    }

    #[test]
    fn a_trailing_start_code_without_a_header_is_not_a_unit() {
        assert_eq!(unit_types(&[0, 0, 1, 0x41, 0, 0, 1]), vec![1]);
    }

    #[test]
    fn emulation_prevention_bytes_are_not_start_codes() {
        assert_eq!(unit_types(&[0, 0, 1, 0x41, 0, 0, 3, 1, 0x65]), vec![1]);
    }

    #[test]
    fn idr_is_found_after_parameter_sets() {
        assert!(contains_idr(&[0, 0, 1, 0x67, 0, 0, 1, 0x68, 0, 0, 1, 0x65]));
    }

    #[test]
    fn non_idr_slice_is_not_idr() {
        assert!(!contains_idr(&[0, 0, 0, 1, 0x41, 0x9a]));
    }

    #[test]
    fn idr_type_in_payload_without_start_code_is_not_idr() {
        assert!(!contains_idr(&[0, 0, 1, 0x41, 0x65, 0x65]));
    }
}
