use super::*;
use std::path::Path;
/// the projection these tests were worked out with: CS2, whose 4:3 view stretched to 16:9 shows
/// 106.26 degrees across
fn cs2() -> Config {
    Config {
        horizontal_fov: 106.2602047083,
        yaw: 0.022,
        pitch: 0.022,
        ..Config::default()
    }
}
#[test]
fn frame_limits_support_unlimited_and_high_refresh_rates() {
    let mut config = Config::default();
    assert!(config.validate().is_ok());
    assert_eq!(config.frame_interval(), None);
    for fps in [1, 240, 500, 1000, 8000, u32::MAX] {
        config.fps = fps;
        assert!(config.validate().is_ok());
        let interval = config.frame_interval().unwrap();
        assert!((interval.as_secs_f64() - 1.0 / fps as f64).abs() <= 1e-9);
    }
}
#[test]
fn cs2_projection_and_off_center_radius_use_edges() {
    let cfg = cs2();
    assert_eq!(cfg.counts(1280.0, 720.0), [0.0, 0.0]);
    let p = cfg.counts(2560.0, 720.0);
    assert!((p[0] - cfg.horizontal_fov / 2.0 / 0.0352).abs() < 1e-8);
    let (_, radius) = cfg.geometry(&Detection {
        bounds: [2000.0, 680.0, 2080.0, 760.0],
        confidence: 1.0,
        class: 0,
    });
    assert!(radius > 0.0);
    assert!(
        radius
            < cfg
                .geometry(&Detection {
                    bounds: [1240.0, 680.0, 1320.0, 760.0],
                    confidence: 1.0,
                    class: 0
                })
                .1
    );
}
/// a ball 30 counts across in angle, 42 degrees right and 25 above a camera looking 10 degrees up:
/// its box is stretched far out of square there, the angle from its center to the nearest edge
/// is still the ball's
#[test]
fn an_off_center_target_above_the_camera_has_its_true_angular_radius() {
    let cfg = cs2();
    let k = cfg.sensitivity * cfg.yaw;
    let (pitch, radius) = (10.0, 30.0);
    let center = toward(42.0, pitch + 25.0);
    // two directions square to the center, to walk the ball's rim
    let side = unit([center[2], 0.0, -center[0]]);
    let other = [
        center[1] * side[2] - center[2] * side[1],
        center[2] * side[0] - center[0] * side[2],
        center[0] * side[1] - center[1] * side[0],
    ];
    let (cos, sin) = (
        (radius * k).to_radians().cos(),
        (radius * k).to_radians().sin(),
    );
    let mut bounds = [f64::INFINITY, f64::INFINITY, 0.0, 0.0];
    for step in 0..3600 {
        let turn = (step as f64 / 10.0).to_radians();
        let rim = [0, 1, 2]
            .map(|a| cos * center[a] + sin * (turn.cos() * side[a] + turn.sin() * other[a]));
        let [x, y] = seen(&cfg, rim, [0.0; 2], pitch);
        bounds = [
            bounds[0].min(x),
            bounds[1].min(y),
            bounds[2].max(x),
            bounds[3].max(y),
        ];
    }
    let (_, found) = cfg.geometry(&Detection {
        bounds,
        confidence: 1.0,
        class: 0,
    });
    assert!((found - radius).abs() < 0.05, "{found}");
}
#[test]
fn yolo_suppresses_duplicates_and_rejects_wrong_layouts() {
    let values = vec![320.0, 321.0, 320.0, 321.0, 40.0, 40.0, 40.0, 40.0, 0.9, 0.8];
    assert_eq!(yolo8(&values, 5, 2, &Config::default()).unwrap().len(), 1);
    assert!(yolo8(&values, 4, 2, &Config::default()).is_err());
}
pub(crate) const ORIGIN: u64 = 1_790_000_000_000_000;
pub(crate) fn scenario() -> Config {
    Config {
        scenario: "synthetic single target".into(),
        ..cs2()
    }
}
fn focal(cfg: &Config) -> f64 {
    cfg.width as f64 / (2.0 * (cfg.horizontal_fov.to_radians() / 2.0).tan())
}
/// a direction in the world from its yaw and its pitch above level, in degrees
fn toward(yaw: f64, pitch: f64) -> [f64; 3] {
    let (yaw, pitch) = (yaw.to_radians(), pitch.to_radians());
    [
        pitch.cos() * yaw.sin(),
        pitch.sin(),
        pitch.cos() * yaw.cos(),
    ]
}
/// where a target in `direction` shows on screen, in pixels, for a camera turned by `mouse`
/// counts from `pitch` degrees; worked out apart from `Camera`, so a mistake in one does not hide
/// in the other
fn seen(cfg: &Config, direction: [f64; 3], mouse: [f64; 2], pitch: f64) -> [f64; 2] {
    let k = [cfg.sensitivity * cfg.yaw, cfg.sensitivity * cfg.pitch];
    let yaw = (mouse[0] * k[0]).to_radians();
    let q = (pitch - mouse[1] * k[1]).to_radians();
    let dy = direction[0].atan2(direction[2]) - yaw;
    let p = direction[1].asin();
    // the camera's own axes: right, up, forward
    let x = p.cos() * dy.sin();
    let y = p.sin() * q.cos() - p.cos() * q.sin() * dy.cos();
    let z = p.sin() * q.sin() + p.cos() * q.cos() * dy.cos();
    let f = focal(cfg);
    [
        cfg.width as f64 / 2.0 + f * x / z,
        cfg.height as f64 / 2.0 - f * y / z,
    ]
}
/// a target seen at tick `i`, its box about 40 counts across at the middle of the screen
pub(crate) fn frame_at(cfg: &Config, i: usize, center: [f64; 2]) -> Frame {
    let half = (20.0 * cfg.sensitivity * cfg.yaw).to_radians().tan() * focal(cfg);
    crate::testkit::fixtures::frame(
        ORIGIN + i as u64 * 1000,
        vec![Detection {
            bounds: [
                center[0] - half,
                center[1] - half,
                center[0] + half,
                center[1] + half,
            ],
            confidence: 1.0,
            class: 0,
        }],
    )
}
/// a finished recording of one report a millisecond moving by `moves`, the left button pressed
/// at report `click`
pub(crate) fn record(path: &Path, cfg: &Config, moves: &[[i16; 2]], click: usize) {
    record_at(path, cfg, moves, &[click], |i| ORIGIN + i as u64 * 1000)
}
/// `record` with each report's time given by `at`, and any number of clicks
fn record_at(
    path: &Path,
    cfg: &Config,
    moves: &[[i16; 2]],
    clicks: &[usize],
    at: impl Fn(usize) -> u64,
) {
    let mut bytes = Vec::new();
    bytes.extend(0xa1b2c3d4u32.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(4u16.to_le_bytes());
    bytes.extend([0u8; 8]);
    bytes.extend(65535u32.to_le_bytes());
    bytes.extend(249u32.to_le_bytes());
    for (i, [dx, dy]) in moves.iter().enumerate() {
        let mut packet = vec![0u8; 27];
        packet[..2].copy_from_slice(&27u16.to_le_bytes());
        packet[16] = 1;
        packet[17..19].copy_from_slice(&1u16.to_le_bytes());
        packet[19..21].copy_from_slice(&31u16.to_le_bytes());
        packet[21] = 129;
        packet[22] = 1;
        packet[23..27].copy_from_slice(&5u32.to_le_bytes());
        packet.push(u8::from(clicks.contains(&i)));
        packet.extend(dx.to_le_bytes());
        packet.extend(dy.to_le_bytes());
        let t = at(i);
        bytes.extend(((t / 1_000_000) as u32).to_le_bytes());
        bytes.extend(((t % 1_000_000) as u32).to_le_bytes());
        bytes.extend((packet.len() as u32).to_le_bytes());
        bytes.extend((packet.len() as u32).to_le_bytes());
        bytes.extend(packet);
    }
    std::fs::write(path, bytes).unwrap();
    let mut metadata = crate::testkit::fixtures::sidecar("complete");
    metadata["selection"] = serde_json::json!({"bus": 1, "address": 31, "endpoint": 129});
    std::fs::write(path.with_extension("json"), metadata.to_string()).unwrap();
    let evidence = Evidence {
        schema: "motion-bench.vision.v1".into(),
        config: cfg.clone(),
        model_sha256: "synthetic".into(),
        provider: "synthetic".into(),
        pipeline: "synthetic".into(),
        inference: None,
        complete: true,
        error: None,
        clock: Some(crate::testkit::fixtures::screen_clock()),
    };
    std::fs::write(
        path.with_extension("vision.json"),
        serde_json::to_vec(&evidence).unwrap(),
    )
    .unwrap();
}
pub(crate) fn write_frames(path: &Path, frames: &[Frame]) {
    std::fs::write(
        path.with_extension("targets.jsonl"),
        frames
            .iter()
            .map(|f| serde_json::to_string(f).unwrap() + "\n")
            .collect::<String>(),
    )
    .unwrap()
}
fn summary(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path.with_extension("events.json")).unwrap()).unwrap()
}
#[test]
fn synchronized_wire_click_trial_exports_full49_and_excludes_bad_geometry() {
    let folder = crate::testkit::traffic::TempDir::new("events");
    std::fs::create_dir_all(folder.path()).unwrap();
    let path = folder.path().join("trial.pcap");
    let cfg = scenario();
    let moves: Vec<_> = (0..550)
        .map(|i| [if (300..500).contains(&i) { 5 } else { 0 }, 0])
        .collect();
    record(&path, &cfg, &moves, 500);
    // a level target 1000 counts to the right of where the camera starts
    let target = toward(1000.0 * cfg.sensitivity * cfg.yaw, 0.0);
    let mouse = |i: usize| [moves[..i].iter().map(|m| m[0] as f64).sum(), 0.0];
    let mut frames: Vec<_> = (250..=510)
        .step_by(10)
        .map(|i| frame_at(&cfg, i, seen(&cfg, target, mouse(i), 0.0)))
        .collect();
    write_frames(&path, &frames);
    assert_eq!(build_events(&path).unwrap(), 1);
    let fitted = summary(&path)["starting_pitch_degrees"].as_f64().unwrap();
    // a level start stays exactly level
    assert_eq!(fitted, 0.0);
    let dataset =
        crate::analysis::dataset::Dataset::load(&path.with_extension("events.npz")).unwrap();
    assert_eq!(dataset.prefix.as_ref().unwrap()[0].len(), 256);
    assert_eq!(dataset.rows("full49").unwrap().1[0].len(), 49);
    assert_eq!(dataset.groups.as_ref().unwrap().len(), 1);
    // the event spans its prefix and movement up to the click, on the pcap clock
    let [from, to] = dataset.spans.as_ref().unwrap()[0];
    assert_eq!(to, ORIGIN + 500_000);
    assert_eq!(
        to - from,
        256_000 + dataset.future[0].len() as u64 * 1000 - 1000
    );
    let crop = |to_us| crate::analysis::compare::Selection {
        from_us: ORIGIN,
        to_us,
    };
    let mut kept =
        crate::analysis::dataset::Dataset::load(&path.with_extension("events.npz")).unwrap();
    kept.within(&crop(to)).unwrap();
    assert_eq!(kept.rows("full49").unwrap().1.len(), 1);
    // a crop that ends before the click leaves the event out, prefix, targets and groups too
    let mut dropped =
        crate::analysis::dataset::Dataset::load(&path.with_extension("events.npz")).unwrap();
    dropped.within(&crop(to - 1)).unwrap();
    assert!(dropped.rows("full49").unwrap().1.is_empty());
    assert!(dropped.groups.unwrap().is_empty());
    // a comparison of the events, matched by their source and cut by a crop
    use crate::analysis::compare::{Source, independent_selected, matching_sources};
    let source = |to_us| Source {
        path: path.clone(),
        decoder: None,
        endpoint: None,
        crop: Some(crop(to_us)),
    };
    let compare = |to_us| {
        let (base, candidate) = matching_sources(&[source(to_us)], &[source(to_us)])?;
        independent_selected(
            &base,
            &candidate,
            "full49",
            None,
            crate::analysis::classify::Config {
                bootstrap: 0,
                permutations: 0,
                ..Default::default()
            },
            &std::sync::atomic::AtomicBool::new(false),
            |_| {},
        )
        .map_err(|e| e.to_string())
    };
    let compared = compare(to).unwrap();
    assert_eq!(compared.real_features.len(), 1);
    assert_eq!(
        compared.crops[&crate::evidence::reader::layout::recording_id(&path)],
        Some(crop(to))
    );
    // the crop leaves no event on either side
    assert!(compare(to - 1).is_err());
    frames[10].detections[0].bounds[0] += 150.0;
    frames[10].detections[0].bounds[2] += 150.0;
    write_frames(&path, &frames);
    assert_eq!(build_events(&path).unwrap(), 0);
    assert!(!path.with_extension("events.npz").exists());
    let summary = summary(&path);
    assert_eq!(summary["accepted"], 0);
    assert!(
        summary["events"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("residual")
    );
}

/// a target 25 degrees above the camera and a flick of 42 degrees across to it: read an axis at a
/// time, its place on screen puts it hundreds of counts off its true height, far more than its size
#[test]
fn a_target_above_the_camera_keeps_its_event_and_b80() {
    let folder = crate::testkit::traffic::TempDir::new("events-above");
    std::fs::create_dir_all(folder.path()).unwrap();
    let path = folder.path().join("trial.pcap");
    let cfg = scenario();
    let k = cfg.sensitivity * cfg.yaw;
    // the camera starts 10 degrees up, the mouse turns it 1200 counts right and 720 up
    let pitch = 10.0;
    let moves: Vec<_> = (0..600)
        .map(|i| {
            if (300..540).contains(&i) {
                [5, -3]
            } else {
                [0, 0]
            }
        })
        .collect();
    let target = toward(1200.0 * k, pitch + 720.0 * k);
    let mouse =
        |i: usize| [0, 1].map(|axis| moves[..i].iter().map(|m| m[axis] as f64).sum::<f64>());
    let frames: Vec<_> = (250..=550)
        .step_by(10)
        .map(|i| frame_at(&cfg, i, seen(&cfg, target, mouse(i), pitch)))
        .collect();
    record(&path, &cfg, &moves, 540);
    write_frames(&path, &frames);
    assert_eq!(
        build_events(&path).unwrap(),
        1,
        "{}",
        summary(&path)["events"]
    );
    let summary = summary(&path);
    let fitted = summary["starting_pitch_degrees"].as_f64().unwrap();
    assert!((fitted - pitch).abs() < 0.05, "{fitted}");
    // the turn left from tick 300 + n is 1200 - 5n right and 720 - 3n up; B80 is where that first
    // comes within the radius plus a fifth of the way from it
    let (_, radius) = cfg.geometry(&frames[0].detections[0]);
    let left = |n: usize| (1200.0 - 5.0 * n as f64).hypot(720.0 - 3.0 * n as f64);
    let n = (0..240)
        .find(|n| left(*n) <= radius + 0.2 * (left(0) - radius))
        .unwrap();
    assert_eq!(
        summary["events"][0]["b_us"].as_u64().unwrap(),
        ORIGIN + (300 + n) as u64 * 1000
    );
    let dataset =
        crate::analysis::dataset::Dataset::load(&path.with_extension("events.npz")).unwrap();
    let truth = [1200.0 - 5.0 * n as f64, -(720.0 - 3.0 * n as f64)];
    let found = dataset.targets.unwrap()[0];
    assert!(
        (found[0] - truth[0]).abs() < 0.5 && (found[1] - truth[1]).abs() < 0.5,
        "{found:?} {truth:?}"
    );
}

#[test]
fn target_events_refuse_a_mouse_recording_that_did_not_finish() {
    let folder = crate::testkit::traffic::TempDir::new("vision-refuse");
    std::fs::create_dir_all(folder.path()).unwrap();
    let pcap = folder.path().join("run.pcap");
    std::fs::write(&pcap, b"not read before the decision").unwrap();
    std::fs::write(
        pcap.with_extension("vision.json"),
        crate::testkit::fixtures::vision_evidence("single target"),
    )
    .unwrap();
    std::fs::write(
        pcap.with_extension("json"),
        crate::testkit::fixtures::sidecar("failed").to_string(),
    )
    .unwrap();
    let error = build_events(&pcap).unwrap_err();
    assert!(
        error.starts_with("mouse recording is incomplete"),
        "{error}"
    );
}

#[test]
fn target_events_refuse_a_recording_that_was_cut_short() {
    let folder = crate::testkit::traffic::TempDir::new("vision-cut-short");
    std::fs::create_dir_all(folder.path()).unwrap();
    let pcap = folder.path().join("run.pcap");
    std::fs::write(
        pcap.with_extension("vision.json"),
        crate::testkit::fixtures::vision_evidence("single target"),
    )
    .unwrap();
    std::fs::write(pcap.with_extension("json"), {
        let mut metadata = crate::testkit::fixtures::sidecar("interrupted");
        metadata["stop_reason"] = "app_exit".into();
        metadata.to_string()
    })
    .unwrap();
    let error = build_events(&pcap).unwrap_err();
    assert!(
        error.starts_with("mouse recording is incomplete"),
        "{error}"
    );
}

#[test]
fn target_events_need_only_a_named_scenario() {
    let folder = crate::testkit::traffic::TempDir::new("vision-unnamed");
    std::fs::create_dir_all(folder.path()).unwrap();
    let pcap = folder.path().join("run.pcap");
    std::fs::write(
        pcap.with_extension("vision.json"),
        crate::testkit::fixtures::vision_evidence("  "),
    )
    .unwrap();
    assert_eq!(
        build_events(&pcap).unwrap_err(),
        "target capture had no scenario name, so this recording has no Full49 events"
    );
}

#[test]
fn a_frame_needs_its_native_clocks_and_settings_and_evidence_every_field() {
    let config = cs2();
    let mut evidence = Evidence {
        schema: "motion-bench.vision.v1".into(),
        config: config.clone(),
        model_sha256: "synthetic".into(),
        provider: "synthetic".into(),
        pipeline: "synthetic".into(),
        inference: None,
        complete: true,
        error: None,
        clock: Some(crate::testkit::fixtures::screen_clock()),
    };
    let good = crate::testkit::fixtures::frame(1_000_000, vec![]);
    assert!(good.validate(&evidence).is_ok());
    let broken: [fn(&mut Frame); 6] = [
        |f| f.source_qpc = 0,
        |f| f.received_qpc = f.source_qpc - 1,
        |f| f.completed_qpc = f.received_qpc - 1,
        |f| f.clock_anchor.qpc_before = f.clock_anchor.qpc_after + 1,
        // a time that isn't the one its clock gives
        |f| f.source_us += 1,
        |f| f.clock_anchor.utc_us += 1,
    ];
    for (i, breaking) in broken.into_iter().enumerate() {
        let mut frame = good.clone();
        breaking(&mut frame);
        assert!(frame.validate(&evidence).is_err(), "{i}");
    }
    evidence.clock = None;
    assert!(good.validate(&evidence).is_err());
    // a frame, the settings or the evidence missing a field does not read
    let without = |value: serde_json::Value, key: &str| {
        let mut value = value;
        value.as_object_mut().unwrap().remove(key);
        value
    };
    let frame = serde_json::to_value(&good).unwrap();
    for key in ["source_qpc", "clock_anchor", "completed_us", "pipeline_ms"] {
        assert!(
            serde_json::from_value::<Frame>(without(frame.clone(), key)).is_err(),
            "{key}"
        );
    }
    let settings = serde_json::to_value(&config).unwrap();
    for key in ["cuda_graph", "tensorrt_fp16", "fps"] {
        assert!(
            serde_json::from_value::<Config>(without(settings.clone(), key)).is_err(),
            "{key}"
        );
    }
    let evidence: serde_json::Value =
        serde_json::from_str(&crate::testkit::fixtures::vision_evidence("x")).unwrap();
    assert!(serde_json::from_value::<Evidence>(without(evidence, "pipeline")).is_err());
}

/// the synchronized trial: one flick to a level target 1000 counts to the right, clicked on, seen
/// every 10 ms. gives its frames back
fn trial(path: &Path) -> Vec<Frame> {
    let cfg = scenario();
    let moves: Vec<_> = (0..550)
        .map(|i| [if (300..500).contains(&i) { 5 } else { 0 }, 0])
        .collect();
    record(path, &cfg, &moves, 500);
    let target = toward(1000.0 * cfg.sensitivity * cfg.yaw, 0.0);
    let mouse = |i: usize| [moves[..i].iter().map(|m| m[0] as f64).sum(), 0.0];
    let frames: Vec<_> = (250..=510)
        .step_by(10)
        .map(|i| frame_at(&cfg, i, seen(&cfg, target, mouse(i), 0.0)))
        .collect();
    write_frames(path, &frames);
    frames
}

/// the synchronized trial's event, its ticks, click and boundaries, pinned
#[test]
fn the_trial_event_stays_put() {
    let folder = crate::testkit::traffic::TempDir::new("event-digest");
    std::fs::create_dir_all(folder.path()).unwrap();
    let path = folder.path().join("trial.pcap");
    trial(&path);
    assert_eq!(build_events(&path).unwrap(), 1);
    let d = crate::analysis::dataset::Dataset::load(&path.with_extension("events.npz")).unwrap();
    let text = format!(
        "{:?} {:?} {:?} {:?} {:?} {:?}",
        d.spans,
        d.future,
        d.prefix,
        d.targets,
        d.radii,
        d.rows("full49").unwrap().1
    );
    assert_eq!(crate::testkit::fixtures::digest(&text), "a848309b58c848ca");
}

#[test]
fn a_failed_rebuild_keeps_the_old_events_but_never_scores_them() {
    let folder = crate::testkit::traffic::TempDir::new("event-stale");
    std::fs::create_dir_all(folder.path()).unwrap();
    let path = folder.path().join("trial.pcap");
    let frames = trial(&path);
    assert_eq!(build_events(&path).unwrap(), 1);
    let read = |extension| std::fs::read(path.with_extension(extension)).unwrap();
    let (dataset, summary) = (read("events.npz"), read("events.json"));
    let scored = |path: &Path| {
        let source = crate::analysis::compare::Source {
            path: path.into(),
            decoder: None,
            endpoint: None,
            crop: None,
        };
        crate::analysis::compare::matching_sources(
            std::slice::from_ref(&source),
            std::slice::from_ref(&source),
        )
        .map(|_| ())
    };
    scored(&path).unwrap();
    let mut regressed = frames.clone();
    regressed.swap(3, 4);
    write_frames(&path, &regressed);
    assert_eq!(
        build_events(&path).unwrap_err(),
        "screen timestamps regressed"
    );
    assert_eq!(read("events.npz"), dataset);
    assert_eq!(read("events.json"), summary);
    assert!(!path.with_extension("events.npz.partial").exists());
    assert!(!path.with_extension("events.json.partial").exists());
    let located = locate(&path).unwrap();
    assert_eq!(located.event_dataset, None);
    assert_eq!(
        located.stale_events.as_deref(),
        Some("screen timestamps regressed")
    );
    let refused = scored(&path).unwrap_err();
    assert!(
        refused.ends_with("none of them are scored, screen timestamps regressed"),
        "{refused}"
    );
    // a build that succeeds again puts them back in
    write_frames(&path, &frames);
    assert_eq!(build_events(&path).unwrap(), 1);
    assert!(!stale_marker(&path).exists());
    scored(&path).unwrap();
    // the app stopping in the middle of a swap leaves its marker up
    std::fs::write(
        stale_marker(&path),
        "the app stopped while they were being replaced",
    )
    .unwrap();
    assert!(scored(&path).is_err());
}

#[test]
fn screen_evidence_without_its_clock_builds_no_events() {
    let folder = crate::testkit::traffic::TempDir::new("event-clockless");
    std::fs::create_dir_all(folder.path()).unwrap();
    let path = folder.path().join("trial.pcap");
    trial(&path);
    let mut evidence: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.with_extension("vision.json")).unwrap())
            .unwrap();
    evidence["clock"] = serde_json::Value::Null;
    std::fs::write(path.with_extension("vision.json"), evidence.to_string()).unwrap();
    assert_eq!(build_events(&path).unwrap_err(), NO_CLOCK);
}

#[test]
fn a_mouse_report_far_ahead_then_back_is_refused_not_indexed_past() {
    let folder = crate::testkit::traffic::TempDir::new("event-spike");
    std::fs::create_dir_all(folder.path()).unwrap();
    let path = folder.path().join("spike.pcap");
    let moves = vec![[1, 0]; 20];
    record_at(&path, &scenario(), &moves, &[10], |i| {
        ORIGIN + if i == 5 { 60_000_000 } else { i as u64 * 1000 }
    });
    write_frames(&path, &[]);
    assert_eq!(
        build_events(&path).unwrap_err(),
        "mouse timestamps regressed"
    );
}

#[test]
fn old_events_that_cannot_be_marked_stale_are_removed() {
    let folder = crate::testkit::traffic::TempDir::new("event-unmarked");
    std::fs::create_dir_all(folder.path()).unwrap();
    let path = folder.path().join("trial.pcap");
    let mut frames = trial(&path);
    assert_eq!(build_events(&path).unwrap(), 1);
    // a folder where the marker goes, so writing it fails
    std::fs::create_dir(stale_marker(&path)).unwrap();
    frames.swap(3, 4);
    write_frames(&path, &frames);
    assert!(build_events(&path).is_err());
    assert!(!path.with_extension("events.npz").exists());
}

/// the user never stops moving, so the next trial starts the tick after a click, while the screen
/// still shows the clicked target leaving with the next one. that swap isn't the next trial's
#[test]
fn the_target_swap_after_a_click_is_left_out_of_the_next_trial() {
    let folder = crate::testkit::traffic::TempDir::new("event-swap");
    std::fs::create_dir_all(folder.path()).unwrap();
    let path = folder.path().join("trial.pcap");
    let cfg = scenario();
    // a flick right to the first target, clicked at 500, then straight back to the second
    let moves: Vec<_> = (0..1050)
        .map(|i| match i {
            300..=499 => [5, 0],
            501..=700 => [-5, 0],
            _ => [0, 0],
        })
        .collect();
    record_at(&path, &cfg, &moves, &[500, 1000], |i| {
        ORIGIN + i as u64 * 1000
    });
    let first = toward(1000.0 * cfg.sensitivity * cfg.yaw, 0.0);
    let second = toward(0.0, 0.0);
    let mouse = |i: usize| [moves[..i].iter().map(|m| m[0] as f64).sum(), 0.0];
    let mut frames: Vec<_> = (250..=500)
        .step_by(10)
        .map(|i| frame_at(&cfg, i, seen(&cfg, first, mouse(i), 0.0)))
        .collect();
    // both on screen for a frame, 6 ms after the click
    let mut swap = frame_at(&cfg, 506, seen(&cfg, first, mouse(506), 0.0));
    swap.detections
        .extend(frame_at(&cfg, 506, seen(&cfg, second, mouse(506), 0.0)).detections);
    frames.push(swap);
    frames.extend(
        (510..=1000)
            .step_by(10)
            .map(|i| frame_at(&cfg, i, seen(&cfg, second, mouse(i), 0.0))),
    );
    write_frames(&path, &frames);
    assert_eq!(build_events(&path).unwrap(), 2, "{}", summary(&path));
}

/// a frame of two targets as one replaces the other isn't marked, two targets for a while is
#[test]
fn only_a_lasting_second_target_is_marked_for_the_crop() {
    let folder = crate::testkit::traffic::TempDir::new("crowded");
    std::fs::create_dir_all(folder.path()).unwrap();
    let path = folder.path().join("trial.pcap");
    let cfg = scenario();
    let frames: Vec<_> = (0..400)
        .step_by(2)
        .map(|i| {
            let mut frame = frame_at(&cfg, i, [1000.0, 700.0]);
            if i == 100 || (200..=320).contains(&i) {
                frame
                    .detections
                    .extend(frame_at(&cfg, i, [1500.0, 700.0]).detections);
            }
            frame
        })
        .collect();
    write_frames(&path, &frames);
    assert_eq!(
        crowded_spans(&path).unwrap(),
        vec![[ORIGIN + 200_000, ORIGIN + 320_000]]
    );
}
