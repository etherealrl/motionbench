use super::*;
use crate::testkit::traffic::TempDir;

#[test]
fn only_a_capture_with_its_metadata_or_a_marker_beside_it_is_a_recording() {
    let folder = TempDir::new("layout");
    std::fs::create_dir_all(folder.path()).unwrap();
    let pcap = folder.path().join("run.pcap");
    std::fs::write(&pcap, b"").unwrap();
    assert!(
        locate(&pcap)
            .err()
            .unwrap()
            .contains("not recorded by Motion Bench")
    );
    for beside in ["json", "incomplete.json", "active.json"] {
        let marker = pcap.with_extension(beside);
        std::fs::write(&marker, b"{}").unwrap();
        let located = locate(&pcap).unwrap();
        assert_eq!(located.sidecar, pcap.with_extension("json"));
        assert_eq!(located.input, pcap.with_extension("input.csv"));
        std::fs::remove_file(marker).unwrap();
    }
    assert!(
        locate(folder.path())
            .err()
            .unwrap()
            .contains("not a folder")
    );
}
