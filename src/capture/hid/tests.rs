use super::*;
#[test]
fn left_click_uses_button_usage_and_report_id() {
    let descriptor = [
        0x05, 9, 0x85, 3, 0x15, 0, 0x25, 1, 0x75, 1, 0x95, 3, 0x19, 1, 0x29, 3, 0x81, 2, 0x75, 5,
        0x95, 1, 0x81, 1, 0x05, 1, 0x15, 0x80, 0x75, 8, 0x95, 2, 0x09, 0x30, 0x09, 0x31, 0x81, 6,
    ];
    let layout = Layout::parse(&descriptor).unwrap();
    assert_eq!(layout.left_button(&[3, 1, 0, 0]), Some(true));
    assert_eq!(layout.left_button(&[3, 2, 0, 0]), Some(false));
    assert_eq!(layout.left_button(&[4, 1, 0, 0]), None);
    assert_eq!(layout.left_button(&[3]), None);
}
#[test]
fn report_id_and_signed_non_byte_aligned_axes() {
    let bytes = [
        0x05, 1, 0x85, 7, 0x15, 0x80, 0x75, 3, 0x95, 1, 0x81, 1, 0x75, 8, 0x95, 2, 0x09, 0x30,
        0x09, 0x31, 0x81, 6,
    ];
    let layout = Layout::parse(&bytes).unwrap();
    let packed = ((-3i8 as u8 as u32) << 3) | (5 << 11);
    let p = packed.to_le_bytes();
    assert_eq!(layout.decode(&[7, p[0], p[1], p[2]]), Some((-3, 5)));
    assert_eq!(layout.decode(&[8, 0, 0, 0]), None);
    assert_eq!(layout.decode(&[7, 0]), None);
}
#[test]
fn unsupported_and_truncated_descriptors_fail() {
    assert!(Layout::parse(&[0x75]).is_err());
    assert!(Layout::parse(&[0x05, 1]).is_err());
}
