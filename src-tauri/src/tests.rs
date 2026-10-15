#[cfg(feature = "developer-tools")]
use crate::commands::experiment::*;
use crate::{
    app::{pump::*, state::*},
    commands::{analysis::*, runs::*, session::*, targets::*},
    library, recording,
    recording::commit::*,
};
use motion_bench::sync::LockExt;

#[cfg(feature = "developer-tools")]
use crate::controller::experiment;

use crate::library::{Library, Run, import};
use crate::recording::Recording;
use motion_bench::{
    analysis::{
        classify::Config,
        compare::{ResultRow, Selection},
        views::plot::{Plot, Views},
    },
    capture::{
        pcap::{self, Endpoint},
        session::{self, Closing, Device},
    },
    coordination::readiness,
    evidence::{
        reader::recording::open_recording,
        reason::{Reason, StopReason},
    },
    hardware::medius,
    signal::stats::Key,
};
use serde_json::{Value, json};
#[cfg(feature = "developer-tools")]
use std::time::Duration;
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    sync::Arc,
};

/// the crate's mock box as the link holds it now: an anybox device over it, its poller left
/// off so the test drives the box itself
fn boxed_mock(mock: anybox::medius::MockBox) -> anybox::Device {
    let (device, _poller) = anybox::Device::mock_medius_manual(
        mock,
        anybox::ClickTiming::Fixed(std::time::Duration::from_millis(5)),
    )
    .expect("a mock box opens");
    device
}

/// the 1 kHz capture the library tests import
fn fixture_pcap() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../tests/fixtures/mouse-1000hz.pcap"
    ))
}
/// the 1 kHz fixture recording as `name` in `folder`, its metadata beside it
fn recorded(folder: &Path, name: &str) -> PathBuf {
    let pcap = folder.join(name);
    std::fs::copy(fixture_pcap(), &pcap).unwrap();
    std::fs::copy(
        fixture_pcap().with_extension("json"),
        pcap.with_extension("json"),
    )
    .unwrap();
    pcap
}
/// the metadata of a recording of the test traffic
fn sidecar(outcome: &str) -> Value {
    json!({
        "schema": motion_bench::evidence::commit::RECORDING_SCHEMA,
        "outcome": outcome,
        "reason": null,
        "stop_reason": "record_stop",
        "health": {"error": null, "pairing_valid": true},
        "participants": [],
        "decoder": motion_bench::testkit::traffic::mouse_decoder(),
    })
}
/// an endpoint row is named by what Windows calls it, and marked when the connected box is the
/// one cloning it
#[test]
fn a_device_row_is_named_and_marked_by_its_box() {
    let mut device = Device {
        id: "1:30:129".into(),
        bus: 1,
        address: 30,
        endpoint: 129,
        vid: Some(0x373b),
        pid: Some(0x11d9),
        kind: "mouse".into(),
        interval: Some(1),
        decoder: "Unavailable".into(),
        reports: 0,
        name: None,
        box_name: None,
    };
    let names = HashMap::from([(
        (0x373b, 0x11d9, 0u8),
        "Wireless mouse 8k dongle-L".to_string(),
    )]);
    assert_eq!(
        windows_name(&device, Some(0), &names).as_deref(),
        Some("Wireless mouse 8k dongle-L")
    );
    // the same ids on another interface are not the collection Windows named
    assert_eq!(windows_name(&device, Some(1), &names), None);
    // and an endpoint whose ids the capture never saw has no name either
    device.vid = None;
    assert_eq!(windows_name(&device, Some(0), &names), None);
    device.vid = Some(0x373b);

    let mut firmware = medius::Snapshot {
        connected: true,
        family: "Medius".into(),
        name: "medius".into(),
        clone_vid: Some(0x373b),
        clone_pid: Some(0x11d9),
        clone_product: "Wireless mouse 8k dongle-L".into(),
        ..medius::Snapshot::default()
    };
    assert_eq!(
        cloned_by(&device, &firmware).as_deref(),
        Some("Medius - medius")
    );
    // a box clones one device, not every device on the wire
    firmware.clone_pid = Some(0x11c0);
    assert_eq!(cloned_by(&device, &firmware), None);
    // a box that is not connected clones nothing
    firmware.clone_pid = Some(0x11d9);
    firmware.connected = false;
    assert_eq!(cloned_by(&device, &firmware), None);
    firmware.connected = true;
    // any box that says what it presents marks it, by its kind when it has no name of its
    // own, and once when its name is its kind and model
    firmware.name = " ".into();
    assert_eq!(cloned_by(&device, &firmware).as_deref(), Some("Medius"));
    firmware.family = "Ferrum".into();
    firmware.name = "Ferrum KM".into();
    assert_eq!(cloned_by(&device, &firmware).as_deref(), Some("Ferrum KM"));
    firmware.family = " ".into();
    firmware.name = " ".into();
    assert_eq!(cloned_by(&device, &firmware), None);
    firmware.name = "medius".into();

    // the box's own name for the clone stands in when Windows named nothing
    assert_eq!(
        clone_product(&firmware).as_deref(),
        Some("Wireless mouse 8k dongle-L")
    );
    firmware.clone_product = " ".into();
    assert_eq!(clone_product(&firmware), None);
}

/// the endpoint a press showed up on stays the box's while the box stays, and is forgotten
/// once it goes away or connects again, since the mouse behind it may have changed
#[test]
fn the_box_endpoint_lasts_as_long_as_the_box_link() {
    let key = Key {
        bus: 1,
        device: 2,
        endpoint: 0x81,
    };
    let data = session::Data::new(true);
    let mut firmware = medius::Snapshot {
        connected: true,
        generation: 4,
        ..medius::Snapshot::default()
    };
    // no press since, so what was found holds
    assert_eq!(
        box_endpoint(Some((key, 4)), &data, &firmware, Some(1)),
        Some((key, 4))
    );
    firmware.generation = 5;
    assert_eq!(
        box_endpoint(Some((key, 4)), &data, &firmware, Some(1)),
        None
    );
    firmware.generation = 4;
    firmware.connected = false;
    assert_eq!(
        box_endpoint(Some((key, 4)), &data, &firmware, Some(1)),
        None
    );
}

/// the Windows names are read again only when the endpoints on the wire change, so a device
/// that came or went is noticed without walking every HID device on each snapshot
#[test]
fn the_endpoints_that_need_a_name_are_the_ones_on_the_wire() {
    let key = |bus, device, endpoint| Key {
        bus,
        device,
        endpoint,
    };
    let endpoint = |interface| Endpoint {
        interface,
        address: 0x81,
        interval: 1,
        max_packet: 8,
        class: 3,
        subclass: 1,
        protocol: 2,
    };
    let mut endpoints = BTreeMap::new();
    endpoints.insert(key(1, 30, 129), endpoint(0));
    endpoints.insert(key(1, 12, 129), endpoint(0));
    let mut names = BTreeMap::new();
    names.insert((1, 30), (0x373b, 0x11d9));
    names.insert((1, 12), (0x373b, 0x11c0));
    assert_eq!(
        name_keys(&endpoints, &names),
        vec![(0x373b, 0x11c0, 0), (0x373b, 0x11d9, 0)]
    );
    // an endpoint whose address the capture has no ids for is left out, and a second endpoint
    // of the same device is its own interface
    endpoints.insert(key(1, 31, 129), endpoint(0));
    endpoints.insert(key(1, 30, 130), endpoint(1));
    assert_eq!(
        name_keys(&endpoints, &names),
        vec![
            (0x373b, 0x11c0, 0),
            (0x373b, 0x11d9, 0),
            (0x373b, 0x11d9, 1)
        ]
    );
    assert!(name_keys(&BTreeMap::new(), &names).is_empty());
}

/// a run with the box's input gets the Relay tab with its plot; the box saw what the wire
/// carried, so every moving report pairs and the counts are conserved
#[test]
fn a_paired_run_plots_its_relay() {
    let folder = tempfile::tempdir().unwrap();
    let pcap = paired_recording(folder.path());
    let mut run = import(&pcap).unwrap();
    assert!(run.paired);
    assert_eq!(
        (
            run.refusals.comparison.as_deref(),
            run.refusals.paired.as_deref()
        ),
        (None, None)
    );
    let plot = wire_plot(&run, None, None, true).unwrap();
    assert!(
        wire_plot(&run, None, None, false)
            .unwrap()
            .tabs
            .relay
            .is_none()
    );
    let relay = plot.tabs.relay.unwrap();
    assert_eq!(relay.box_counts, relay.wire_counts);
    assert_eq!((relay.box_only, relay.wire_only), (0, 0));
    assert!(relay.matched > 300);
    assert!(relay.conserved.iter().all(|c| c[1] == 0.0 && c[2] == 0.0));
    let input = input_plot(&run, None, None).unwrap();
    assert_eq!(input.readouts.moving, plot.readouts.moving);
    run.paired = false;
    assert!(
        wire_plot(&run, None, None, true)
            .unwrap()
            .tabs
            .relay
            .is_none()
    );
}
/// the run's whole plot and a crop from 100 ms in to `end` before its last report
fn inner(run: &Run, end: u64) -> (Plot, Selection) {
    let whole = wire_plot(run, None, None, false).unwrap();
    let crop = Selection {
        from_us: whole.from_us + 100_000,
        to_us: whole.to_us - end,
    };
    (whole, crop)
}
/// the app's state with `run` as its only listed run
fn listing(folder: &Path, run: &Run) -> Shared {
    let (controller, _) = recording::hotkey::Controller::for_test(folder);
    let state = Shared::new(controller.clone(), Bench::new(controller)).in_folder(folder);
    state.library().add(run.clone()).unwrap();
    state
}
/// `request` started and waited for, at most 30 s
fn analyzed(state: &Shared, request: AnalysisRequest) -> (u64, library::jobs::Jobs) {
    let id = analyze_inner(request, state).unwrap();
    let jobs = state.bench.locked().jobs.clone();
    let started = std::time::Instant::now();
    while jobs.running() && started.elapsed().as_secs() < 30 {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    (id, jobs)
}
/// the 1 kHz fixture with the box's input beside it, the same counts the wire carried
fn paired_recording(folder: &Path) -> PathBuf {
    let pcap = recorded(folder, "paired.pcap");
    let mut rows = String::from("# paired_shift_us=0\nbox_us,dx,dy,dz\n");
    let bytes = std::fs::read(&pcap).unwrap();
    motion_bench::capture::pcap::read_pcap(&bytes[..], |record| {
        if let motion_bench::capture::pcap::Record::Packet {
            event: Some(motion_bench::capture::pcap::Event::Report(report)),
            ..
        } = record
            && let Some((x, y)) = motion_bench::capture::pcap::motion(report.bytes())
        {
            rows.push_str(&format!("{},{x},{y},0\n", report.t_us));
        }
    })
    .unwrap();
    std::fs::write(pcap.with_extension("input.csv"), rows).unwrap();
    pcap
}
/// a crop is kept in the library, read by every view, refused when it would be empty or while
/// an analysis runs, and puts away the results it no longer matches; the recording's files
/// never change
#[test]
fn a_crop_lives_in_the_library_and_the_views_read_only_it() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path();
    let pcap = recorded(folder, "cropped.pcap");
    let files =
        || ["pcap", "json"].map(|extension| std::fs::read(pcap.with_extension(extension)).unwrap());
    let before = files();
    let mut run = import(&pcap).unwrap();
    let (whole, crop) = inner(&run, 100_000);
    let row = |crop: Option<Selection>| {
        let mut row = ResultRow {
            crops: BTreeMap::from([(run.id.clone(), crop)]),
            ..library::jobs::tests::row("texture19")
        };
        row.identify();
        row
    };
    run.result = Some(row(None));
    let state = listing(folder, &run);
    let set = |crop| crop_run(&state, &run.id, crop);

    // too short, and wholly after the last report
    let short = Selection {
        from_us: crop.from_us,
        to_us: crop.from_us + 5_000,
    };
    let after = Selection {
        from_us: whole.to_us + 1_000_000,
        to_us: whole.to_us + 2_000_000,
    };
    assert!(set(Some(short)).is_err());
    assert!(set(Some(after)).is_err());
    let jobs = state.bench.locked().jobs.clone();
    let (job, _) = jobs.start().unwrap();
    assert!(set(Some(crop)).unwrap_err().contains("analysis"));
    library::jobs::spawn(
        jobs,
        job,
        |_| Ok(library::jobs::tests::row("done")),
        |_| None,
    )
    .unwrap()
    .join()
    .unwrap();
    assert!(state.run(&run.id).unwrap().crop.is_none());

    set(Some(crop)).unwrap();
    let cropped = state.run(&run.id).unwrap();
    assert_eq!(cropped.crop, Some(crop));
    // the result read the whole recording, it no longer describes it
    assert!(cropped.result.is_none());
    let plot = wire_plot(&cropped, None, None, false).unwrap();
    assert!(plot.from_us >= crop.from_us && plot.from_us < crop.from_us + 2_000);
    assert!(plot.to_us <= crop.to_us && plot.reports < whole.reports);
    // a range counts from the crop's first report
    let ranged = wire_plot(&cropped, Some([0.0, 0.2]), None, false).unwrap();
    assert_eq!(ranged.from_us, plot.from_us);

    // restored and imported again it keeps its crop, and a saved result keeps showing only
    // while it was read with the same crop
    for (saved, shown) in [(None, false), (Some(crop), true)] {
        state
            .library()
            .attach(&row(saved), std::slice::from_ref(&run.id))
            .unwrap();
        let library = Library::new(folder.to_path_buf());
        let warnings = library.restore();
        assert!(warnings.is_empty(), "{warnings:?}");
        let runs = library.runs();
        assert_eq!(runs[0].crop, Some(crop));
        assert_eq!(runs[0].result.is_some(), shown);
        let again = library.add(import(&pcap).unwrap()).unwrap();
        assert_eq!((again.crop, again.result.is_some()), (Some(crop), shown));
    }

    // the whole recording again is no crop
    set(Some(Selection {
        from_us: 0,
        to_us: u64::MAX,
    }))
    .unwrap();
    assert!(state.run(&run.id).unwrap().crop.is_none());
    assert_eq!(files(), before);
}
/// plots stop at the crop's end whatever range is asked, the comparisons and the response
/// delay read only the crop, and a run read again after its recording keeps it
#[test]
fn every_analysis_of_a_run_reads_its_crop() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path();
    let pcap = paired_recording(folder);
    let mut run = import(&pcap).unwrap();
    let (whole, crop) = inner(&run, 300_000);
    run.crop = Some(crop);
    run.name = "cropped".into();
    let state = listing(folder, &run);

    let past = Some([0.0, 5.0]);
    assert_eq!(wire_plot(&run, past, None, true).unwrap().to_us, crop.to_us);
    assert_eq!(input_plot(&run, past, None).unwrap().to_us, crop.to_us);
    // the dialog choosing a crop sees the whole recording
    let uncropped = plotted(&state, &run.id, Some(true)).unwrap();
    assert_eq!(
        wire_plot(&uncropped, None, None, false).unwrap().from_us,
        whole.from_us
    );
    assert_eq!(plotted(&state, &run.id, None).unwrap().crop, Some(crop));

    for paired in [false, true] {
        let request = AnalysisRequest {
            matching: false,
            question: None,
            baseline: if paired {
                Vec::new()
            } else {
                vec![run.id.clone()]
            },
            candidate: vec![run.id.clone()],
            paired,
            panel: "texture19".into(),
            config: Config {
                bootstrap: 0,
                permutations: 0,
                ..Config::default()
            },
            range: None,
        };
        let (id, jobs) = analyzed(&state, request);
        let result = jobs
            .result(id)
            .unwrap_or_else(|e| panic!("{e}: {:?}", jobs.current().unwrap().error));
        assert_eq!(result.crops[&run.id], Some(crop), "paired {paired}");
    }

    // past the last report a crop leaves the response delay no movement to read
    std::fs::write(
        pcap.with_extension("vision.json"),
        serde_json::to_vec(&motion_bench::targets::vision::Evidence {
            schema: "motion-bench.vision.v1".into(),
            config: motion_bench::targets::vision::Config {
                scenario: "stationary target".into(),
                ..Default::default()
            },
            model_sha256: "x".into(),
            provider: "CUDA".into(),
            pipeline: "synthetic".into(),
            inference: None,
            complete: true,
            error: None,
            clock: Some(motion_bench::targets::vision::Clock {
                initial_bracket_us: 0.0,
                qpc_origin: 0,
                qpc_frequency: 10_000_000,
                epoch_origin_us: 0,
            }),
        })
        .unwrap(),
    )
    .unwrap();
    let late = Run {
        crop: Some(Selection {
            from_us: whole.to_us + 1_000_000,
            to_us: whole.to_us + 2_000_000,
        }),
        ..run.clone()
    };
    assert!(
        calibrate(&late)
            .err()
            .unwrap()
            .contains("no decoded movement")
    );
    assert!(
        !calibrate(&uncropped)
            .err()
            .unwrap()
            .contains("no decoded movement")
    );

    // read again after its recording it keeps its name and crop, one not listed stays out
    state.library().relist(import(&pcap).unwrap()).unwrap();
    let again = state.run(&run.id).unwrap();
    assert_eq!((again.name.as_str(), again.crop), ("cropped", Some(crop)));
    let elsewhere = tempfile::tempdir().unwrap();
    let none = Library::new(elsewhere.path().to_path_buf());
    none.relist(import(&pcap).unwrap()).unwrap();
    assert!(none.runs().is_empty());
}
#[test]
fn a_recording_request_without_usb_monitoring_fails_without_a_phantom_recording() {
    let folder = tempfile::tempdir().unwrap();
    let (controller, _) = recording::hotkey::Controller::for_test(folder.path());
    let mut bench = Bench::new(controller.clone());
    bench.library_loaded = true;
    let state = Shared::new(controller, bench);
    for _ in 0..2 {
        let result = recording::toggle(
            &state,
            motion_bench::capture::clock::Anchor::take(),
            Some(true),
        );
        let snapshot = state.bench.locked().snapshot(0, Views::Readouts);
        let blocker = snapshot.readiness.record.unwrap();
        assert_eq!(blocker.need, readiness::Need::CaptureStopped);
        assert_eq!(result.unwrap_err(), blocker.message);
        assert!(snapshot.recording.is_none());
        assert!(!snapshot.recording_hotkey.busy);
        assert_eq!(snapshot.recording_hotkey.error.unwrap(), blocker.message);
    }
    // nothing may appear, not even the settings file
    assert_eq!(std::fs::read_dir(folder.path()).unwrap().count(), 0);
}
#[test]
fn firmware_names_with_control_characters_reach_the_snapshot() {
    let product = "VXE\nline \"two\" \u{1}";
    let mock = anybox::medius::MockBox::new().with_device_info(anybox::medius::DeviceInfo {
        vid: 0x373b,
        pid: 0x11d9,
        bcd_device: 0,
        bcd_usb: 0x0200,
        has_serial: false,
        has_bos: false,
        kind: anybox::medius::DeviceKind::Mouse,
        product: product.into(),
    });
    let folder = tempfile::tempdir().unwrap();
    let (controller, _) = recording::hotkey::Controller::for_test(folder.path());
    let mut bench = Bench::new(controller);
    bench.link = medius::Link::with_device(boxed_mock(mock));
    let started = std::time::Instant::now();
    // connected is set before the box is asked who it clones, so wait for the answer
    while bench.link.snapshot().clone.is_empty() && started.elapsed().as_secs() < 8 {
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    let shown = bench.snapshot(0, Views::Readouts).box_;
    assert_eq!(shown["link"], "connected");
    assert_eq!(shown["medius"]["clone"], format!("373B:11D9 {product}"));
}
/// a box that does not answer keeps disarming waiting, never the commands and snapshots that
/// need the bench lock
#[cfg(feature = "developer-tools")]
#[test]
fn a_stuck_box_leaves_the_bench_lock_free_while_disarming() {
    let (_mock, _link, worker, source, _focus) = experiment::tests::setup();
    worker
        .arm(motion_bench::controller::linear::Config::default(), source)
        .unwrap();
    let worker = Arc::new(worker);
    let folder = tempfile::tempdir().unwrap();
    let (controller, _) = recording::hotkey::Controller::for_test(folder.path());
    let mut bench = Bench::new(controller.clone());
    bench.experiment = Some(worker.clone());
    let state = Shared::new(controller, bench);
    let stuck = worker.hold_box();
    let disarming = state.clone();
    let disarm = std::thread::spawn(move || disarm_experiment(&disarming));
    let start = std::time::Instant::now();
    while worker.snapshot().armed {
        assert!(start.elapsed() < Duration::from_secs(5), "never disarmed");
        std::thread::sleep(Duration::from_millis(2));
    }
    // disarming now waits on the box; the lock is free all the same
    let asked = std::time::Instant::now();
    while state.bench.try_lock().is_err() {
        assert!(
            asked.elapsed() < Duration::from_secs(1),
            "the bench lock is held while the box is stuck"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    state.bench.locked().snapshot(0, Views::Readouts);
    assert!(!disarm.is_finished());
    drop(stuck);
    disarm.join().unwrap().unwrap();
}
/// a stop on its way owns what it stops: starting capture, target capture and the aim, and
/// disarming, are refused until it is done
#[test]
fn commands_are_refused_while_a_stop_is_on_its_way() {
    let folder = tempfile::tempdir().unwrap();
    let (controller, _) = recording::hotkey::Controller::for_test(folder.path());
    let state = Shared::new(controller.clone(), Bench::new(controller.clone()));
    let stopping = Recording::claim(&state).unwrap();
    let refused = |result: Result<(), String>| result.unwrap_err().contains("still finishing");
    assert!(refused(stop_screen_capture(&state)));
    assert!(refused(start_screen_capture(&state)));
    assert!(refused(start_capture_at(&state, "USBPcap9".into())));
    #[cfg(feature = "developer-tools")]
    {
        assert!(refused(arm_experiment(&state)));
        assert!(refused(disarm_experiment(&state)));
    }
    drop(stopping);
    assert!(stop_screen_capture(&state).is_ok());
}
/// a build without the developer tools has no aim controller: nothing to arm, nothing in the
/// recording, and the ui is told so
#[cfg(not(feature = "developer-tools"))]
#[test]
fn a_normal_build_has_no_aim_controller() {
    let folder = tempfile::tempdir().unwrap();
    let (controller, _) = recording::hotkey::Controller::for_test(folder.path());
    let mut bench = Bench::new(controller);
    assert!(!bench.snapshot(0, Views::Readouts).developer_tools);
    assert_eq!(bench.aim_provenance(), json!({"enabled": false}));
    assert!(!bench.aim_armed());
    assert_eq!(
        bench.aim().begin(&folder.path().join("run.pcap")),
        Ok(false)
    );
    assert!(bench.aim().disarm().is_ok() && bench.aim_error().is_none());
    assert!(bench.take_aim().stop().is_none());
}
#[test]
fn a_paired_comparison_refuses_anything_but_one_recording_before_starting_a_job() {
    let folder = tempfile::tempdir().unwrap();
    let fixture = recorded(folder.path(), "paired.pcap");
    let mut first = import(&fixture).unwrap();
    first.paired = true;
    let mut second = first.clone();
    second.id = "second".into();
    let (controller, _) = recording::hotkey::Controller::for_test(folder.path());
    let state = Shared::new(controller.clone(), Bench::new(controller)).in_folder(folder.path());
    state.library().add(first.clone()).unwrap();
    state.library().add(second.clone()).unwrap();
    let before: Vec<_> = std::fs::read_dir(folder.path()).unwrap().collect();
    for candidate in [vec![], vec![first.id.clone(), second.id.clone()]] {
        let request = AnalysisRequest {
            matching: false,
            question: None,
            baseline: Vec::new(),
            candidate,
            paired: true,
            panel: "texture19".into(),
            config: Config::default(),
            range: None,
        };
        assert!(analyze_inner(request, &state).is_err());
        let bench = state.bench.locked();
        assert!(bench.jobs.current().is_none());
    }
    assert_eq!(
        std::fs::read_dir(folder.path()).unwrap().count(),
        before.len()
    );
    // the rule is for paired mode only, an independent comparison takes any number
    for (paired, candidate) in [
        (true, vec![first.id.clone()]),
        (false, vec![first.id.clone(), second.id.clone()]),
    ] {
        let request = AnalysisRequest {
            matching: false,
            question: None,
            baseline: vec![first.id.clone()],
            candidate,
            paired,
            panel: "texture19".into(),
            config: Config::default(),
            range: None,
        };
        assert!(check_request(&request).is_ok(), "paired {paired}");
    }

    // a baseline left selected from independent mode no longer exists, paired mode ignores it
    let request = AnalysisRequest {
        matching: false,
        question: None,
        baseline: vec!["gone".into()],
        candidate: vec![first.id.clone()],
        paired: true,
        panel: "texture19".into(),
        config: Config::default(),
        range: None,
    };
    let (id, jobs) = analyzed(&state, request);
    let job = jobs.current().unwrap();
    assert_eq!(job.id, id);
    assert_ne!(job.error.as_deref(), Some("selected run is unavailable"));
}
#[test]
fn an_export_names_its_result_and_never_falls_back_to_another() {
    let source = |text: &str| serde_json::from_str::<ResultSource>(text);
    assert!(matches!(
        source(r#"{"kind":"job","id":3}"#),
        Ok(ResultSource::Job(3))
    ));
    assert!(
        matches!(source(r#"{"kind":"result","id":"abc"}"#), Ok(ResultSource::Result(id)) if id == "abc")
    );
    assert!(source(r#"{"kind":"job","id":"3"}"#).is_err());

    let folder = tempfile::tempdir().unwrap();
    let (controller, _) = recording::hotkey::Controller::for_test(folder.path());
    let mut bench = Bench::new(controller);
    bench.library = Arc::new(Library::new(folder.path().to_path_buf()));
    let finished = |panel: &'static str| {
        let (id, _) = bench.jobs.start().unwrap();
        library::jobs::spawn(
            bench.jobs.clone(),
            id,
            move |_| Ok(library::jobs::tests::row(panel)),
            |_| None,
        )
        .unwrap()
        .join()
        .unwrap();
        id
    };
    let (first, second) = (finished("first"), finished("second"));
    let fixture = fixture_pcap();
    let run = bench.library.add(import(fixture).unwrap()).unwrap();
    let mut saved = library::jobs::tests::row("saved");
    saved.identify();
    bench
        .library
        .attach(&saved, std::slice::from_ref(&run.id))
        .unwrap();

    let out = folder.path().join("out.json");
    let panel =
        || serde_json::from_slice::<Value>(&std::fs::read(&out).unwrap()).unwrap()["panel"].clone();
    assert!(export(&bench, &out, ResultSource::Job(first)).is_err());
    assert!(!out.exists());
    export(&bench, &out, ResultSource::Job(second)).unwrap();
    assert_eq!(panel(), "second");
    export(&bench, &out, ResultSource::Result(saved.id.clone())).unwrap();
    assert_eq!(panel(), "saved");
    // a recording's id is not a result's
    assert!(export(&bench, &out, ResultSource::Result(run.id.clone())).is_err());
}
#[test]
fn a_failed_recording_is_listed_with_its_status_and_kept_out_of_comparisons() {
    let folder = tempfile::tempdir().unwrap();
    let pcap = folder.path().join("failed.pcap");
    std::fs::copy(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../tests/fixtures/mouse-1000hz.pcap"
        ),
        &pcap,
    )
    .unwrap();
    std::fs::write(
        pcap.with_extension("incomplete.json"),
        r#"{"reason":"device_lost","error":"selected device disconnected"}"#,
    )
    .unwrap();
    let run = import(&pcap).unwrap();
    assert_eq!(run.status.kind, "failed");
    assert_eq!(run.status.reason, Some(Reason::DeviceLost));
    assert_eq!(
        run.status.message.as_deref(),
        Some("selected device disconnected")
    );
    assert!(run.panels.is_empty());
    assert!(run.missing["texture19"].contains("selected device disconnected"));
    // its evidence is still there to look at
    assert!(!run.devices.is_empty());
    let complete = import(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../tests/fixtures/mouse-1000hz.pcap"
    )))
    .unwrap();
    assert_eq!(complete.status.kind, "complete");
    // no box input was saved with it, so only the paired comparison turns it down
    assert_eq!(complete.refusals.comparison, None);
    assert!(
        complete
            .refusals
            .paired
            .as_deref()
            .is_some_and(|why| why.contains("no physical input"))
    );
    assert!(complete.panels.iter().any(|p| p == "texture19"));
}
#[test]
fn a_refused_recording_never_offers_a_panel_wherever_it_is_listed_from() {
    let folder = tempfile::tempdir().unwrap();
    let pcap = folder.path().join("failed.pcap");
    let fixtures = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/fixtures"));
    std::fs::copy(fixtures.join("mouse-1000hz.pcap"), &pcap).unwrap();
    std::fs::copy(
        fixtures.join("events.npz"),
        pcap.with_extension("events.npz"),
    )
    .unwrap();
    std::fs::write(
        pcap.with_extension("incomplete.json"),
        r#"{"reason":"writer_failed","error":"disk full"}"#,
    )
    .unwrap();
    // the event dataset beside it would offer three panels
    let run = import(&pcap).unwrap();
    assert!(run.panels.is_empty(), "{:?}", run.panels);
    for panel in ["texture19", "trajectory14", "full49"] {
        assert!(run.missing[panel].contains("disk full"), "{panel}");
    }
    // a restart restores it with its saved decoder
    Library::new(folder.path().to_path_buf()).add(run).unwrap();
    let library = Library::new(folder.path().to_path_buf());
    let warnings = library.restore();
    assert!(warnings.is_empty(), "{warnings:?}");
    let runs = library.runs();
    let restored = runs.iter().find(|r| r.name == "failed.pcap").unwrap();
    assert_eq!(restored.status.kind, "failed");
    assert!(restored.panels.is_empty(), "{:?}", restored.panels);
}
#[test]
fn a_capture_with_nothing_readable_is_listed_as_failed() {
    let folder = tempfile::tempdir().unwrap();
    let pcap = folder.path().join("empty.pcap");
    std::fs::write(&pcap, b"").unwrap();
    // with nothing beside it, it was not recorded here
    assert!(import(&pcap).is_err());
    // with the marker beside it, it is what a crash right after starting leaves
    std::fs::write(pcap.with_extension("active.json"), b"{}").unwrap();
    let run = import(&pcap).unwrap();
    assert_eq!(run.status.kind, "interrupted");
    assert!(run.panels.is_empty());
    assert_eq!(run.path, pcap.display().to_string());
    assert!(!run.path.starts_with(r"\\?\"));
    // it never wrote metadata, so the marker says when it began
    assert_eq!(run.started_us, None);
    std::fs::write(
        pcap.with_extension("active.json"),
        br#"{"started_us":1792020044265455}"#,
    )
    .unwrap();
    assert_eq!(import(&pcap).unwrap().started_us, Some(1792020044265455));
    // but a failure file or published metadata beside it still decides first
    std::fs::write(
        pcap.with_extension("incomplete.json"),
        br#"{"reason":"writer_failed","error":"disk full"}"#,
    )
    .unwrap();
    let run = import(&pcap).unwrap();
    assert_eq!(run.status.kind, "failed");
    assert_eq!(run.status.message.as_deref(), Some("disk full"));
    // and with published metadata, a capture with nothing readable in it failed
    std::fs::remove_file(pcap.with_extension("incomplete.json")).unwrap();
    std::fs::write(pcap.with_extension("json"), sidecar("complete").to_string()).unwrap();
    let run = import(&pcap).unwrap();
    assert_eq!(run.status.kind, "failed");
    assert!(
        run.status
            .message
            .as_deref()
            .unwrap()
            .starts_with("the capture file cannot be read")
    );
    assert!(run.panels.is_empty());
}
#[test]
fn a_saved_device_that_is_not_in_the_recording_is_said_and_not_compared() {
    use motion_bench::testkit::traffic::{Descriptors, Packet, Traffic, mouse_payload, ticks};
    let folder = tempfile::tempdir().unwrap();
    let pcap = folder.path().join("moved.pcap");
    // two mice, so nothing is picked without the saved selection
    let mut traffic = Traffic::default();
    for device in [2, 3] {
        traffic.describe(0, 1, device, &Descriptors::mouse(0x373b, 0x11d9));
        traffic.extend(
            ticks(1000, 1_000_000 + u64::from(device), 50)
                .map(|t| (t, Packet::report(1, device, 0x81, &mouse_payload(1, -1, 7)))),
        );
    }
    std::fs::write(&pcap, traffic.sorted().pcap().bytes()).unwrap();
    let mut metadata = sidecar("complete");
    metadata["selection"] = json!({"bus": 9, "address": 9, "endpoint": 129});
    std::fs::write(pcap.with_extension("json"), metadata.to_string()).unwrap();
    let mut run = import(&pcap).unwrap();
    assert!(run.selected.is_none());
    assert!(run.missing["texture19"].contains("saved device is not in this recording"));
    assert!(!run.panels.iter().any(|p| p == "texture19"));
    // picking one clears the note and offers the comparison
    run.pick(
        "1:2:129".into(),
        motion_bench::testkit::traffic::mouse_decoder(),
    );
    assert!(!run.missing.contains_key("texture19"));
    assert_eq!(run.panels, ["texture19"]);
    // a refused recording stays refused whichever device is picked
    metadata["outcome"] = "failed".into();
    metadata["reason"] = "writer_failed".into();
    metadata["health"]["error"] = "disk full".into();
    std::fs::write(pcap.with_extension("json"), metadata.to_string()).unwrap();
    let mut run = import(&pcap).unwrap();
    run.pick(
        "1:2:129".into(),
        motion_bench::testkit::traffic::mouse_decoder(),
    );
    assert!(run.panels.is_empty(), "{:?}", run.panels);
    assert!(run.missing["texture19"].contains("disk full"));
}
/// a live recording of a generated mouse, closed and ready to commit
fn closed_recording(folder: &Path) -> Closing {
    use motion_bench::testkit::traffic::{Descriptors, Packet, Traffic, mouse_payload, ticks};
    let mut traffic = Traffic::default();
    traffic.describe(0, 1, 2, &Descriptors::mouse(0x373b, 0x11d9));
    traffic.extend(
        ticks(1000, 1_000_000, 50)
            .map(|t| (t, Packet::report(1, 2, 0x81, &mouse_payload(1, -1, 7)))),
    );
    let mut data = session::Data::new(true);
    pcap::read_pcap(traffic.pcap().bytes(), |record| {
        data.ingest(&record);
        if data.recorder.is_none() && data.health.reports == 1 {
            data.select(
                "1:2:129",
                Some(motion_bench::testkit::traffic::mouse_decoder()),
            )
            .unwrap();
            data.trust_selection();
            data.begin("test", folder, None).unwrap();
        }
    })
    .unwrap();
    data.close().unwrap()
}

#[test]
fn the_participants_end_up_in_the_metadata_once() {
    let folder = tempfile::tempdir().unwrap();
    let closing = closed_recording(folder.path());
    std::fs::write(
        closing.path("observers.json"),
        br#"{"complete":true,"error":null}"#,
    )
    .unwrap();
    let control = json!({"path": closing.path("pcap").display().to_string()});
    let (committed, saved) = commit_recording(closing, StopReason::RecordStop, Some(control));
    assert_eq!(saved, None);
    let pcap = committed.unwrap();
    let read = |kind: &str| -> Value {
        serde_json::from_slice(&std::fs::read(pcap.with_extension(kind)).unwrap()).unwrap()
    };
    let metadata = read("json");
    assert_eq!(metadata["outcome"], "complete");
    assert_eq!(metadata["observers"]["complete"], true);
    // the journal is saved first and the metadata keeps the same copy
    assert_eq!(
        metadata["recording_control"],
        read("recording-control.json")
    );
    assert_eq!(metadata["recording_control"]["complete"], true);
    assert!(!pcap.with_extension("active.json").exists());

    // a journal that cannot be saved leaves the usb evidence whole: the recording is
    // complete, its copy of the journal says it failed, and the reader calls it degraded
    let folder = tempfile::tempdir().unwrap();
    let closing = closed_recording(folder.path());
    let (committed, saved) = commit_recording(closing, StopReason::RecordStop, Some(json!({})));
    assert!(saved.unwrap().contains("no capture path"));
    let pcap = committed.unwrap();
    let metadata: Value =
        serde_json::from_slice(&std::fs::read(pcap.with_extension("json")).unwrap()).unwrap();
    assert_eq!(metadata["outcome"], "degraded");
    assert_eq!(metadata["recording_control"]["complete"], false);
    assert!(matches!(
        open_recording(&pcap, None).unwrap().status,
        motion_bench::evidence::reader::status::Status::Degraded(_)
    ));

    // observers that left an unreadable file fail the recording, which still publishes
    let folder = tempfile::tempdir().unwrap();
    let closing = closed_recording(folder.path());
    std::fs::write(closing.path("observers.json"), b"{").unwrap();
    let (committed, _) = commit_recording(closing, StopReason::RecordStop, None);
    let failure = committed.unwrap_err();
    assert_eq!(failure.reason, Reason::ParticipantFailed);
    assert!(
        failure
            .message
            .contains("input observer evidence cannot be read")
    );
    let name = std::fs::read_dir(folder.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "pcap"))
        .unwrap();
    let metadata: Value =
        serde_json::from_slice(&std::fs::read(name.with_extension("json")).unwrap()).unwrap();
    assert_eq!(metadata["outcome"], "failed");
    assert_eq!(metadata["health"]["reason"], "participant_failed");

    // observers that say they did not see it through leave the usb evidence whole
    let folder = tempfile::tempdir().unwrap();
    let closing = closed_recording(folder.path());
    std::fs::write(
        closing.path("observers.json"),
        br#"{"complete":false,"error":null}"#,
    )
    .unwrap();
    let pcap = commit_recording(closing, StopReason::RecordStop, None)
        .0
        .unwrap();
    let metadata: Value =
        serde_json::from_slice(&std::fs::read(pcap.with_extension("json")).unwrap()).unwrap();
    assert_eq!(metadata["outcome"], "degraded");
    assert_eq!(metadata["participants"][0]["outcome"], "failed");
    assert_eq!(
        metadata["participants"][0]["error"],
        "the input observers did not complete"
    );

    // so do observers whose file is there but cannot be read at all
    let folder = tempfile::tempdir().unwrap();
    let closing = closed_recording(folder.path());
    std::fs::create_dir(closing.path("observers.json")).unwrap();
    let (committed, _) = commit_recording(closing, StopReason::RecordStop, None);
    assert_eq!(committed.unwrap_err().reason, Reason::ParticipantFailed);
}

#[test]
fn a_recording_the_app_closed_on_is_listed_interrupted_and_still_compared() {
    let folder = tempfile::tempdir().unwrap();
    let closing = closed_recording(folder.path());
    let control = json!({
        "path": closing.path("pcap").display().to_string(),
        "stop_cause": "application closed without an explicit finish",
    });
    let (committed, saved) = commit_recording(closing, StopReason::AppExit, Some(control));
    assert_eq!(saved, None);
    let pcap = committed.unwrap();
    let journal: Value = serde_json::from_slice(
        &std::fs::read(pcap.with_extension("recording-control.json")).unwrap(),
    )
    .unwrap();
    // the journal saw the stop through, the recording's metadata says it was cut short
    assert_eq!(journal["complete"], true);
    let run = import(&pcap).unwrap();
    assert_eq!(run.status.kind, "interrupted");
    assert_eq!(
        run.status.message.as_deref(),
        Some("the app closed while it was recording")
    );
    assert!(
        run.panels.iter().any(|p| p == "texture19"),
        "{:?}",
        run.missing
    );
}

#[test]
fn a_calibration_never_rewrites_the_recordings_metadata() {
    let folder = tempfile::tempdir().unwrap();
    let pcap = folder.path().join("run.pcap");
    let metadata = br#"{"complete":true,"health":{"error":null}}"#;
    std::fs::write(pcap.with_extension("json"), metadata).unwrap();
    let result = motion_bench::targets::calibration::Response {
        schema: "test".into(),
        method: "test".into(),
        estimated_delay_ms: 12,
        admissible_delay_ms: [10, 14],
        residual_counts: 0.5,
        radius_counts: 1.0,
        movement_span_counts: 100.0,
        frames: 60,
        span_ms: 1000,
        selected_range_seconds: [0.0, 1.0],
        single_target_segments: 1,
        excluded_samples: 0,
        identifiable: true,
        reason: None,
        profile: Default::default(),
        residual_curve: vec![],
        absolute_hardware_latency_known: false,
        applied_to_scores: false,
        crop_us: None,
    };
    save_calibration(&pcap, &result).unwrap();
    assert_eq!(
        std::fs::read(pcap.with_extension("json")).unwrap(),
        metadata
    );
    let saved: Value =
        serde_json::from_slice(&std::fs::read(pcap.with_extension("calibration.json")).unwrap())
            .unwrap();
    assert_eq!(saved["estimated_delay_ms"], 12);
}
#[test]
fn target_events_beside_a_recording_offer_their_panels_and_alone_are_not_a_recording() {
    let folder = tempfile::tempdir().unwrap();
    let pcap = recorded(folder.path(), "run.pcap");
    let events = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../tests/fixtures/events.npz"
    ));
    std::fs::copy(events, pcap.with_extension("events.npz")).unwrap();
    let run = import(&pcap).unwrap();
    assert_eq!(run.panels, vec!["texture19", "trajectory14", "full49"]);
    assert!(!run.paired);
    assert!(import(events).is_err());
}

/// target capture that made no trial says why, by the reason most clicks were left out for
#[test]
fn a_recording_with_no_trials_says_why() {
    let folder = tempfile::tempdir().unwrap();
    let pcap = folder.path().join("run.pcap");
    let missing = r#"{"reason":"target missing or multiple detections"}"#;
    let residual = |n: f64| {
        format!(
            r#"{{"reason":"target/projection residual {n} counts exceeds tolerance, check calibration, display delay, or target motion"}}"#
        )
    };
    std::fs::write(
        pcap.with_extension("events.json"),
        format!(
            r#"{{"accepted":0,"events":[{},{},{missing}]}}"#,
            residual(44.6),
            residual(52.1)
        ),
    )
    .unwrap();
    let why = crate::library::no_trials(&pcap).unwrap();
    assert!(
        why.contains("none of the 3 clicks made a trial, 2 of them"),
        "{why}"
    );
    assert!(why.contains("check the sensitivity"), "{why}");
    std::fs::write(pcap.with_extension("events.json"), r#"{"events":[]}"#).unwrap();
    assert!(
        crate::library::no_trials(&pcap)
            .unwrap()
            .contains("no click")
    );
    std::fs::remove_file(pcap.with_extension("events.json")).unwrap();
    assert_eq!(crate::library::no_trials(&pcap), None);
}

#[test]
fn a_recordings_folder_is_made_and_checked_for_writing_before_it_is_saved() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("new").join("recordings");
    crate::commands::session::writable(&folder).unwrap();
    assert!(folder.is_dir());
    // the check leaves nothing behind
    assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 0);
    let file = dir.path().join("a file");
    std::fs::write(&file, b"").unwrap();
    assert!(crate::commands::session::writable(&file.join("inside")).is_err());
}
