use super::*;
use crate::capture::{pcap, session::Data};
use crate::evidence::reason::{Reason, StopReason};
use crate::sync::LockExt;
use crate::testkit::faults::{self, Fault, Gate, inject};
use crate::testkit::traffic::{Descriptors, Packet, TempDir, Traffic, mouse_payload, ticks};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn train(count: u64) -> Traffic {
    let mut traffic = Traffic::default();
    traffic.describe(0, 1, 2, &Descriptors::mouse(0x373b, 0x11d9));
    traffic.extend(
        ticks(1000, 1_000_000, count)
            .map(|t| (t, Packet::report(1, 2, 0x81, &mouse_payload(1, -1, 7)))),
    );
    traffic
}

/// the live state recording every report of the train after the first, still recording
fn recording(folder: &Path, count: u64) -> (Data, String) {
    recording_with(folder, count, false)
}

/// the same, with the box sending input beside every report when `input` is set, so the
/// input and receipt files exist too
fn recording_with(folder: &Path, count: u64, input: bool) -> (Data, String) {
    let mut data = Data::new(true);
    let mut stem = None;
    let mut sent = 0u64;
    pcap::read_pcap(train(count).pcap().bytes(), |record| {
        data.ingest(&record);
        if input && data.recorder.is_some() {
            sent += 1;
            data.input(&crate::hardware::medius::Motion {
                arrived_us: crate::capture::session::wall_us(),
                arrived_qpc: crate::capture::clock::qpc(),
                box_us: 5_000_000 + sent * 1000,
                dx: 1,
                dy: 0,
                dz: 0,
            });
        }
        if stem.is_none() && data.health.reports > 0 {
            data.select("1:2:129", Some(crate::testkit::traffic::mouse_decoder()))
                .unwrap();
            data.trust_selection();
            stem = Some(data.begin("test", folder, None).unwrap());
        }
    })
    .unwrap();
    (data, stem.unwrap())
}

fn names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn published_metadata_is_never_written_twice() {
    let folder = TempDir::new("publish-twice");
    std::fs::create_dir_all(folder.path()).unwrap();
    let path = folder.path().join("run.json");
    publish(&path, &serde_json::json!({"first": true})).unwrap();
    let error = publish(&path, &serde_json::json!({"first": false})).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    let kept: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(kept["first"], true);
    assert_eq!(names(folder.path()), ["run.json"]);
}

#[test]
fn the_marker_goes_down_first_and_comes_off_last() {
    let folder = TempDir::new("order");
    let _ = faults::take_log();
    let (mut data, _) = recording_with(folder.path(), 50, true);
    data.finish().unwrap();
    let log = faults::take_log();
    let at = |entry: &str| {
        log.iter()
            .position(|e| e == entry)
            .unwrap_or_else(|| panic!("{entry} missing from {log:?}"))
    };
    assert_eq!(&log[..2], ["create active.json", "sync active.json"]);
    for evidence in ["csv", "motion.csv", "clocks.jsonl", "pcap"] {
        assert!(at("sync active.json") < at(&format!("create {evidence}")));
    }
    for evidence in EVIDENCE {
        assert!(
            at(&format!("sync {evidence}")) < at("create json.tmp"),
            "{evidence}"
        );
    }
    assert_eq!(
        &log[log.len() - 4..],
        [
            "create json.tmp",
            "sync json.tmp",
            "rename json.tmp json",
            "remove active.json"
        ]
    );
}

/// every file a recording with box input writes evidence into
const EVIDENCE: [&str; 6] = [
    "csv",
    "motion.csv",
    "clocks.jsonl",
    "pcap",
    "input.csv",
    "input-arrivals.csv",
];

/// each evidence file holds all its bytes by the time it is synced, so a sync never makes
/// half a buffer durable
#[test]
fn every_evidence_file_is_whole_when_it_is_synced() {
    for kind in EVIDENCE {
        let gate: &'static Gate = Box::leak(Box::new(Gate::new()));
        let folder = TempDir::new("whole");
        let path = folder.path().to_path_buf();
        let recording = std::thread::spawn(move || {
            let _fault = inject(kind, Fault::Pause(gate));
            let (mut data, stem) = recording_with(&path, 100, true);
            data.finish().unwrap();
            stem
        });
        assert!(gate.reached(Duration::from_secs(20)), "{kind} never synced");
        let file = std::fs::read_dir(folder.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .and_then(|n| n.split_once('.'))
                    .is_some_and(|(_, k)| k == kind)
            })
            .unwrap();
        let at_sync = std::fs::metadata(&file).unwrap().len();
        gate.release();
        recording.join().unwrap();
        assert_eq!(at_sync, std::fs::metadata(&file).unwrap().len(), "{kind}");
    }
}

#[test]
fn box_input_that_cannot_be_synced_fails_the_recording() {
    for kind in ["input.csv", "input-arrivals.csv"] {
        let folder = TempDir::new("input-sync");
        let _fault = inject(kind, Fault::Sync);
        let (mut data, stem) = recording_with(folder.path(), 50, true);
        data.finish().unwrap_err();
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(folder.path().join(format!("{stem}.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["reason"], "writer_failed", "{kind}");
    }
}

#[test]
fn the_marker_names_the_recording_its_process_and_its_start() {
    let folder = TempDir::new("marker-content");
    let (mut data, stem) = recording(folder.path(), 50);
    let marker: serde_json::Value = serde_json::from_slice(
        &std::fs::read(folder.path().join(format!("{stem}.active.json"))).unwrap(),
    )
    .unwrap();
    let path = data.finish().unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.with_extension("json")).unwrap()).unwrap();
    assert_eq!(marker["schema"], "motion-bench.active.v1");
    assert_eq!(marker["stem"], stem);
    assert_eq!(marker["pid"], std::process::id());
    assert_eq!(marker["started_us"], manifest["clock"]["system_us"]);
}

#[test]
fn a_recording_refused_because_capture_failed_meanwhile_leaves_nothing() {
    let folder = TempDir::new("failed-meanwhile");
    let mut data = Data::new(true);
    pcap::read_pcap(train(3).pcap().bytes(), |record| data.ingest(&record)).unwrap();
    data.select("1:2:129", Some(crate::testkit::traffic::mouse_decoder()))
        .unwrap();
    data.trust_selection();
    let (device, _) = data.plan("test", None).unwrap();
    let recorder = crate::evidence::recorder::Recorder::begin(folder.path(), device).unwrap();
    // a report from the past keeps the selection but has it checked again. selecting starts the
    // device's history afresh, so one report comes first to be behind
    let behind = Traffic(vec![
        (
            1_010_000,
            Packet::report(1, 2, 0x81, &mouse_payload(1, 1, 7)),
        ),
        (
            1_005_000,
            Packet::report(1, 2, 0x81, &mouse_payload(1, 1, 7)),
        ),
    ]);
    pcap::read_pcap(behind.pcap().bytes(), |record| data.ingest(&record)).unwrap();
    assert!(data.selected.is_some());
    assert!(data.readiness().is_err());
    data.install(recorder).err().unwrap().abandon();
    assert!(
        names(folder.path()).is_empty(),
        "{:?}",
        names(folder.path())
    );
    // and capture itself ending meanwhile
    data.trust_selection();
    let (device, _) = data.plan("test", None).unwrap();
    let recorder = crate::evidence::recorder::Recorder::begin(folder.path(), device).unwrap();
    data.fail(Reason::TransportEnded, "USBPcap stream ended unexpectedly");
    data.install(recorder).err().unwrap().abandon();
    assert!(names(folder.path()).is_empty());
}

#[test]
fn a_marker_that_cannot_be_written_whole_refuses_the_recording_and_leaves_nothing() {
    for fault in [
        Fault::Create,
        Fault::WriteAfter(4),
        Fault::Flush,
        Fault::Sync,
    ] {
        let folder = TempDir::new("marker");
        let mut data = Data::new(true);
        pcap::read_pcap(train(2).pcap().bytes(), |record| data.ingest(&record)).unwrap();
        data.select("1:2:129", Some(crate::testkit::traffic::mouse_decoder()))
            .unwrap();
        let _fault = inject("active.json", fault);
        data.trust_selection();
        assert!(
            data.begin("test", folder.path(), None).is_err(),
            "{fault:?}"
        );
        assert!(data.recorder.is_none());
        assert!(names(folder.path()).is_empty(), "{fault:?}");
    }
}

#[test]
fn a_recording_refused_once_its_files_exist_leaves_nothing() {
    let folder = TempDir::new("refused");
    let mut data = Data::new(true);
    let mut traffic = train(2);
    pcap::read_pcap(traffic.pcap().bytes(), |record| data.ingest(&record)).unwrap();
    data.select("1:2:129", Some(crate::testkit::traffic::mouse_decoder()))
        .unwrap();
    data.trust_selection();
    let (device, _) = data.plan("test", None).unwrap();
    let recorder = crate::evidence::recorder::Recorder::begin(folder.path(), device).unwrap();
    // the device goes while the files were being made
    traffic = Traffic::default();
    traffic.add(1_100_000, Packet::disconnect(1, 2, 0x81));
    pcap::read_pcap(traffic.pcap().bytes(), |record| data.ingest(&record)).unwrap();
    data.install(recorder).err().unwrap().abandon();
    assert!(data.recorder.is_none());
    assert!(
        names(folder.path()).is_empty(),
        "{:?}",
        names(folder.path())
    );
}

#[test]
fn a_recording_refused_because_another_device_was_picked_leaves_nothing() {
    let folder = TempDir::new("repicked");
    let mut traffic = train(2);
    traffic.describe(10, 1, 3, &Descriptors::mouse(0x373b, 0x11d9));
    traffic.add(
        1_000_500,
        Packet::report(1, 3, 0x81, &mouse_payload(1, 1, 7)),
    );
    let mut data = Data::new(true);
    pcap::read_pcap(traffic.sorted().pcap().bytes(), |record| {
        data.ingest(&record)
    })
    .unwrap();
    data.select("1:2:129", Some(crate::testkit::traffic::mouse_decoder()))
        .unwrap();
    data.trust_selection();
    let (device, _) = data.plan("test", None).unwrap();
    let recorder = crate::evidence::recorder::Recorder::begin(folder.path(), device).unwrap();
    // nothing failed, the other mouse was simply picked while the files were made
    data.select("1:3:129", Some(crate::testkit::traffic::mouse_decoder()))
        .unwrap();
    data.install(recorder).err().unwrap().abandon();
    assert!(data.health.error.is_none());
    assert!(
        names(folder.path()).is_empty(),
        "{:?}",
        names(folder.path())
    );
}

/// a participant that fails once input has ended, with box input recorded: the recording is
/// failed, and neither the input file nor the metadata claims the two clocks were related
#[test]
fn a_failure_after_input_ends_fails_the_recording_and_relates_no_clocks() {
    let folder = TempDir::new("late-failure");
    let mut data = Data::new(true);
    let mut stem = None;
    let mut sent = 0u64;
    pcap::read_pcap(train(200).pcap().bytes(), |record| {
        data.ingest(&record);
        if data.recorder.is_some() {
            sent += 1;
            data.input(&crate::hardware::medius::Motion {
                arrived_us: crate::capture::session::wall_us(),
                arrived_qpc: crate::capture::clock::qpc(),
                box_us: 5_000_000 + sent * 1000,
                dx: 1,
                dy: 0,
                dz: 0,
            });
        } else if stem.is_none() && data.health.reports > 0 {
            data.select("1:2:129", Some(crate::testkit::traffic::mouse_decoder()))
                .unwrap();
            data.trust_selection();
            stem = Some(data.begin("test", folder.path(), None).unwrap());
        }
    })
    .unwrap();
    let stem = stem.unwrap();
    let file = |kind: &str| folder.path().join(format!("{stem}.{kind}"));
    let mut closing = data.close().unwrap();
    closing.fail(Reason::ParticipantFailed, "screen recording stopped early");
    let failure = closing
        .commit(StopReason::RecordStop, Vec::new())
        .unwrap_err();
    assert_eq!(failure.reason, Reason::ParticipantFailed);
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(file("json")).unwrap()).unwrap();
    assert_eq!(manifest["outcome"], "failed");
    assert_eq!(manifest["health"]["reason"], "participant_failed");
    assert_eq!(manifest["health"]["pairing_valid"], false);
    assert_eq!(manifest["paired"]["shift_us"], serde_json::Value::Null);
    assert!(manifest["paired"]["input_rows"].as_u64().unwrap() > 0);
    let input = std::fs::read_to_string(file("input.csv")).unwrap();
    assert!(!input.contains("# paired_shift_us"), "{input}");
}

#[test]
fn every_stage_of_publishing_fails_alone_and_keeps_the_marker_and_the_evidence() {
    let reference = TempDir::new("publish-reference");
    let (mut data, stem) = recording(reference.path(), 200);
    data.finish().unwrap();
    let full_pcap = std::fs::read(reference.path().join(format!("{stem}.pcap"))).unwrap();
    for (kind, fault) in [
        ("json.tmp", Fault::Create),
        ("json.tmp", Fault::WriteAfter(16)),
        ("json.tmp", Fault::Flush),
        ("json.tmp", Fault::Sync),
        ("json", Fault::Rename),
    ] {
        let folder = TempDir::new("publish");
        let _fault = inject(kind, fault);
        let (mut data, stem) = recording(folder.path(), 200);
        let error = data.finish().unwrap_err();
        assert!(error.contains("metadata"), "{kind} {fault:?}: {error}");
        // a recording that could not be published does not stop capture
        assert!(data.health.error.is_none());
        let file = |kind: &str| folder.path().join(format!("{stem}.{kind}"));
        assert!(!file("json").exists(), "{kind} {fault:?}");
        assert!(!file("json.tmp").exists(), "{kind} {fault:?}");
        assert!(file("active.json").is_file(), "{kind} {fault:?}");
        assert_eq!(std::fs::read(file("pcap")).unwrap(), full_pcap);
        // the code goes where every reader already looks, so it reads as failed, not unknown
        let opened =
            crate::evidence::reader::recording::open_recording(&file("pcap"), None).unwrap();
        assert!(
            matches!(
                opened.status,
                crate::evidence::reader::status::Status::Failed {
                    reason: Some(Reason::MetadataFailed),
                    ..
                }
            ),
            "{kind} {fault:?}: {:?}",
            opened.status
        );
    }
}

#[test]
fn a_marker_that_will_not_come_off_loses_to_the_published_metadata() {
    let folder = TempDir::new("unmark");
    let _fault = inject("active.json", Fault::Remove);
    let (mut data, stem) = recording(folder.path(), 50);
    data.finish().unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(folder.path().join(format!("{stem}.json"))).unwrap())
            .unwrap();
    assert_eq!(manifest["outcome"], "complete");
    assert!(folder.path().join(format!("{stem}.active.json")).is_file());
}

#[test]
fn closing_the_files_and_publishing_run_without_the_live_state() {
    static AT_SYNC: Gate = Gate::new();
    let folder = TempDir::new("unlocked");
    let (data, _) = recording(folder.path(), 50);
    let data = Arc::new(Mutex::new(data));
    let closing = data.locked().close().unwrap();
    let commit = std::thread::spawn(move || {
        let _fault = inject("json.tmp", Fault::Pause(&AT_SYNC));
        closing.commit(StopReason::RecordStop, Vec::new())
    });
    // the commit is stopped in the middle of syncing the metadata, and capture goes on
    let reached = AT_SYNC.reached(Duration::from_secs(10));
    let later = Traffic(vec![(
        2_000_000,
        Packet::report(1, 2, 0x81, &mouse_payload(1, 1, 7)),
    )]);
    let before = data.locked().health.reports;
    pcap::read_pcap(later.pcap().bytes(), |record| data.locked().ingest(&record)).unwrap();
    assert_eq!(data.locked().health.reports, before + 1);
    AT_SYNC.release();
    commit.join().unwrap().unwrap();
    assert!(reached, "the commit never synced its metadata");
}

#[test]
fn a_write_stuck_on_the_disk_never_holds_the_live_state() {
    static AT_WRITE: Gate = Gate::new();
    let folder = TempDir::new("stuck-write");
    let (data, _) = {
        let _fault = inject("csv", Fault::HoldWrite(&AT_WRITE));
        recording(folder.path(), 50)
    };
    let data = Arc::new(Mutex::new(data));
    let reader = {
        let data = data.clone();
        std::thread::spawn(move || {
            let later = Traffic(
                ticks(1000, 2_000_000, 2000)
                    .map(|t| (t, Packet::report(1, 2, 0x81, &mouse_payload(1, 1, 7))))
                    .collect(),
            );
            pcap::read_pcap(later.pcap().bytes(), |record| {
                crate::capture::session::ingest_shared(&data, &record)
            })
            .unwrap();
        })
    };
    assert!(
        AT_WRITE.reached(Duration::from_secs(10)),
        "the csv was never written"
    );
    // the reader is stopped in the middle of a write, and the live state is free
    assert!(data.try_lock().is_ok());
    AT_WRITE.release();
    reader.join().unwrap();
    let closing = data.locked().close().unwrap();
    let pcap = closing.commit(StopReason::RecordStop, Vec::new()).unwrap();
    let csv = std::fs::read_to_string(pcap.with_extension("csv")).unwrap();
    let times: Vec<u64> = csv
        .lines()
        .skip(1)
        .map(|row| row.split(',').next().unwrap().parse().unwrap())
        .collect();
    // the first report starts the recording, every one after it is in the csv
    assert_eq!(times.len() as u64, data.locked().health.reports - 1);
    assert!(times.windows(2).all(|pair| pair[0] < pair[1]));
}

/// two writers: the reader is stuck writing, so the files are taken, and the pump's box input
/// comes in meanwhile. the pump waits for the files with the live state let go
#[test]
fn a_second_writer_waits_for_the_files_without_the_live_state() {
    static AT_WRITE: Gate = Gate::new();
    let folder = TempDir::new("two-writers");
    let (data, _) = {
        let _fault = inject("csv", Fault::HoldWrite(&AT_WRITE));
        recording(folder.path(), 50)
    };
    let data = Arc::new(Mutex::new(data));
    let reader = {
        let data = data.clone();
        std::thread::spawn(move || {
            let later = Traffic(
                ticks(1000, 2_000_000, 2000)
                    .map(|t| (t, Packet::report(1, 2, 0x81, &mouse_payload(1, 1, 7))))
                    .collect(),
            );
            pcap::read_pcap(later.pcap().bytes(), |record| {
                crate::capture::session::ingest_shared(&data, &record)
            })
            .unwrap();
        })
    };
    assert!(
        AT_WRITE.reached(Duration::from_secs(10)),
        "the csv was never written"
    );
    let (queued, waiting) = std::sync::mpsc::channel();
    let pump = {
        let data = data.clone();
        std::thread::spawn(move || {
            let mut guard = data.locked();
            guard.recorder.as_mut().unwrap().observe_clock(true);
            let writes = guard.writes().expect("a clock anchor waits to be written");
            queued.send(()).unwrap();
            writes.after(guard);
        })
    };
    waiting.recv().unwrap();
    // the pump is waiting for the files the reader holds, and the live state is free. if it
    // still held it, this would wait on the stuck write and run out
    let until = std::time::Instant::now() + Duration::from_secs(5);
    while data.try_lock().is_err() {
        assert!(
            std::time::Instant::now() < until,
            "the pump held the live state over a write"
        );
        std::thread::yield_now();
    }
    AT_WRITE.release();
    reader.join().unwrap();
    pump.join().unwrap();
    let closing = data.locked().close().unwrap();
    let pcap = closing.commit(StopReason::RecordStop, Vec::new()).unwrap();
    let csv = std::fs::read_to_string(pcap.with_extension("csv")).unwrap();
    let rows = csv.lines().skip(1).count() as u64;
    assert_eq!(rows, data.locked().health.reports - 1);
}

#[test]
fn making_the_files_runs_without_the_live_state() {
    static AT_SYNC: Gate = Gate::new();
    let folder = TempDir::new("unlocked-begin");
    let data = Arc::new(Mutex::new(Data::new(true)));
    pcap::read_pcap(train(2).pcap().bytes(), |record| {
        data.locked().ingest(&record)
    })
    .unwrap();
    data.locked()
        .select("1:2:129", Some(crate::testkit::traffic::mouse_decoder()))
        .unwrap();
    data.locked().trust_selection();
    let (device, decoder) = data.locked().plan("test", None).unwrap();
    let path = folder.path().to_path_buf();
    let begin = std::thread::spawn(move || {
        let _fault = inject("active.json", Fault::Pause(&AT_SYNC));
        crate::evidence::recorder::Recorder::begin(&path, device)
    });
    let reached = AT_SYNC.reached(Duration::from_secs(10));
    let later = Traffic(vec![(
        1_500_000,
        Packet::report(1, 2, 0x81, &mouse_payload(1, 1, 7)),
    )]);
    pcap::read_pcap(later.pcap().bytes(), |record| data.locked().ingest(&record)).unwrap();
    AT_SYNC.release();
    let mut recorder = begin.join().unwrap().unwrap();
    assert!(reached, "the marker was never synced");
    recorder.decoder = decoder;
    assert!(data.locked().install(recorder).is_ok());
    assert!(data.locked().recorder.is_some());
}

/// the metadata the writer publishes has every key and value type the hand written file had
/// the sidecar for fixed traffic keeps its keys and its values, except those that depend on when
/// the test runs. the golden is the writer's own output for this same traffic
#[test]
fn the_metadata_keeps_its_shape() {
    let info = crate::hardware::medius::Snapshot::default().json();
    let folder = TempDir::new("golden");
    let mut data = Data::new(true);
    let mut stem = None;
    let mut sent = 0u64;
    pcap::read_pcap(
        crate::testkit::traffic::Traffic(
            train(0)
                .0
                .into_iter()
                .chain(ticks(1000, 1_000_000, 300).map(|t| {
                    let step = (t / 1000 % 5) as i16 - 2;
                    (
                        t,
                        Packet::report(1, 2, 0x81, &mouse_payload(step, -step, 7)),
                    )
                }))
                .collect(),
        )
        .pcap()
        .bytes(),
        |record| {
            data.ingest(&record);
            if data.recorder.is_some() {
                sent += 1;
                if sent.is_multiple_of(10) {
                    data.input(&crate::hardware::medius::Motion {
                        arrived_us: crate::capture::session::wall_us(),
                        arrived_qpc: crate::capture::clock::qpc(),
                        box_us: 5_000_000 + sent * 1000,
                        dx: 1,
                        dy: -1,
                        dz: 0,
                    });
                }
            } else if stem.is_none() && data.health.reports > 0 {
                data.select("1:2:129", Some(crate::testkit::traffic::mouse_decoder()))
                    .unwrap();
                data.trust_selection();
                stem = Some(
                    data.begin("USBPcap1", folder.path(), Some(info.clone()))
                        .unwrap(),
                );
            }
        },
    )
    .unwrap();
    let observers = serde_json::json!({"complete": true, "error": null, "devices": []});
    let closing = data.close().unwrap();
    let path = closing
        .commit(
            StopReason::RecordStop,
            vec![super::Participant {
                id: "observers",
                error: None,
                section: observers,
            }],
        )
        .unwrap();
    let new: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.with_extension("json")).unwrap()).unwrap();
    let old: serde_json::Value = serde_json::from_slice(
        &std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/recording/sidecar.json"
        ))
        .unwrap(),
    )
    .unwrap();
    let timed = [
        "/tool",
        "/clock/qpc",
        "/clock/qpc_frequency",
        "/clock/started_local",
        "/clock/system_us",
        "/timing/anchors",
        "/timing/host_qpc_frequency",
        "/timing/anchor_summary/max_bracket_us",
        "/timing/anchor_summary/max_mapping_change_us",
        // one more sample for every second the recording runs
        "/timing/anchor_summary/samples",
        "/paired/shift_us",
        "/health/records",
        "/health/reports",
    ];
    fn walk(old: &serde_json::Value, new: &serde_json::Value, at: String, timed: &[&str]) {
        use serde_json::Value::*;
        match (old, new) {
            (Object(a), Object(b)) => {
                let keys = |m: &serde_json::Map<std::string::String, serde_json::Value>| {
                    m.keys().cloned().collect::<Vec<_>>()
                };
                assert_eq!(keys(a), keys(b), "keys at {at}");
                for (key, value) in a {
                    walk(value, &b[key], format!("{at}/{key}"), timed);
                }
            }
            (Number(a), Number(b)) if timed.contains(&at.as_str()) => {
                assert_eq!(a.is_f64(), b.is_f64(), "number kind at {at}")
            }
            (String(_), String(_)) if timed.contains(&at.as_str()) => {}
            _ => assert_eq!(old, new, "at {at}"),
        }
    }
    walk(&old, &new, String::new(), &timed);
}

/// failed wins, then a stop that cut the recording short, then a participant that failed
#[test]
fn the_outcome_is_failed_then_interrupted_then_degraded_then_complete() {
    for (fail, stop, broken, outcome, reason, read) in [
        (
            false,
            StopReason::RecordStop,
            false,
            "complete",
            None,
            "complete",
        ),
        (
            false,
            StopReason::HotkeyStop,
            true,
            "degraded",
            None,
            "degraded",
        ),
        (
            false,
            StopReason::AppExit,
            true,
            "interrupted",
            None,
            "interrupted",
        ),
        (
            false,
            StopReason::CaptureStop,
            false,
            "interrupted",
            None,
            "interrupted",
        ),
        (
            true,
            StopReason::AppExit,
            true,
            "failed",
            Some("device_lost"),
            "failed",
        ),
        // a start that failed never finished, even when nothing else said so
        (
            false,
            StopReason::StartFailed,
            false,
            "failed",
            Some("participant_failed"),
            "failed",
        ),
    ] {
        let folder = TempDir::new("outcome");
        let (mut data, stem) = recording(folder.path(), 50);
        if fail {
            data.fail(Reason::DeviceLost, "selected device disconnected");
        }
        let participants = vec![super::Participant {
            id: "observers",
            error: broken.then(|| "observers lost a batch".to_string()),
            section: serde_json::json!({"complete": !broken}),
        }];
        let committed = data.close().unwrap().commit(stop, participants);
        assert_eq!(committed.is_err(), reason.is_some(), "{outcome}");
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(folder.path().join(format!("{stem}.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["schema"], RECORDING_SCHEMA);
        assert_eq!(manifest["outcome"], outcome);
        assert!(manifest.get("complete").is_none(), "{outcome}");
        assert_eq!(manifest["stop_reason"], serde_json::to_value(stop).unwrap());
        assert_eq!(manifest["reason"], serde_json::to_value(reason).unwrap());
        assert_eq!(
            manifest["participants"],
            serde_json::json!([{
                "id": "observers",
                "outcome": if broken { "failed" } else { "complete" },
                "error": if broken { "observers lost a batch".into() } else { serde_json::Value::Null },
            }])
        );
        assert_eq!(manifest["observers"]["complete"], !broken);
        // and the reader reads back what the writer meant
        let pcap = folder.path().join(format!("{stem}.pcap"));
        let opened = crate::evidence::reader::recording::open_recording(&pcap, None).unwrap();
        let said = crate::evidence::reader::status::summary(&opened.status);
        assert_eq!(said.kind, read, "{outcome} {stop:?}");
    }
}

#[test]
fn a_device_that_never_said_who_it_is_has_no_ids() {
    let folder = TempDir::new("unnamed");
    let traffic = Traffic(
        ticks(1000, 1_000_000, 20)
            .map(|t| (t, Packet::report(1, 2, 0x81, &mouse_payload(1, -1, 7))))
            .collect(),
    );
    let mut data = Data::new(true);
    let mut stem = None;
    pcap::read_pcap(traffic.pcap().bytes(), |record| {
        data.ingest(&record);
        if stem.is_none() && data.health.reports > 0 {
            data.select("1:2:129", Some(crate::testkit::traffic::mouse_decoder()))
                .unwrap();
            data.trust_selection();
            stem = Some(data.begin("test", folder.path(), None).unwrap());
        }
    })
    .unwrap();
    let path = data.finish().unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.with_extension("json")).unwrap()).unwrap();
    assert_eq!(manifest["selection"]["vid"], serde_json::Value::Null);
    assert_eq!(manifest["selection"]["pid"], serde_json::Value::Null);
    assert!(stem.unwrap().starts_with("00000000-"));
}

/// an evidence file that fails at the end keeps the shift out of the input file too, so no
/// file of a failed recording says the two clocks were related
#[test]
fn a_late_evidence_fault_keeps_the_shift_out_of_the_input_file() {
    for kind in ["pcap", "clocks.jsonl", "input-arrivals.csv"] {
        let folder = TempDir::new("late-sync");
        let _fault = inject(kind, Fault::Sync);
        let (mut data, stem) = recording_with(folder.path(), 200, true);
        data.finish().unwrap_err();
        let input =
            std::fs::read_to_string(folder.path().join(format!("{stem}.input.csv"))).unwrap();
        assert!(!input.contains("# paired_shift_us"), "{kind}: {input}");
    }
    // and a recording that finishes writes it
    let folder = TempDir::new("shift-written");
    let (mut data, stem) = recording_with(folder.path(), 200, true);
    data.finish().unwrap();
    let input = std::fs::read_to_string(folder.path().join(format!("{stem}.input.csv"))).unwrap();
    assert!(input.contains("# paired_shift_us"), "{input}");
}

#[test]
fn a_recording_ended_because_it_failed_is_never_complete() {
    let folder = TempDir::new("ended-failed");
    let (mut data, stem) = recording(folder.path(), 50);
    let closing = data.close().unwrap();
    assert!(
        closing
            .commit(StopReason::RecordingFailed, Vec::new())
            .is_err()
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(folder.path().join(format!("{stem}.json"))).unwrap())
            .unwrap();
    assert_eq!(manifest["outcome"], "failed");
    assert_eq!(manifest["stop_reason"], "recording_failed");
}

#[test]
fn the_participant_ids_are_what_is_on_disk() {
    use super::participant::*;
    // what readers of saved recordings look for, so changing a line here breaks them
    assert_eq!(
        [
            OBSERVERS,
            RECORDING_CONTROL,
            MEDIUS_INPUT,
            VISION,
            INJECTION
        ],
        [
            "observers",
            "recording_control",
            "medius_input",
            "vision",
            "injection"
        ]
    );
}

#[test]
fn a_participant_handed_in_at_the_end_keeps_what_went_wrong_during_the_recording() {
    use super::participant::OBSERVERS;
    let folder = TempDir::new("handed-in");
    let (mut data, stem) = recording(folder.path(), 50);
    data.degrade(OBSERVERS, "the observer stopped");
    // the first error is the one kept
    data.degrade(OBSERVERS, "and then something else");
    let section = serde_json::json!({"complete": true});
    data.close()
        .unwrap()
        .commit(
            StopReason::RecordStop,
            vec![super::Participant {
                id: OBSERVERS,
                error: None,
                section: section.clone(),
            }],
        )
        .unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(folder.path().join(format!("{stem}.json"))).unwrap())
            .unwrap();
    assert_eq!(manifest["outcome"], "degraded");
    assert_eq!(
        manifest["participants"],
        serde_json::json!([{"id": "observers", "outcome": "failed", "error": "the observer stopped"}])
    );
    assert_eq!(manifest["observers"], section);
}
