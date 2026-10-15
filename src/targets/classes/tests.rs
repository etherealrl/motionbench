use super::*;

fn varint(mut value: u64, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn bytes(field: u64, value: &[u8], out: &mut Vec<u8>) {
    varint(field << 3 | 2, out);
    varint(value.len() as u64, out);
    out.extend_from_slice(value);
}

fn entry(key: &str, value: &str) -> Vec<u8> {
    let mut out = Vec::new();
    bytes(KEY, key.as_bytes(), &mut out);
    bytes(VALUE, value.as_bytes(), &mut out);
    out
}

/// an onnx model as an exporter writes it: a version, a large graph, then the metadata
fn model(metadata: &[(&str, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    varint(1 << 3, &mut out);
    varint(9, &mut out);
    bytes(7, &vec![0xab; 300_000], &mut out);
    for (key, value) in metadata {
        bytes(METADATA_PROPS, &entry(key, value), &mut out);
    }
    out
}

#[test]
fn the_names_come_from_the_metadata_in_index_order() {
    let file = model(&[
        ("author", "someone"),
        ("names", "{0: 'person', 1: \"cat's toy\", 2: 'head'}"),
    ]);
    assert_eq!(
        class_names(&mut io::Cursor::new(file)).unwrap(),
        ["person", "cat's toy", "head"]
    );
}

#[test]
fn a_model_without_names_has_none() {
    let file = model(&[("author", "someone")]);
    assert!(class_names(&mut io::Cursor::new(file)).unwrap().is_empty());
}

/// names that skip an index would put a name on the wrong class, so there are none
#[test]
fn names_with_a_gap_give_none() {
    assert!(parse("{0: 'a', 2: 'c'}").is_empty());
    assert!(parse("{0: a}").is_empty());
    assert_eq!(parse("{}"), Vec::<String>::new());
    assert_eq!(parse("{0: 'a'}"), ["a"]);
}

#[test]
fn a_file_that_is_no_model_is_refused() {
    assert!(class_names(&mut io::Cursor::new(vec![0x0f, 0xff, 0xff])).is_err());
}

/// a broken file's lengths can't be trusted, it says so rather than allocating or looping forever
#[test]
fn a_length_past_the_end_of_the_file_is_refused() {
    for field in [7, METADATA_PROPS] {
        for length in [1_000_000, u64::MAX - 3] {
            let mut file = Vec::new();
            varint(field << 3 | 2, &mut file);
            varint(length, &mut file);
            file.extend_from_slice(&[0; 16]);
            assert!(class_names(&mut io::Cursor::new(file)).is_err());
        }
    }
}
