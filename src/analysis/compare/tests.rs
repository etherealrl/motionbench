use super::*;

/// a recording's wire, read the way a comparison reads it
fn read_wire(
    path: &Path,
    decoder: Option<Decoder>,
    endpoint: Option<&str>,
    crop: Option<Selection>,
    cancel: &AtomicBool,
) -> Result<Wire, crate::analysis::error::Error> {
    // the reader only opens a recording this app made
    let sidecar = path.with_extension("json");
    if !sidecar.exists() {
        std::fs::write(
            sidecar,
            crate::testkit::fixtures::sidecar("complete").to_string(),
        )
        .unwrap();
    }
    let source = Source {
        path: path.into(),
        decoder,
        endpoint: endpoint.map(String::from),
        crop,
    };
    open_wire(&source, path, cancel)
}

#[test]
fn windows_process_a_full_recording_without_a_ten_minute_cutoff() {
    let mut stream = crate::signal::stats::Stream::new(u64::MAX);
    for t in [0, 700_000_000, 700_160_000] {
        stream.push_decoded(t, Some((8, 3)));
    }
    let (rows, idle) = windows(
        &stream.view(0, 701_000_000),
        0,
        701_000,
        160,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(rows.iter().any(|r| r.index > 4000));
    assert!(idle > 4000);
    assert!(rows.len() <= 3);
}
#[test]
fn cancelled_window_analysis_stops_before_scoring() {
    let mut stream = crate::signal::stats::Stream::new(u64::MAX);
    stream.push_decoded(1, Some((2, 3)));
    assert_eq!(
        windows(&stream.view(0, 1000), 0, 1000, 160, &AtomicBool::new(true))
            .err()
            .unwrap(),
        crate::analysis::error::Error::Cancelled
    );
}
#[test]
fn paired_streaming_preserves_counts_and_zero_gaps() {
    let folder = crate::testkit::traffic::TempDir::new("paired-stream");
    let fixture = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures"));
    let path = crate::testkit::fixtures::recorded(
        folder.path(),
        "mouse-1000hz.pcap",
        &serde_json::from_slice(&std::fs::read(fixture.join("mouse-1000hz.json")).unwrap())
            .unwrap(),
    );
    let source = std::fs::read(&path).unwrap();
    let mut text = String::from("# paired_shift_us=0\nbox_us,dx,dy,dz\n");
    crate::capture::pcap::read_pcap(&source[..], |record| {
        if let crate::capture::pcap::Record::Packet {
            event: Some(crate::capture::pcap::Event::Report(report)),
            ..
        } = record
        {
            let (x, y) = crate::capture::pcap::motion(report.bytes()).unwrap();
            text.push_str(&format!("{},{x},{y},0\n", report.t_us));
        }
    })
    .unwrap();
    std::fs::write(path.with_extension("input.csv"), text).unwrap();
    let config = Config {
        bootstrap: 0,
        permutations: 0,
        ..Config::default()
    };
    let result = paired(&path, None, config, None, &AtomicBool::new(false), |_| {}).unwrap();
    assert!(result.w1.iter().all(|v| *v == 0.0));
    // the paired result says how the recording it read ended, and what it counted
    assert_eq!(result.metadata["recording_status"]["kind"], "complete");
    assert_eq!(result.metadata["paired"]["input_rows"], 400);
    assert_eq!(result.metadata["counts"]["reports"], 400);
    assert!(result.judge_unavailable.is_some());
    assert!(
        !result.metadata["source_groups"]["baseline"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_cancel_reaches_the_caller_as_cancelled_through_the_reader_and_the_judge() {
    let asked = AtomicBool::new(true);
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/mouse-1000hz.pcap"
    );
    // the reader stops inside `io::Error`, the variant has to survive the trip
    assert_eq!(
        read_wire(
            Path::new(path),
            Some(crate::testkit::traffic::mouse_decoder()),
            None,
            None,
            &asked
        )
        .err(),
        Some(crate::analysis::error::Error::Cancelled)
    );
    assert_eq!(
        WindowCollector::new(0, 1_000_000, 160, &asked)
            .push(0, 1, 1)
            .err(),
        Some(crate::analysis::error::Error::Cancelled)
    );
    // a judge that was cancelled must not turn into a finished result without a judge
    let population = || Population {
        rows: vec![vec![1.0; 3]; 8],
        groups: (0..8).map(|i| format!("trial-{}", i / 2)).collect(),
        idle: 0,
    };
    let result = finish(
        population(),
        population(),
        vec!["a".into(), "b".into(), "c".into()],
        Config::default(),
        &asked,
        |_| {},
    );
    assert_eq!(result.err(), Some(crate::analysis::error::Error::Cancelled));
}

#[test]
fn a_disconnect_only_refuses_the_range_it_falls_in() {
    use crate::testkit::traffic::{Descriptors, Packet, TempDir, Traffic, mouse_payload, ticks};
    let folder = TempDir::new("wire-disconnect");
    std::fs::create_dir_all(folder.path()).unwrap();
    let path = folder.path().join("run.pcap");
    let mut traffic = Traffic::default();
    traffic.describe(0, 1, 2, &Descriptors::mouse(0x373b, 0x11d9));
    traffic.extend(
        ticks(1000, 1_000_000, 500)
            .map(|t| (t, Packet::report(1, 2, 0x81, &mouse_payload(1, -1, 7)))),
    );
    traffic.add(1_600_000, Packet::disconnect(1, 2, 0x81));
    std::fs::write(&path, traffic.pcap().bytes()).unwrap();

    let read = read_wire(
        &path,
        Some(crate::testkit::traffic::mouse_decoder()),
        None,
        None,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(read.disconnected_at, Some(1_600_000));
    // the reports all came before the unplug, so the whole recording and any part of it reads
    assert!(read.covers(read.first, read.last).is_ok());
    assert!(read.covers(1_100_000, 1_200_000).is_ok());
    // a range that reaches past the unplug does not
    assert!(read.covers(1_100_000, 1_700_000).is_err());
}

#[test]
fn paired_and_independent_comparisons_ask_the_shared_decision() {
    use crate::testkit::traffic::{Descriptors, Packet, TempDir, Traffic, mouse_payload, ticks};
    let folder = TempDir::new("compare-refuse");
    std::fs::create_dir_all(folder.path()).unwrap();
    // a paired recording that failed is refused before anything is read
    let failed = folder.path().join("failed.pcap");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/mouse-1000hz.pcap"
        ),
        &failed,
    )
    .unwrap();
    std::fs::write(
        failed.with_extension("incomplete.json"),
        r#"{"reason":"device_lost","error":"unplugged"}"#,
    )
    .unwrap();
    let error = paired(
        &failed,
        Some(crate::testkit::traffic::mouse_decoder()),
        Config::default(),
        None,
        &AtomicBool::new(false),
        |_| {},
    )
    .err()
    .unwrap()
    .to_string();
    assert!(error.contains("unplugged"), "{error}");

    // an independent range that reaches past an unplug is refused, one before it is not
    let unplugged = folder.path().join("unplugged.pcap");
    let mut traffic = Traffic::default();
    traffic.describe(0, 1, 2, &Descriptors::mouse(0x373b, 0x11d9));
    traffic.extend(
        ticks(1000, 1_000_000, 2000)
            .map(|t| (t, Packet::report(1, 2, 0x81, &mouse_payload(1, -1, 7)))),
    );
    traffic.add(2_500_000, Packet::disconnect(1, 2, 0x81));
    std::fs::write(&unplugged, traffic.pcap().bytes()).unwrap();
    std::fs::write(
        unplugged.with_extension("json"),
        crate::testkit::fixtures::sidecar("complete").to_string(),
    )
    .unwrap();
    let source = || Source {
        path: unplugged.clone(),
        decoder: Some(crate::testkit::traffic::mouse_decoder()),
        endpoint: None,
        crop: None,
    };
    let compare = |range: [f64; 2]| {
        independent_selected(
            &[source()],
            &[source()],
            "texture19",
            Some(range),
            Config {
                bootstrap: 0,
                permutations: 0,
                ..Config::default()
            },
            &AtomicBool::new(false),
            |_| {},
        )
        .err()
        .map(|e| e.to_string())
    };
    assert!(compare([0.0, 2.0]).is_some_and(|e| e.contains("disconnected")));
    assert!(!compare([0.0, 1.0]).is_some_and(|e| e.contains("disconnected")));
}

fn unplug_traffic(
    devices: &[(u16, u16)],
    unplug: Option<(u16, u64)>,
) -> crate::testkit::traffic::Traffic {
    use crate::testkit::traffic::{Descriptors, Packet, Traffic, mouse_payload, ticks};
    let mut traffic = Traffic::default();
    for (address, vid) in devices {
        traffic.describe(0, 1, *address, &Descriptors::mouse(*vid, 0x11d9));
    }
    for t in ticks(1000, 1_000_000, 500) {
        for (address, _) in devices {
            // a device that was unplugged sends nothing after it
            if unplug.is_some_and(|(gone, at)| gone == *address && t >= at) {
                continue;
            }
            traffic.add(
                t,
                Packet::report(1, *address, 0x81, &mouse_payload(1, -1, 7)),
            );
        }
    }
    if let Some((address, t)) = unplug {
        traffic.add(t, Packet::disconnect(1, address, 0x81));
    }
    traffic
}

#[test]
fn an_unplug_is_tied_to_its_own_device_and_its_first_moment() {
    use crate::testkit::traffic::{Packet, TempDir};
    let folder = TempDir::new("wire-unplug-cases");
    std::fs::create_dir_all(folder.path()).unwrap();
    let read = |name: &str, traffic: crate::testkit::traffic::Traffic, endpoint: Option<&str>| {
        let path = folder.path().join(name);
        std::fs::write(&path, traffic.pcap().bytes()).unwrap();
        read_wire(
            &path,
            Some(crate::testkit::traffic::mouse_decoder()),
            endpoint,
            None,
            &AtomicBool::new(false),
        )
    };

    // with two devices nothing is picked alone, the same as Runs, and the other one going away
    // says nothing about the one picked
    let other = || unplug_traffic(&[(2, 0x373b), (3, 0x1234)], Some((3, 1_200_000)));
    assert!(read("other.pcap", other(), None).is_err());
    let mouse = read("picked.pcap", other(), Some("1:2:129")).unwrap();
    assert_eq!(mouse.key.device, 2);
    assert_eq!(mouse.disconnected_at, None);

    // the first unplug counts, a later one does not move it
    let mut twice = unplug_traffic(&[(2, 0x373b)], Some((2, 1_600_000)));
    twice.add(1_700_000, Packet::disconnect(1, 2, 0x81));
    let read_twice = read("twice.pcap", twice, None).unwrap();
    assert_eq!(read_twice.disconnected_at, Some(1_600_000));

    // the boundary: a range ending exactly at the unplug reaches it, one after it starts after it
    assert!(read_twice.covers(1_100_000, 1_599_999).is_ok());
    assert!(read_twice.covers(1_100_000, 1_600_000).is_err());
    let after = read_twice
        .covers(1_650_000, 1_800_000)
        .unwrap_err()
        .to_string();
    assert!(after.contains("before the selected range"), "{after}");

    // another device taking the address mixes two devices under one key, so it is refused
    let mut reused = unplug_traffic(&[(2, 0x373b)], Some((2, 1_600_000)));
    reused.describe(
        1_650_000,
        1,
        2,
        &crate::testkit::traffic::Descriptors::mouse(0x0bad, 0x0001),
    );
    for t in crate::testkit::traffic::ticks(1000, 1_700_000, 100) {
        reused.add(
            t,
            Packet::report(1, 2, 0x81, &crate::testkit::traffic::mouse_payload(1, 1, 7)),
        );
    }
    let mut again = unplug_traffic(&[(2, 0x373b)], None);
    again.describe(
        1_200_000,
        1,
        2,
        &crate::testkit::traffic::Descriptors::mouse(0x373b, 0x11d9),
    );
    assert!(read("again.pcap", again, Some("1:2:129")).is_ok());

    let error = read("reused.pcap", reused, Some("1:2:129"))
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains("took the selected device's address"),
        "{error}"
    );
}

#[test]
fn a_paired_range_past_an_unplug_is_refused() {
    use crate::testkit::traffic::TempDir;
    let folder = TempDir::new("paired-unplug");
    std::fs::create_dir_all(folder.path()).unwrap();
    let path = folder.path().join("run.pcap");
    std::fs::write(
        &path,
        unplug_traffic(&[(2, 0x373b)], Some((2, 1_300_000)))
            .pcap()
            .bytes(),
    )
    .unwrap();
    std::fs::write(
        path.with_extension("json"),
        crate::testkit::fixtures::sidecar("complete").to_string(),
    )
    .unwrap();
    let mut input = String::from("# paired_shift_us=0\nbox_us,dx,dy,dz\n");
    for i in 0..500u64 {
        input.push_str(&format!("{},1,-1,0\n", 1_000_000 + i * 1000));
    }
    std::fs::write(path.with_extension("input.csv"), input).unwrap();
    let run = |to_us: u64| {
        paired(
            &path,
            Some(crate::testkit::traffic::mouse_decoder()),
            Config {
                bootstrap: 0,
                permutations: 0,
                ..Config::default()
            },
            // the first report is at 1 s
            Some([0.0, (to_us - 1_000_000) as f64 / 1e6]),
            &AtomicBool::new(false),
            |_| {},
        )
        .err()
        .map(|e| e.to_string())
    };
    // the reports stop before the unplug, so a range that ends there is whole
    assert!(!run(1_250_000).is_some_and(|e| e.contains("disconnected")));
    // one that reaches the unplug is not
    assert!(run(1_400_000).is_some_and(|e| e.contains("disconnected")));
}

#[test]
fn an_independent_comparison_reads_a_recording_whose_pairing_failed() {
    use crate::testkit::traffic::TempDir;
    let folder = TempDir::new("independent-unpaired");
    std::fs::create_dir_all(folder.path()).unwrap();
    let path = folder.path().join("run.pcap");
    std::fs::write(&path, unplug_traffic(&[(2, 0x373b)], None).pcap().bytes()).unwrap();
    std::fs::write(
        path.with_extension("json"),
        crate::testkit::fixtures::sidecar("degraded")
            .as_object()
            .cloned()
            .map(|mut s| {
                s["health"] = serde_json::json!({"error": null, "pairing_valid": false});
                serde_json::Value::Object(s).to_string()
            })
            .unwrap(),
    )
    .unwrap();
    let source = || Source {
        path: path.clone(),
        decoder: Some(crate::testkit::traffic::mouse_decoder()),
        endpoint: None,
        crop: None,
    };
    let result = independent_selected(
        &[source()],
        &[source()],
        "texture19",
        None,
        Config {
            bootstrap: 0,
            permutations: 0,
            ..Config::default()
        },
        &AtomicBool::new(false),
        |_| {},
    );
    let result = result.unwrap();
    // the result says what kind of recording each source was
    assert_eq!(
        result.metadata["sources"][0]["recording_status"]["kind"],
        "degraded"
    );
}

#[test]
fn both_independent_branches_refuse_a_failed_recording() {
    use crate::testkit::traffic::TempDir;
    let folder = TempDir::new("independent-failed");
    std::fs::create_dir_all(folder.path()).unwrap();
    let pcap = folder.path().join("run.pcap");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/mouse-1000hz.pcap"
        ),
        &pcap,
    )
    .unwrap();
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/events.npz"),
        pcap.with_extension("events.npz"),
    )
    .unwrap();
    std::fs::write(
        pcap.with_extension("incomplete.json"),
        r#"{"reason":"writer_failed","error":"disk full"}"#,
    )
    .unwrap();
    let source = || Source {
        path: pcap.clone(),
        decoder: Some(crate::testkit::traffic::mouse_decoder()),
        endpoint: None,
        crop: None,
    };
    // texture reads the wire, trajectory reads the event dataset beside it
    for panel in ["texture19", "trajectory14"] {
        let error = independent_selected(
            &[source()],
            &[source()],
            panel,
            None,
            Config::default(),
            &AtomicBool::new(false),
            |_| {},
        )
        .err()
        .unwrap()
        .to_string();
        assert!(error.contains("disk full"), "{panel}: {error}");
    }
}

#[test]
fn a_paired_comparison_refuses_a_recording_whose_pairing_failed() {
    let folder = crate::testkit::traffic::TempDir::new("paired-unpaired");
    std::fs::create_dir_all(folder.path()).unwrap();
    let pcap = folder.path().join("run.pcap");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/mouse-1000hz.pcap"
        ),
        &pcap,
    )
    .unwrap();
    std::fs::write(
        pcap.with_extension("json"),
        crate::testkit::fixtures::sidecar("degraded")
            .as_object()
            .cloned()
            .map(|mut s| {
                s["health"] = serde_json::json!({"error": null, "pairing_valid": false});
                serde_json::Value::Object(s).to_string()
            })
            .unwrap(),
    )
    .unwrap();
    let error = paired(
        &pcap,
        Some(crate::testkit::traffic::mouse_decoder()),
        Config::default(),
        None,
        &AtomicBool::new(false),
        |_| {},
    )
    .err()
    .unwrap()
    .to_string();
    assert!(error.contains("clock could not be lined up"), "{error}");
}

#[test]
fn an_event_comparison_says_what_kind_of_recording_each_source_was() {
    let folder = crate::testkit::traffic::TempDir::new("events-status");
    std::fs::create_dir_all(folder.path()).unwrap();
    let pcap = folder.path().join("run.pcap");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/mouse-1000hz.pcap"
        ),
        &pcap,
    )
    .unwrap();
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/events.npz"),
        pcap.with_extension("events.npz"),
    )
    .unwrap();
    std::fs::write(
        pcap.with_extension("json"),
        crate::testkit::fixtures::sidecar("degraded").to_string(),
    )
    .unwrap();
    let source = || Source {
        path: pcap.clone(),
        decoder: Some(crate::testkit::traffic::mouse_decoder()),
        endpoint: None,
        crop: None,
    };
    let result = independent_selected(
        &[source()],
        &[source()],
        "trajectory14",
        None,
        Config {
            bootstrap: 0,
            permutations: 0,
            ..Config::default()
        },
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(
        result.metadata["sources"][0]["recording_status"]["kind"],
        "degraded"
    );
}

#[test]
fn the_cli_refuses_a_failed_recording_and_writes_nothing() {
    let folder = crate::testkit::traffic::TempDir::new("cli-failed");
    std::fs::create_dir_all(folder.path()).unwrap();
    let pcap = folder.path().join("run.pcap");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/mouse-1000hz.pcap"
        ),
        &pcap,
    )
    .unwrap();
    std::fs::write(
        pcap.with_extension("incomplete.json"),
        r#"{"reason":"device_lost","error":"unplugged"}"#,
    )
    .unwrap();
    let code = cli(vec!["judge".into(), pcap.display().to_string()]);
    assert_eq!(code, 1);
    assert!(!pcap.with_extension("results.json").exists());
}

/// the driver hands reports over in batches, each a few milliseconds late by a different
/// amount. the live view must still move at the clock's pace: its edge trails the clock by the
/// longest recent delay instead of jumping with every batch
#[test]
fn the_live_view_moves_at_the_clocks_pace_whatever_each_batch_delay() {
    use crate::capture::session::Delivery;
    let mut delivery = Delivery {
        delay_us: 4_000,
        since_us: 0,
    };
    let mut edges = Vec::new();
    for (now, delay) in [
        (10_000, 9_000),
        (20_000, 3_000),
        (30_000, 7_000),
        (40_000, 2_000),
    ] {
        delivery.observe(delay, now);
        edges.push(now as i64 + 5_000 - delivery.delay_us);
    }
    // the longest delay holds, so the edge advances exactly as the clock does
    assert_eq!(delivery.delay_us, 9_000);
    assert!(edges.windows(2).all(|w| w[1] - w[0] == 10_000), "{edges:?}");
    // two seconds on, the current delay takes over once
    delivery.observe(2_000, 2_020_000);
    assert_eq!(delivery.delay_us, 2_000);
}

#[test]
fn only_the_chosen_device_going_back_in_time_refuses_its_capture() {
    use crate::testkit::traffic::{Descriptors, Packet, TempDir, Traffic, mouse_payload, ticks};
    let folder = TempDir::new("wire-regression");
    std::fs::create_dir_all(folder.path()).unwrap();
    let read = |behind: u16, endpoint: &str| {
        let mut traffic = Traffic::default();
        traffic.describe(0, 1, 2, &Descriptors::mouse(0x373b, 0x11d9));
        traffic.describe(0, 1, 3, &Descriptors::mouse(0x046d, 0xc08b));
        for device in [2, 3] {
            traffic.extend(
                ticks(1000, 1_000_000, 300)
                    .map(move |t| (t, Packet::report(1, device, 0x81, &mouse_payload(1, -1, 7)))),
            );
        }
        // written last, stamped before what that device already sent
        traffic.add(
            1_100_000,
            Packet::report(1, behind, 0x81, &mouse_payload(1, -1, 7)),
        );
        let path = folder.path().join(format!(
            "behind-{behind}-{}.pcap",
            endpoint.replace(':', "-")
        ));
        std::fs::write(&path, traffic.pcap().bytes()).unwrap();
        // the saved selection is device 2, whatever the comparison asks for
        let mut sidecar = crate::testkit::fixtures::sidecar("complete");
        sidecar["selection"] = serde_json::json!({"bus": 1, "address": 2, "endpoint": 129});
        std::fs::write(path.with_extension("json"), sidecar.to_string()).unwrap();
        read_wire(
            &path,
            Some(crate::testkit::traffic::mouse_decoder()),
            Some(endpoint),
            None,
            &AtomicBool::new(false),
        )
        .map(|_| ())
    };
    assert!(read(3, "1:2:129").is_ok());
    assert!(
        read(2, "1:2:129")
            .unwrap_err()
            .to_string()
            .contains("regressed")
    );
    // picking the healthy device over a saved one whose clock went back reads it
    assert!(read(2, "1:3:129").is_ok());
}

#[test]
fn a_crop_keeps_every_comparison_inside_it() {
    use crate::testkit::traffic::{Descriptors, Packet, TempDir, Traffic, mouse_payload, ticks};
    let folder = TempDir::new("compare-crop");
    std::fs::create_dir_all(folder.path()).unwrap();
    let path = folder.path().join("cropped.pcap");
    let mut traffic = Traffic::default();
    traffic.describe(0, 1, 2, &Descriptors::mouse(0x373b, 0x11d9));
    traffic.extend(
        ticks(1000, 1_000_000, 2000)
            .map(|t| (t, Packet::report(1, 2, 0x81, &mouse_payload(1, -1, 7)))),
    );
    traffic.add(2_500_000, Packet::disconnect(1, 2, 0x81));
    std::fs::write(&path, traffic.pcap().bytes()).unwrap();
    std::fs::write(
        path.with_extension("json"),
        crate::testkit::fixtures::sidecar("complete").to_string(),
    )
    .unwrap();
    let crop = Selection {
        from_us: 1_200_000,
        to_us: 2_000_000,
    };
    let read = read_wire(
        &path,
        Some(crate::testkit::traffic::mouse_decoder()),
        None,
        Some(crop),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        (read.first, read.last, read.reports),
        (1_200_000, 2_000_000, 801)
    );

    let source = |crop| Source {
        path: path.clone(),
        decoder: Some(crate::testkit::traffic::mouse_decoder()),
        endpoint: None,
        crop,
    };
    let compare_over = |crop, range| {
        independent_selected(
            &[source(crop)],
            &[source(crop)],
            "texture19",
            range,
            Config {
                bootstrap: 0,
                permutations: 0,
                ..Config::default()
            },
            &AtomicBool::new(false),
            |_| {},
        )
    };
    let compare = |crop| compare_over(crop, None);
    // the whole recording reaches the unplug, the crop ends before it
    assert!(
        compare(None)
            .err()
            .is_some_and(|e| e.to_string().contains("disconnected"))
    );
    let result = compare(Some(crop)).unwrap();
    // 800 ms of 160 ms windows
    assert_eq!(result.real_features.len(), 5);
    assert_eq!(
        result.crops[&crate::evidence::reader::layout::recording_id(&path)],
        Some(crop)
    );
    // a range longer than the crop stops at its end, short of the unplug
    let longer = compare_over(Some(crop), Some([0.0, 1.5])).unwrap();
    assert_eq!(longer.real_features.len(), 5);
    assert_eq!(
        result.metadata["sources"][0]["crop_us"],
        serde_json::json!(crop)
    );
}

#[test]
fn a_paired_crop_leaves_the_input_outside_it_unused() {
    let folder = crate::testkit::traffic::TempDir::new("paired-crop");
    let fixture = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures"));
    let path = crate::testkit::fixtures::recorded(
        folder.path(),
        "mouse-1000hz.pcap",
        &serde_json::from_slice(&std::fs::read(fixture.join("mouse-1000hz.json")).unwrap())
            .unwrap(),
    );
    let bounds = read_wire(&path, None, None, None, &AtomicBool::new(false))
        .map(|w| [w.first, w.last])
        .unwrap();
    let crop = Selection {
        from_us: bounds[0] + 100_000,
        to_us: bounds[1] - 100_000,
    };
    // the box saw something else than the wire before the crop, nowhere after it
    let mut text = String::from("# paired_shift_us=0\nbox_us,dx,dy,dz\n");
    crate::capture::pcap::read_pcap(&std::fs::read(&path).unwrap()[..], |record| {
        if let crate::capture::pcap::Record::Packet {
            event: Some(crate::capture::pcap::Event::Report(report)),
            ..
        } = record
        {
            let (x, y) = crate::capture::pcap::motion(report.bytes()).unwrap();
            let x = if crop.contains(report.t_us) {
                x
            } else {
                x * 3 + 1
            };
            text.push_str(&format!("{},{x},{y},0\n", report.t_us));
        }
    })
    .unwrap();
    std::fs::write(path.with_extension("input.csv"), text).unwrap();
    let config = Config {
        bootstrap: 0,
        permutations: 0,
        width_ms: 8,
        ..Config::default()
    };
    let compare_over = |crop, range| {
        let source = Source {
            path: path.clone(),
            decoder: None,
            endpoint: None,
            crop,
        };
        paired_selected(
            &source,
            config.clone(),
            range,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap()
    };
    let compare = |crop| compare_over(crop, None);
    assert!(compare(None).w1.iter().any(|v| *v != 0.0));
    let cropped = compare(Some(crop));
    assert!(cropped.w1.iter().all(|v| *v == 0.0));
    assert_eq!(cropped.metadata["crop_us"], serde_json::json!(crop));
    // the crop's own first and last report
    let inside = read_wire(&path, None, None, Some(crop), &AtomicBool::new(false))
        .map(|w| [w.first, w.last])
        .unwrap();
    assert!(inside[0] >= crop.from_us && inside[0] < crop.from_us + 2000);
    assert_eq!(
        cropped.metadata["selected_range_us"],
        serde_json::json!(inside)
    );
    // a range past the crop's end reads no input after it either
    let longer = compare_over(
        Some(crop),
        Some([0.0, (bounds[1] - inside[0]) as f64 / 1e6]),
    );
    assert!(longer.w1.iter().all(|v| *v == 0.0));
    assert_eq!(
        longer.metadata["selected_range_us"],
        serde_json::json!([inside[0], crop.to_us])
    );
}

mod digests;

/// a layout picked for the recording stays its layout even when the mouse's own descriptor only
/// arrives after its last report
#[test]
fn a_chosen_layout_outlasts_a_descriptor_that_comes_last() {
    use crate::testkit::traffic::{Descriptors, Packet, TempDir, Traffic, mouse_payload, ticks};
    let folder = TempDir::new("late-descriptor");
    std::fs::create_dir_all(folder.path()).unwrap();
    let path = folder.path().join("late.pcap");
    let mut traffic = Traffic::default();
    traffic.describe(0, 1, 2, &Descriptors::mouse(0x373b, 0x11d9));
    traffic.extend(
        ticks(1000, 1_000_000, 200)
            .map(|t| (t, Packet::report(1, 2, 0x81, &mouse_payload(1, -1, 7)))),
    );
    traffic.hid_descriptor(1_500_000, 1, 2, 0, 9);
    std::fs::write(&path, traffic.pcap().bytes()).unwrap();
    let read = |decoder| read_wire(&path, decoder, None, None, &AtomicBool::new(false));
    // the captured descriptor alone gives a layout
    assert!(read(None).is_ok());
    let refused = read(Some(Decoder::Unavailable)).err().unwrap().to_string();
    assert!(refused.contains("no movement layout"), "{refused}");
}

/// a result keeps which of its recordings didn't end complete, with how they ended, and only those
#[test]
fn a_result_names_the_recordings_that_did_not_end_complete() {
    let folder = crate::testkit::traffic::TempDir::new("not-complete");
    let fixture = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures"));
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture.join("mouse-1000hz.json")).unwrap()).unwrap();
    let whole = crate::testkit::fixtures::recorded(folder.path(), "mouse-1000hz.pcap", &saved);
    let short = folder.path().join("short.pcap");
    std::fs::copy(&whole, &short).unwrap();
    let mut cut = saved.clone();
    cut["outcome"] = "interrupted".into();
    cut["stop_reason"] = "app_exit".into();
    std::fs::write(short.with_extension("json"), cut.to_string()).unwrap();
    let config = Config {
        bootstrap: 0,
        permutations: 0,
        ..Config::default()
    };
    let result = independent(
        std::slice::from_ref(&whole),
        std::slice::from_ref(&short),
        "texture19",
        None,
        config,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    let listed: Vec<_> = result
        .not_complete
        .iter()
        .map(|n| (n.path.as_str(), n.kind.as_str()))
        .collect();
    assert_eq!(
        listed,
        [(short.display().to_string().as_str(), "interrupted")]
    );
    assert!(result.not_complete[0].message.is_some());
}

/// a recording that ended whole but has no box input beside it can't be paired, and says why
#[test]
fn a_paired_comparison_refuses_a_recording_without_its_box_input() {
    let folder = crate::testkit::traffic::TempDir::new("no-input");
    let fixture = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures"));
    let pcap = crate::testkit::fixtures::recorded(
        folder.path(),
        "mouse-1000hz.pcap",
        &serde_json::from_slice(&std::fs::read(fixture.join("mouse-1000hz.json")).unwrap())
            .unwrap(),
    );
    let error = paired(
        &pcap,
        None,
        Config::default(),
        None,
        &AtomicBool::new(false),
        |_| {},
    )
    .err()
    .unwrap()
    .to_string();
    assert!(error.contains("no physical input"), "{error}");
}

/// a result names every recording it read by id, with the sha256 of each file it read, which code
/// made it, and the group labels the folds used: the path, as the reference grouping always has
#[test]
fn a_result_records_what_it_read_and_what_read_it() {
    use sha2::Digest;
    let folder = crate::testkit::traffic::TempDir::new("provenance");
    let pcap = digests::paired_fixture(folder.path());
    let sha = |path: &Path| format!("{:x}", sha2::Sha256::digest(std::fs::read(path).unwrap()));
    let config = Config {
        bootstrap: 0,
        permutations: 0,
        ..Config::default()
    };
    let paired = paired(
        &pcap,
        None,
        config.clone(),
        None,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    let id = crate::evidence::reader::layout::recording_id(&pcap);
    assert_eq!(
        (&paired.baseline, &paired.candidate),
        (&vec![id.clone()], &vec![id.clone()])
    );
    let source = &paired.sources[0];
    assert_eq!(source.id, id);
    assert_eq!(source.digests["pcap"], sha(&pcap));
    assert_eq!(
        source.digests["input.csv"],
        sha(&pcap.with_extension("input.csv"))
    );
    assert_eq!(source.digests["json"], sha(&pcap.with_extension("json")));
    assert_eq!(paired.tick_us, 1000);
    assert_eq!(paired.implementation.app_version, env!("CARGO_PKG_VERSION"));
    assert!(!paired.implementation.revision.is_empty());

    let independent = independent(
        std::slice::from_ref(&pcap),
        std::slice::from_ref(&pcap),
        "texture19",
        None,
        config,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(independent.sources.len(), 2);
    assert!(
        independent
            .sources
            .iter()
            .all(|s| s.digests.keys().eq(["json", "pcap"]))
    );
    let label = format!("str:{}", pcap.display());
    let groups = &independent.metadata["source_groups"]["baseline"];
    assert!(
        groups.as_array().unwrap().iter().all(|g| g == &label),
        "{groups}"
    );
}
