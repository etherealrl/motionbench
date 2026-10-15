use super::*;
use crate::targets::vision::tests::{ORIGIN, frame_at, record, scenario, write_frames};

/// the trial's flick and its frames come back on one timeline: the counts run up as they were
/// sent, the click is where it was pressed, and each frame sits at its ms from the first report
#[test]
fn a_recording_plays_back_on_the_mouse_clock() {
    let folder = crate::testkit::traffic::TempDir::new("replay");
    std::fs::create_dir_all(folder.path()).unwrap();
    let path = folder.path().join("trial.pcap");
    let cfg = scenario();
    let moves: Vec<_> = (0..550)
        .map(|i| [if (300..500).contains(&i) { 5 } else { 0 }, 0])
        .collect();
    record(&path, &cfg, &moves, 500);
    write_frames(
        &path,
        &[
            frame_at(&cfg, 250, [1280.0, 720.0]),
            frame_at(&cfg, 400, [900.0, 720.0]),
        ],
    );
    let replay = replay(&path).unwrap();
    assert_eq!(replay.from_us, ORIGIN);
    assert_eq!(replay.mouse.len(), 550);
    assert_eq!(replay.mouse[299], [0, 0]);
    assert_eq!(replay.mouse[549], [1000, 0]);
    assert_eq!(replay.clicks, vec![500]);
    assert_eq!(
        replay.frames.iter().map(|f| f.at_ms).collect::<Vec<_>>(),
        vec![250.0, 400.0]
    );
    assert_eq!(
        replay.degrees,
        [cfg.sensitivity * cfg.yaw, cfg.sensitivity * cfg.pitch]
    );
    assert_eq!(replay.width, cfg.width);
}
