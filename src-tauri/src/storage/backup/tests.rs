use super::*;

#[test]
fn copies_are_byte_identical_and_reused_for_the_same_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.json");
    let first = preserve(&path, b"broken\0\xff").unwrap();
    assert_eq!(first, dir.path().join("library.corrupt-1.json"));
    assert_eq!(std::fs::read(&first).unwrap(), b"broken\0\xff");
    assert_eq!(preserve(&path, b"broken\0\xff").unwrap(), first);
    // same length, different bytes, so a slot is only reused for an exact match
    let second = preserve(&path, b"broken\0\xfe").unwrap();
    assert_eq!(second, dir.path().join("library.corrupt-2.json"));
    assert_eq!(std::fs::read(&first).unwrap(), b"broken\0\xff");
    assert_eq!(std::fs::read(second).unwrap(), b"broken\0\xfe");
}

#[test]
fn callers_racing_for_one_slot_both_get_a_verified_copy() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.json");
    let copies: Vec<_> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..8)
            .map(|_| scope.spawn(|| preserve(&path, b"broken")))
            .collect();
        workers.into_iter().map(|w| w.join().unwrap()).collect()
    });
    for copy in copies {
        assert_eq!(std::fs::read(copy.unwrap()).unwrap(), b"broken");
    }
}

#[test]
fn an_unreadable_copy_slot_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("config.corrupt-1.json")).unwrap();
    assert!(preserve(&dir.path().join("config.json"), b"broken").is_err());
}
