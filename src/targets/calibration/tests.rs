use super::*;
use crate::targets::vision::{Evidence, FovAxis, VISION_SCHEMA};
/// the projection these tests were worked out with: CS2, whose 90 degrees at 4:3 show 106.26
/// degrees across a 16:9 screen
fn cs2() -> Config {
    Config {
        fov: 90.0,
        fov_axis: FovAxis::Horizontal4x3,
        yaw: 0.022,
        pitch: 0.022,
        ..Config::default()
    }
}
/// the screen the tests are worked out on
const SCREEN: Screen = [2560, 1440];
type Motion = Vec<(u64, [f64; 2])>;
type Observations = Vec<(u64, [f64; 2], f64)>;
/// where a target in `direction` shows on screen, in the counts `Config::counts` gives, for a
/// camera turned by `mouse` counts from `pitch` degrees; worked out apart from `Camera`, so a
/// mistake in one does not hide in the other
fn seen(profile: &Config, direction: [f64; 3], mouse: [f64; 2], pitch: f64) -> [f64; 2] {
    let k = [
        profile.sensitivity * profile.yaw,
        profile.sensitivity * profile.pitch,
    ];
    let yaw = (mouse[0] * k[0]).to_radians();
    let pitch = (pitch - mouse[1] * k[1]).to_radians();
    // the target's yaw and pitch in the world, then relative to where the camera looks
    let target_yaw = direction[0].atan2(direction[2]);
    let target_pitch = direction[1].asin();
    let (dy, p, q) = (target_yaw - yaw, target_pitch, pitch);
    // the camera's own axes: right, up, forward
    let x = p.cos() * dy.sin();
    let y = p.sin() * q.cos() - p.cos() * q.sin() * dy.cos();
    let z = p.sin() * q.sin() + p.cos() * q.cos() * dy.cos();
    // screen y grows downwards; each axis as `Config::counts` reads a pixel offset
    [
        x.atan2(z).to_degrees() / k[0],
        (-y).atan2(z).to_degrees() / k[1],
    ]
}
/// mouse movement and the target's place on screen 37 ms later, for a camera that started
/// looking `pitch` degrees up at a target `swing` counts across
fn fixture_at(linear: bool, pitch: f64, swing: f64) -> (Motion, Observations) {
    let profile = cs2();
    let path = |t: f64| {
        if linear {
            [t * 40.0, 0.0]
        } else {
            [(t * 4.7).sin() * swing, (t * 2.9).cos() * swing * 0.75]
        }
    };
    let target = Camera::new(&profile, SCREEN).direction([60.0, -40.0], [0.0, 0.0], pitch);
    let motion = (0..6001)
        .map(|i| (1_000_000 + i * 1000, path(i as f64 / 1000.0)))
        .collect();
    let frames = (25..600)
        .map(|i| {
            let t = i as f64 / 100.0;
            let p = seen(&profile, target, path(t - 0.037), pitch);
            (1_000_000 + i * 10_000, p, 30.0)
        })
        .collect();
    (motion, frames)
}
fn fixture(linear: bool) -> (Motion, Observations) {
    fixture_at(linear, 0.0, 200.0)
}
#[test]
fn varying_movement_recovers_delay_and_preserves_unknown_hardware_latency() {
    let (m, f) = fixture(false);
    let r = fit(&cs2(), SCREEN, &m, &f).unwrap();
    assert!(r.estimated_delay_ms.abs_diff(37) <= 1);
    assert!(r.identifiable);
    assert!(!r.absolute_hardware_latency_known && !r.applied_to_scores);
}
/// a target well above the camera, swung far across the screen: each axis alone would misplace it
/// by more than its size, the camera's whole rotation keeps every frame on it
#[test]
fn a_target_above_the_camera_fits_with_wide_swings() {
    let (m, f) = fixture_at(false, 26.0, 900.0);
    let r = fit(&cs2(), SCREEN, &m, &f).unwrap();
    assert!(
        r.estimated_delay_ms.abs_diff(37) <= 1,
        "{}",
        r.estimated_delay_ms
    );
    assert!(r.identifiable, "{:?}", r.reason);
    // the starting pitch is found to a quarter degree, about 7 counts at the most
    assert!(r.residual_counts < 10.0, "{}", r.residual_counts);
    assert_eq!(r.frames, f.len());
}
#[test]
fn constant_velocity_cannot_identify_delay_and_gaps_are_rejected() {
    let (m, mut f) = fixture(true);
    let r = fit(&cs2(), SCREEN, &m, &f).unwrap();
    assert!(!r.identifiable);
    assert_eq!(r.admissible_delay_ms, [0, 250]);
    f.drain(150..180);
    assert!(fit(&cs2(), SCREEN, &m, &f).err().unwrap().contains("gaps"));
}
#[test]
fn recording_import_selects_single_target_coverage() {
    use crate::targets::vision::Detection;
    let folder = std::env::temp_dir().join(format!(
        "motion-bench-calibration-{}",
        crate::capture::clock::qpc()
    ));
    std::fs::create_dir_all(&folder).unwrap();
    let pcap = folder.join("trial.pcap");
    let config = Config {
        scenario: "synthetic stationary target".into(),
        ..cs2()
    };
    let origin = 1_790_000_000_000_000u64;
    let (m, _) = fixture(false);
    let motion: Vec<_> = m
        .iter()
        .map(|(t, p)| (origin + t - 1_000_000, [p[0].round(), p[1].round()]))
        .collect();
    let mut bytes = Vec::new();
    bytes.extend(0xa1b2c3d4u32.to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(4u16.to_le_bytes());
    bytes.extend([0; 8]);
    bytes.extend(65535u32.to_le_bytes());
    bytes.extend(249u32.to_le_bytes());
    let mut previous = [0.0; 2];
    for (t, p) in &motion {
        let mut packet = vec![0u8; 27];
        packet[..2].copy_from_slice(&27u16.to_le_bytes());
        packet[16] = 1;
        packet[17..19].copy_from_slice(&1u16.to_le_bytes());
        packet[19..21].copy_from_slice(&31u16.to_le_bytes());
        packet[21] = 129;
        packet[22] = 1;
        packet[23..27].copy_from_slice(&5u32.to_le_bytes());
        packet.push(0);
        packet.extend(((p[0] - previous[0]) as i16).to_le_bytes());
        packet.extend(((p[1] - previous[1]) as i16).to_le_bytes());
        previous = *p;
        bytes.extend(((t / 1_000_000) as u32).to_le_bytes());
        bytes.extend(((t % 1_000_000) as u32).to_le_bytes());
        bytes.extend((packet.len() as u32).to_le_bytes());
        bytes.extend((packet.len() as u32).to_le_bytes());
        bytes.extend(packet);
    }
    std::fs::write(&pcap, &bytes).unwrap();
    let metadata = {
        let mut metadata = crate::testkit::fixtures::sidecar("complete");
        metadata["selection"] = serde_json::json!({"bus": 1, "address": 31, "endpoint": 129});
        metadata
    };
    std::fs::write(
        pcap.with_extension("json"),
        serde_json::to_vec(&metadata).unwrap(),
    )
    .unwrap();
    let evidence = Evidence {
        schema: VISION_SCHEMA.into(),
        config: config.clone(),
        screen: SCREEN,
        model_sha256: "synthetic".into(),
        provider: "synthetic".into(),
        pipeline: "synthetic".into(),
        inference: None,
        complete: true,
        error: None,
        clock: Some(crate::testkit::fixtures::screen_clock()),
    };
    std::fs::write(
        pcap.with_extension("vision.json"),
        serde_json::to_vec(&evidence).unwrap(),
    )
    .unwrap();
    let focal = config.focal(SCREEN);
    let target = Camera::new(&config, SCREEN).direction([500.0, 200.0], [0.0, 0.0], 0.0);
    let mut journal = String::new();
    for i in 0..600u64 {
        let t = origin + i * 10_000;
        let c = seen(
            &config,
            target,
            position(&motion, t.saturating_sub(37_000)),
            0.0,
        );
        let px = |v: f64| {
            SCREEN[0] as f64 / 2.0
                + (v * config.sensitivity * config.yaw).to_radians().tan() * focal
        };
        let py = |v: f64| {
            SCREEN[1] as f64 / 2.0
                + (v * config.sensitivity * config.pitch).to_radians().tan() * focal
        };
        let d = Detection {
            bounds: [
                // a box drawn around the target's center on screen
                px(c[0]) - 40.0,
                py(c[1]) - 40.0,
                px(c[0]) + 40.0,
                py(c[1]) + 40.0,
            ],
            confidence: 1.0,
            class: 0,
        };
        let detections = if i < 20 { vec![d.clone(), d] } else { vec![d] };
        journal.push_str(
            &serde_json::to_string(&crate::testkit::fixtures::frame(t, detections)).unwrap(),
        );
        journal.push('\n');
    }
    std::fs::write(pcap.with_extension("targets.jsonl"), journal).unwrap();
    let result = recording(&pcap, None).unwrap();
    assert!(result.identifiable);
    assert!(
        result.estimated_delay_ms.abs_diff(37) <= 1,
        "{} [{:?}] {}",
        result.estimated_delay_ms,
        result.admissible_delay_ms,
        result.residual_counts
    );
    assert!(result.selected_range_seconds[0] >= 0.25);
    assert_eq!(result.single_target_segments, 1);
    let mut clicked = bytes.clone();
    let mut offset = 24;
    for index in 0..motion.len() {
        let length =
            u32::from_le_bytes(clicked[offset + 8..offset + 12].try_into().unwrap()) as usize;
        if (100..150).contains(&index) || (1200..1400).contains(&index) || index >= 5900 {
            clicked[offset + 16 + 27] = 1;
        }
        offset += 16 + length;
    }
    std::fs::write(&pcap, clicked).unwrap();
    let recovered = recording(&pcap, None).unwrap();
    assert!(recovered.identifiable);
    assert!(recovered.estimated_delay_ms.abs_diff(37) <= 1);
    assert!(recovered.selected_range_seconds[0] > 1.65);
    assert!(recovered.selected_range_seconds[1] < 5.9);
    assert_eq!(recovered.single_target_segments, 2);
    assert!(recovered.excluded_samples > 0);
    // cropped after the clicks at the start, they and the movement around them are not read
    let crop = crate::analysis::compare::Selection {
        from_us: origin + 1_450_000,
        to_us: origin + 6_000_000,
    };
    let cropped = recording(&pcap, Some(crop)).unwrap();
    assert!(cropped.identifiable);
    assert!(cropped.estimated_delay_ms.abs_diff(37) <= 1);
    assert_eq!(cropped.single_target_segments, 1);
    assert!((0.25..0.3).contains(&cropped.selected_range_seconds[0]));
    assert_eq!(cropped.crop_us, Some(crop));
    let invalid = crate::testkit::fixtures::frame(
        origin,
        vec![Detection {
            bounds: [0.0, 0.0, 10000.0, 10000.0],
            confidence: 1.0,
            class: 0,
        }],
    );
    std::fs::write(
        pcap.with_extension("targets.jsonl"),
        serde_json::to_string(&invalid).unwrap(),
    )
    .unwrap();
    assert!(recording(&pcap, None).err().unwrap().contains("geometry"));
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn calibration_needs_only_a_named_scenario() {
    let folder = crate::testkit::traffic::TempDir::new("calibration-unnamed");
    std::fs::create_dir_all(folder.path()).unwrap();
    let pcap = folder.path().join("trial.pcap");
    std::fs::write(&pcap, b"not read before the decision").unwrap();
    std::fs::write(
        pcap.with_extension("vision.json"),
        crate::testkit::fixtures::vision_evidence(""),
    )
    .unwrap();
    std::fs::write(
        pcap.with_extension("json"),
        {
            let mut metadata = crate::testkit::fixtures::sidecar("complete");
            metadata["selection"] = serde_json::json!({"bus": 1, "address": 31, "endpoint": 129});
            metadata
        }
        .to_string(),
    )
    .unwrap();
    assert_eq!(
        recording(&pcap, None).err().unwrap(),
        "target capture had no scenario name, so this recording cannot calibrate"
    );
}

#[test]
fn calibration_refuses_a_mouse_recording_that_did_not_finish() {
    let folder = crate::testkit::traffic::TempDir::new("calibration-refuse");
    std::fs::create_dir_all(folder.path()).unwrap();
    let pcap = folder.path().join("run.pcap");
    std::fs::write(
        pcap.with_extension("json"),
        crate::testkit::fixtures::sidecar("failed").to_string(),
    )
    .unwrap();
    let error = recording(&pcap, None).err().unwrap();
    assert!(error.contains("complete mouse recording"), "{error}");
}

#[test]
fn calibration_refuses_a_recording_that_was_cut_short() {
    let folder = crate::testkit::traffic::TempDir::new("calibration-cut-short");
    std::fs::create_dir_all(folder.path()).unwrap();
    let pcap = folder.path().join("run.pcap");
    std::fs::write(pcap.with_extension("json"), {
        let mut metadata = crate::testkit::fixtures::sidecar("interrupted");
        metadata["stop_reason"] = "app_exit".into();
        metadata.to_string()
    })
    .unwrap();
    let error = recording(&pcap, None).err().unwrap();
    assert!(error.contains("complete mouse recording"), "{error}");
}
