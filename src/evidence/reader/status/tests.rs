use super::*;
use crate::evidence::reason::StopReason;
use crate::testkit::traffic::TempDir;
use serde_json::json;

/// valid metadata with the given outcome
fn manifest(outcome: &str) -> serde_json::Value {
    json!({
        "schema": "motion-bench.recording.v2",
        "outcome": outcome,
        "reason": if outcome == "failed" { json!("device_lost") } else { json!(null) },
        "stop_reason": "app_exit",
        "health": {
            "error": if outcome == "failed" { json!("unplugged") } else { json!(null) },
            "pairing_valid": true,
        },
        "participants": [],
    })
}

fn valid(value: serde_json::Value) -> Metadata {
    Metadata::Valid(value)
}

/// the metadata decides first, a failed outcome even before the pcap is looked at; complete,
/// degraded and interrupted still answer to the pcap
#[test]
fn the_metadata_decides_and_the_pcap_can_still_fail_it() {
    let folder = TempDir::new("status");
    std::fs::create_dir_all(folder.path()).unwrap();
    let run = folder.path().join("run.pcap");
    // markers beside valid metadata only lost the race to be removed
    std::fs::write(
        run.with_extension("incomplete.json"),
        r#"{"reason":"writer_failed","error":"disk full"}"#,
    )
    .unwrap();
    std::fs::write(run.with_extension("active.json"), "{}").unwrap();
    let cut = Some((None, "the capture file is cut short".to_string()));
    assert_eq!(
        classify(&run, &valid(manifest("complete")), None),
        Status::Complete
    );
    assert_eq!(
        classify(&run, &valid(manifest("failed")), cut.clone()),
        Status::Failed {
            reason: Some(Reason::DeviceLost),
            message: "unplugged".into()
        }
    );
    for outcome in ["complete", "degraded", "interrupted"] {
        assert_eq!(
            classify(&run, &valid(manifest(outcome)), cut.clone()),
            Status::Failed {
                reason: None,
                message: "the capture file is cut short".into()
            },
            "{outcome}"
        );
    }
    assert_eq!(
        classify(&run, &valid(manifest("interrupted")), None),
        Status::Interrupted {
            stop: Some(StopReason::AppExit),
            degraded: vec![]
        }
    );
    // only a stop that cuts it short makes an interrupted recording comparable, a missing or
    // ordinary one leaves it like a crash leftover
    for stop in [json!(null), json!("record_stop"), json!("power_cut")] {
        let mut odd = manifest("interrupted");
        odd["stop_reason"] = stop.clone();
        assert_eq!(
            classify(&run, &valid(odd), None),
            Status::Interrupted {
                stop: None,
                degraded: vec![]
            },
            "{stop}"
        );
    }
}

#[test]
fn what_failed_beside_whole_usb_evidence_degrades_it() {
    let run = Path::new("run.pcap");
    let mut lost = manifest("degraded");
    lost["health"]["pairing_valid"] = false.into();
    lost["participants"] = json!([
        {"id": "observers", "outcome": "failed", "error": "lost"},
        {"id": "vision", "outcome": "failed", "error": "stopped"},
        // the box's input failing is already said by the pairing
        {"id": "medius_input", "outcome": "failed", "error": "reset"},
    ]);
    assert_eq!(
        classify(run, &valid(lost.clone()), None),
        Status::Degraded(vec![
            Degradation::PairingInvalid,
            Degradation::ParticipantFailed,
            Degradation::TargetsFailed,
        ])
    );
    // what was already wrong stays with an interrupted recording
    lost["outcome"] = "interrupted".into();
    lost["participants"] = json!([]);
    assert_eq!(
        classify(run, &valid(lost), None),
        Status::Interrupted {
            stop: Some(StopReason::AppExit),
            degraded: vec![Degradation::PairingInvalid]
        }
    );
    // degraded stays degraded even when it does not say what failed
    assert_eq!(
        classify(run, &valid(manifest("degraded")), None),
        Status::Degraded(vec![Degradation::ParticipantFailed])
    );
}

/// without metadata, the failure marker the writer leaves when publishing fails says how, and
/// the marker a running recording keeps says it never finished
#[test]
fn without_metadata_the_markers_say_how_it_ended() {
    let folder = TempDir::new("markers");
    std::fs::create_dir_all(folder.path()).unwrap();
    let run = folder.path().join("run.pcap");
    let incomplete = run.with_extension("incomplete.json");
    std::fs::write(run.with_extension("active.json"), "{}").unwrap();
    std::fs::write(
        &incomplete,
        r#"{"reason":"writer_failed","error":"disk full"}"#,
    )
    .unwrap();
    assert_eq!(
        classify(&run, &Metadata::Missing, None),
        Status::Failed {
            reason: Some(Reason::WriterFailed),
            message: "disk full".into()
        }
    );
    std::fs::write(&incomplete, "not json").unwrap();
    assert_eq!(
        classify(&run, &Metadata::Missing, None),
        Status::Failed {
            reason: None,
            message: "the recording did not finish".into()
        }
    );
    std::fs::remove_file(&incomplete).unwrap();
    assert_eq!(
        classify(&run, &Metadata::Missing, None),
        Status::Interrupted {
            stop: None,
            degraded: vec![]
        }
    );
    // a marker that is there but cannot be read still means it did not finish
    let locked = folder.path().join("locked.pcap");
    std::fs::create_dir(locked.with_extension("incomplete.json")).unwrap();
    assert!(matches!(
        classify(&locked, &Metadata::Missing, None),
        Status::Failed { reason: None, .. }
    ));
    assert_eq!(
        classify(&folder.path().join("bare.pcap"), &Metadata::Missing, None),
        Status::Failed {
            reason: None,
            message: "nothing beside the capture says how it ended".into()
        }
    );
}

#[test]
fn metadata_that_cannot_be_trusted_fails_the_recording_and_says_why() {
    let run = Path::new("run.pcap");
    assert_eq!(
        classify(run, &Metadata::Corrupt("eof".into()), None),
        Status::Failed {
            reason: None,
            message: "its metadata cannot be read (eof)".into()
        }
    );
    assert_eq!(
        classify(run, &Metadata::Unsupported("v9".into()), None),
        Status::Failed {
            reason: None,
            message: "its metadata is from a newer version (v9)".into()
        }
    );
}

#[test]
fn metadata_tells_missing_corrupt_unsupported_and_valid_apart() {
    let folder = TempDir::new("metadata");
    std::fs::create_dir_all(folder.path()).unwrap();
    let path = folder.path().join("run.json");
    assert_eq!(Metadata::read(&path), Metadata::Missing);
    for broken in ["", "{", "[]", "null"] {
        std::fs::write(&path, broken).unwrap();
        assert!(
            matches!(Metadata::read(&path), Metadata::Corrupt(_)),
            "{broken:?}"
        );
    }
    std::fs::write(&path, manifest("complete").to_string()).unwrap();
    assert!(Metadata::read(&path).valid().is_some());
    // metadata of any other shape is not this version's
    for other in [
        json!({"complete": true}),
        json!({"schema": "motion-bench.recording.v1", "outcome": "complete"}),
    ] {
        std::fs::write(&path, other.to_string()).unwrap();
        assert_eq!(
            Metadata::read(&path),
            Metadata::Corrupt("it is not the metadata this version writes".into()),
            "{other}"
        );
    }
    let mut paused = manifest("complete");
    paused["outcome"] = "paused".into();
    std::fs::write(&path, paused.to_string()).unwrap();
    assert!(matches!(Metadata::read(&path), Metadata::Corrupt(why) if why.contains("paused")));
    // a version this build cannot even parse is no older than it
    for schema in [
        "motion-bench.recording.v3",
        "motion-bench.recording.v3.0",
        "motion-bench.timing.v2",
        "motion-bench.timing.v1-beta",
    ] {
        let mut newer = manifest("complete");
        if schema.contains("timing") {
            newer["timing"] = json!({ "schema": schema });
        } else {
            newer["schema"] = schema.into();
        }
        std::fs::write(&path, newer.to_string()).unwrap();
        assert_eq!(
            Metadata::read(&path),
            Metadata::Unsupported(schema.into()),
            "{schema}"
        );
    }
    // one that is there but cannot be read is not missing
    let folder_as_sidecar = folder.path().join("folder.json");
    std::fs::create_dir(&folder_as_sidecar).unwrap();
    assert!(matches!(
        Metadata::read(&folder_as_sidecar),
        Metadata::Corrupt(_)
    ));
}

#[test]
fn participants_come_from_the_list_and_one_that_did_not_complete_failed() {
    let found = participants(Some(&json!({
        "participants": [
            {"id": "observers", "outcome": "complete", "error": null},
            {"id": "recording_control", "outcome": "failed", "error": null},
            {"id": "screen", "outcome": "failed", "error": "stopped early"},
        ],
        // the sections are kept for what they hold, the list decides
        "observers": {"complete": false},
    })));
    assert_eq!(
        found,
        vec![
            Participant {
                name: "observers".into(),
                failure: None
            },
            Participant {
                name: "recording_control".into(),
                failure: Some("recording_control did not complete".into())
            },
            Participant {
                name: "screen".into(),
                failure: Some("stopped early".into())
            },
        ]
    );
    assert!(participants(Some(&json!({"observers": {"complete": false}}))).is_empty());
    assert!(participants(None).is_empty());
}

#[test]
fn the_summary_names_the_kind_and_says_why() {
    let failed = Status::Failed {
        reason: Some(Reason::DeviceLost),
        message: "unplugged".into(),
    };
    assert_eq!(
        summary(&failed),
        Summary {
            kind: "failed",
            reason: Some(Reason::DeviceLost),
            message: Some("unplugged".into())
        }
    );
    assert_eq!(summary(&Status::Complete).kind, "complete");
    let degraded = summary(&Status::Degraded(vec![
        Degradation::PairingInvalid,
        Degradation::ParticipantFailed,
    ]));
    assert_eq!(degraded.kind, "degraded");
    assert_eq!(
        degraded.message.unwrap(),
        format!(
            "{}, {}",
            Degradation::PairingInvalid.describe(),
            Degradation::ParticipantFailed.describe()
        )
    );
    // the code is written the way the recording wrote it
    assert_eq!(
        serde_json::to_value(summary(&failed)).unwrap()["reason"],
        "device_lost"
    );
}

#[test]
fn an_interrupted_recording_says_what_cut_it_short() {
    let words = |stop| {
        summary(&Status::Interrupted {
            stop,
            degraded: vec![],
        })
    };
    assert_eq!(words(None).kind, "interrupted");
    assert_eq!(
        words(None).message.unwrap(),
        "it never finished, the app or the computer stopped while it was recording"
    );
    assert_eq!(
        words(Some(StopReason::CaptureStop)).message.unwrap(),
        "usb capture was stopped while it was recording"
    );
}
