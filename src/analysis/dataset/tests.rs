use super::*;
#[test]
fn malformed_and_pickle_arrays_are_refused() {
    assert!(read_array(b"bad").is_err());
    let header = "{'descr': '|O8', 'fortran_order': False, 'shape': (1,), }";
    let mut bytes = b"\x93NUMPY\x01\x00".to_vec();
    bytes.extend((header.len() as u16).to_le_bytes());
    bytes.extend(header.as_bytes());
    bytes.extend([0; 8]);
    assert!(read_array(&bytes).unwrap_err().contains("pickle"));
}

/// a dtype with a character wider than a byte is refused like any other bad one, not a panic
/// that takes the whole library listing with it
#[test]
fn a_multibyte_dtype_is_refused_without_a_panic() {
    for descr in ["'é8'", "é8é", "'<é8'"] {
        let header = format!("{{'descr': {descr}, 'fortran_order': False, 'shape': (1,), }}");
        let mut bytes = b"\x93NUMPY\x01\x00".to_vec();
        bytes.extend((header.len() as u16).to_le_bytes());
        bytes.extend(header.as_bytes());
        bytes.extend([0; 8]);
        assert!(read_array(&bytes).is_err());
    }
}
