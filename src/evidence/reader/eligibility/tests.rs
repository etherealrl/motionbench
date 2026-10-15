use super::*;

/// everything an analysis could read beside the capture
const SAVED: &[Stream] = &[Stream::PhysicalInput];

#[test]
fn each_analysis_takes_only_what_it_can_use() {
    let valid = Metadata::Valid(serde_json::json!({"outcome": "complete"}));
    let failed = Status::Failed {
        reason: Some(Reason::DeviceLost),
        message: "unplugged".into(),
    };
    let pairing = Status::Degraded(vec![Degradation::PairingInvalid]);
    let targets = Status::Degraded(vec![Degradation::TargetsFailed]);
    let participant = Status::Degraded(vec![Degradation::ParticipantFailed]);
    let unreadable = Status::Failed {
        reason: None,
        message: "its metadata cannot be read (x)".into(),
    };
    // (metadata, status, comparison, paired, targets)
    let table = [
        (&valid, &Status::Complete, true, true, true),
        (&valid, &failed, false, false, false),
        (&valid, &pairing, true, false, true),
        (&valid, &targets, true, true, false),
        // the usb evidence is whole, every analysis that reads it can use it
        (&valid, &participant, true, true, true),
        (
            &Metadata::Corrupt("x".into()),
            &unreadable,
            false,
            false,
            false,
        ),
        (
            &Metadata::Unsupported("v9".into()),
            &unreadable,
            false,
            false,
            false,
        ),
    ];
    for (metadata, status, comparison, paired, targets) in table {
        for (needs, expected) in [
            (COMPARISON, comparison),
            (PAIRED, paired),
            (TARGETS, targets),
        ] {
            assert_eq!(
                eligibility(metadata, status, SAVED, needs).is_ok(),
                expected,
                "{metadata:?} {status:?} {needs:?}"
            );
        }
    }
}

#[test]
fn a_refusal_keeps_the_recordings_own_code_and_says_why() {
    let refusal = eligibility(
        &Metadata::Missing,
        &Status::Failed {
            reason: Some(Reason::TimestampRegressed),
            message: "USBPcap timestamp regressed".into(),
        },
        SAVED,
        COMPARISON,
    )
    .unwrap_err();
    assert_eq!(
        refusal,
        Refusal::Failed {
            reason: Some(Reason::TimestampRegressed),
            message: "USBPcap timestamp regressed".into()
        }
    );
    assert!(refusal.to_string().contains("USBPcap timestamp regressed"));
    // corrupt metadata is refused before a status that would pass
    assert_eq!(
        eligibility(
            &Metadata::Corrupt("eof".into()),
            &Status::Complete,
            SAVED,
            COMPARISON
        ),
        Err(Refusal::CorruptMetadata("eof".into()))
    );
}

/// cut short with its files saved, it is compared but never taken for a finished recording;
/// with only the marker left it is refused everywhere
#[test]
fn an_interrupted_recording_is_compared_only_when_its_files_were_saved() {
    let saved = Status::Interrupted {
        stop: Some(crate::evidence::reason::StopReason::AppExit),
        degraded: vec![],
    };
    assert_eq!(
        eligibility(&Metadata::Missing, &saved, SAVED, COMPARISON),
        Ok(())
    );
    assert_eq!(
        eligibility(&Metadata::Missing, &saved, SAVED, PAIRED),
        Ok(())
    );
    let refusal = eligibility(&Metadata::Missing, &saved, SAVED, TARGETS).unwrap_err();
    assert_eq!(
        refusal.to_string(),
        "the app closed while it was recording, and this needs a recording that finished"
    );
    // lost pairing still refuses the paired analysis
    let unpaired = Status::Interrupted {
        stop: Some(crate::evidence::reason::StopReason::AppExit),
        degraded: vec![Degradation::PairingInvalid],
    };
    assert_eq!(
        eligibility(&Metadata::Missing, &unpaired, SAVED, COMPARISON),
        Ok(())
    );
    assert_eq!(
        eligibility(&Metadata::Missing, &unpaired, SAVED, PAIRED),
        Err(Refusal::Degraded(Degradation::PairingInvalid))
    );
    let crashed = Status::Interrupted {
        stop: None,
        degraded: vec![],
    };
    for needs in [COMPARISON, PAIRED, TARGETS] {
        assert_eq!(
            eligibility(&Metadata::Missing, &crashed, SAVED, needs),
            Err(Refusal::Unfinished)
        );
    }
}

/// a sidecar of the current shape whose outcome this version does not know, or that has none,
/// cannot be trusted at all: it is refused everywhere, whatever else lies beside it
#[test]
fn an_unknown_outcome_is_refused_everywhere() {
    let folder = crate::testkit::traffic::TempDir::new("unknown-outcome");
    std::fs::create_dir_all(folder.path()).unwrap();
    let pcap = folder.path().join("run.pcap");
    for outcome in [serde_json::json!("paused"), serde_json::json!(null)] {
        std::fs::write(
            pcap.with_extension("json"),
            serde_json::json!({
                "schema": "motion-bench.recording.v2",
                "outcome": outcome,
            })
            .to_string(),
        )
        .unwrap();
        let metadata = Metadata::read(&pcap.with_extension("json"));
        assert!(matches!(metadata, Metadata::Corrupt(_)), "{metadata:?}");
        let status = super::super::status::classify(&pcap, &metadata, None);
        for needs in [COMPARISON, PAIRED, TARGETS] {
            assert!(
                matches!(
                    eligibility(&metadata, &status, SAVED, needs),
                    Err(Refusal::CorruptMetadata(_))
                ),
                "{outcome} {needs:?}"
            );
        }
    }
}

/// a paired comparison reads the box's record of the mouse, a recording without one is refused
/// for it alone, and how a failed one ended still comes first
#[test]
fn a_paired_comparison_needs_the_physical_input() {
    for needs in [COMPARISON, TARGETS] {
        assert_eq!(
            eligibility(&Metadata::Missing, &Status::Complete, &[], needs),
            Ok(())
        );
    }
    let refusal = eligibility(&Metadata::Missing, &Status::Complete, &[], PAIRED).unwrap_err();
    assert_eq!(refusal, Refusal::Missing(Stream::PhysicalInput));
    assert!(refusal.to_string().contains("no physical input"));
    let failed = Status::Failed {
        reason: None,
        message: "unplugged".into(),
    };
    assert!(matches!(
        eligibility(&Metadata::Missing, &failed, &[], PAIRED),
        Err(Refusal::Failed { .. })
    ));
}
