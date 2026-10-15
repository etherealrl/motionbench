//! the recording metadata shapes this version writes, plus broken and future ones, run through
//! the readers. a reader may refuse a file but it must never panic or change a byte on disk

use crate::{
    analysis::views::plot::{Views, plot},
    evidence::{
        reader::{
            eligibility::{COMPARISON, PAIRED, TARGETS, eligibility},
            layout::locate,
            recording::open_recording,
            status::{Degradation, Metadata, Status, assess},
        },
        reason::{Reason, StopReason},
    },
    testkit::traffic::{Packet, Pcap, TempDir, fixture, mouse_payload},
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn corpus(group: &str) -> Vec<PathBuf> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/corpus")
        .join(group);
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    assert!(!files.is_empty());
    files
}

fn named(group: &str, name: &str) -> PathBuf {
    corpus(group)
        .into_iter()
        .find(|p| p.ends_with(name))
        .unwrap()
}

/// every file in `folder` with its bytes, so a comparison also catches files added or removed
fn files(folder: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect()
}

/// the 1 kHz fixture capture as `recording.pcap`, followed by 50 reports from a second mouse
/// at 1:5 so only the sidecar can say which one was recorded, with the given files beside it
fn recording(folder: &Path, beside: &[(&str, &Path)]) -> PathBuf {
    std::fs::create_dir_all(folder).unwrap();
    let pcap = folder.join("recording.pcap");
    let mut other = Pcap::default();
    for i in 0..50 {
        let report = Packet::report(1, 5, 0x81, &mouse_payload(2, -2, 7));
        other.record(1_784_571_480_000_000 + i * 1000, &report.bytes());
    }
    let mut bytes = fixture("mouse-1000hz.pcap");
    bytes.extend_from_slice(&other.bytes()[24..]);
    std::fs::write(&pcap, bytes).unwrap();
    for (extension, source) in beside {
        std::fs::copy(source, pcap.with_extension(extension)).unwrap();
    }
    pcap
}

/// every shape opens, says how the recording ended and plots, and no byte changes
#[test]
fn every_metadata_shape_opens_and_says_how_it_ended_without_changing_a_byte() {
    let mut cases: Vec<(PathBuf, &str)> = Vec::new();
    for group in ["recording-v2", "malformed", "unsupported"] {
        for sidecar in corpus(group) {
            cases.push((sidecar, "json"));
        }
    }
    cases.push((
        named("interrupted", "recorder.incomplete.json"),
        "incomplete.json",
    ));
    cases.push((named("marker", "active.json"), "active.json"));
    for (file, extension) in cases {
        let folder = TempDir::new("corpus-open");
        let pcap = recording(folder.path(), &[(extension, &file)]);
        let before = files(folder.path());
        let opened = open_recording(&pcap, None).unwrap_or_else(|e| panic!("{file:?}: {e}"));
        let name = file.file_name().unwrap().to_string_lossy().to_string();
        let kind = match &opened.status {
            Status::Complete => "complete",
            Status::Degraded(_) => "degraded",
            Status::Failed { .. } => "failed",
            Status::Interrupted { .. } => "interrupted",
        };
        let expected = match name.as_str() {
            "complete.json" => "complete",
            "degraded.json" => "degraded",
            "interrupted-app-exit.json" | "interrupted-capture-stop.json" | "active.json" => {
                "interrupted"
            }
            // a bad selection or decoder does not change how the recording ended
            "decoder-unknown.json" | "selection-null.json" | "selection-strings.json" => "complete",
            _ => "failed",
        };
        assert_eq!(kind, expected, "{file:?}: {:?}", opened.status);
        if name.starts_with("selection-") {
            assert!(opened.selection_error.is_some(), "{file:?}");
        }
        if file.parent().unwrap().ends_with("unsupported") {
            assert!(matches!(opened.metadata, Metadata::Unsupported(_)));
        }
        plot(&opened.into_data(), None, None, false, Views::All);
        assert_eq!(files(folder.path()), before, "{file:?}");
    }
}

#[test]
fn event_data_loads_without_changing_a_byte() {
    let folder = TempDir::new("corpus-events");
    std::fs::create_dir_all(folder.path()).unwrap();
    let path = folder.path().join("events.npz");
    std::fs::write(&path, fixture("events.npz")).unwrap();
    let before = files(folder.path());
    crate::analysis::dataset::Dataset::load(&path).unwrap();
    assert_eq!(files(folder.path()), before);
}

/// set MOTION_BENCH_CORPUS to a recordings folder to run the same checks on real recordings.
/// each recording and the files beside it are copied to a temporary folder first, so the real
/// folder is never touched even if a reader misbehaves
#[test]
fn local_recordings_load_without_changing_a_byte() {
    let Some(source) = std::env::var_os("MOTION_BENCH_CORPUS") else {
        return;
    };
    let source = PathBuf::from(source);
    let names: Vec<String> = std::fs::read_dir(&source)
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            entry.file_type().ok()?.is_file().then_some(())?;
            entry.file_name().into_string().ok()
        })
        .collect();
    assert!(
        names.iter().any(|name| name.ends_with(".pcap")),
        "no recordings in {source:?}"
    );
    let (mut loaded, mut refused) = (0, Vec::new());
    for pcap in names.iter().filter(|name| name.ends_with(".pcap")) {
        let stem = pcap.trim_end_matches(".pcap");
        let folder = TempDir::new("local-corpus");
        std::fs::create_dir_all(folder.path()).unwrap();
        for name in names.iter().filter(|n| n.starts_with(&format!("{stem}."))) {
            std::fs::copy(source.join(name), folder.path().join(name)).unwrap();
        }
        let before = files(folder.path());
        match open_recording(&folder.path().join(pcap), None) {
            Ok(opened) => {
                plot(&opened.into_data(), None, None, false, Views::All);
                loaded += 1;
            }
            Err(error) => refused.push(format!("{pcap}: {error}")),
        }
        assert_eq!(files(folder.path()), before, "{pcap}");
    }
    eprintln!(
        "{loaded} recordings loaded, {} refused:\n{}",
        refused.len(),
        refused.join("\n")
    );
}

#[test]
fn the_metadata_and_the_markers_decide_by_their_own_rules() {
    let v2 = |name: &str| Some(named("recording-v2", &format!("{name}.json")));
    let failure = named("interrupted", "recorder.incomplete.json");
    let marker = named("marker", "active.json");
    let failed = |reason| Status::Failed {
        reason: Some(reason),
        message: String::new(),
    };
    let interrupted = |stop| Status::Interrupted {
        stop,
        degraded: vec![],
    };
    let all = [true; 3];
    let none = [false; 3];
    // sidecar, failure file, marker, status, and the comparison, paired and target decisions
    // the box input is taken as saved, the files beside the capture decide
    type Row = (Option<PathBuf>, bool, bool, Status, [bool; 3]);
    let rows: Vec<Row> = vec![
        (v2("complete"), false, false, Status::Complete, all),
        // a marker that lost the race to be removed and a stray failure file lose to the metadata
        (v2("complete"), true, true, Status::Complete, all),
        (
            v2("degraded"),
            false,
            true,
            Status::Degraded(vec![Degradation::ParticipantFailed]),
            all,
        ),
        (v2("failed"), false, true, failed(Reason::DeviceLost), none),
        // cut short with its files saved: compared, but never used as a finished recording
        (
            v2("interrupted-app-exit"),
            false,
            true,
            interrupted(Some(StopReason::AppExit)),
            [true, true, false],
        ),
        (
            v2("interrupted-capture-stop"),
            false,
            false,
            interrupted(Some(StopReason::CaptureStop)),
            [true, true, false],
        ),
        // what a crash leaves: only the marker, view only
        (None, false, true, interrupted(None), none),
        // a sidecar that could not be published leaves the marker and the code
        (None, true, true, failed(Reason::WriterFailed), none),
    ];
    for (sidecar, incomplete, active, expected, decisions) in rows {
        let folder = TempDir::new("corpus-current");
        let mut beside = Vec::new();
        if let Some(sidecar) = &sidecar {
            beside.push(("json", sidecar.as_path()));
        }
        if incomplete {
            beside.push(("incomplete.json", failure.as_path()));
        }
        if active {
            beside.push(("active.json", marker.as_path()));
        }
        let pcap = recording(folder.path(), &beside);
        let before = files(folder.path());
        let row = format!("{sidecar:?} incomplete {incomplete} marker {active}");
        let opened = open_recording(&pcap, None).unwrap();
        let (metadata, assessed) = assess(&locate(&pcap).unwrap());
        for status in [&opened.status, &assessed] {
            match (status, &expected) {
                (Status::Failed { reason, .. }, Status::Failed { reason: want, .. }) => {
                    assert_eq!(reason, want, "{row}")
                }
                _ => assert_eq!(status, &expected, "{row}"),
            }
        }
        for (needs, want) in [COMPARISON, PAIRED, TARGETS].into_iter().zip(decisions) {
            assert_eq!(
                eligibility(&metadata, &assessed, PAIRED.streams, needs).is_ok(),
                want,
                "{row} {needs:?}"
            );
        }
        assert_eq!(files(folder.path()), before, "{row}");
    }
    // a capture with nothing beside it was not made here, and is not opened at all
    let folder = TempDir::new("corpus-bare");
    let pcap = recording(folder.path(), &[]);
    assert!(open_recording(&pcap, None).is_err());
}
