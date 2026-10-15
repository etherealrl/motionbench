use super::*;

// what lands on disk, so a renamed variant cannot change a saved code unnoticed
const GOLDEN: [(Reason, &str); 13] = [
    (Reason::WriterFailed, "writer_failed"),
    (Reason::DescriptorCacheFull, "descriptor_cache_full"),
    (Reason::DeviceLost, "device_lost"),
    (Reason::TimestampRegressed, "timestamp_regressed"),
    (Reason::DeviceReenumerated, "device_reenumerated"),
    (Reason::AddressReused, "address_reused"),
    (Reason::MetadataFailed, "metadata_failed"),
    (Reason::TransportFailed, "transport_failed"),
    (Reason::TransportEnded, "transport_ended"),
    (Reason::DriverReported, "driver_reported"),
    (Reason::ParticipantFailed, "participant_failed"),
    (Reason::ClockFailed, "clock_failed"),
    (Reason::Unknown, "unknown"),
];

#[test]
fn every_code_is_pinned_and_round_trips() {
    for (reason, code) in GOLDEN {
        assert_eq!(serde_json::to_value(reason).unwrap(), code);
        assert_eq!(
            serde_json::from_value::<Reason>(code.into()).unwrap(),
            reason
        );
    }
}

#[test]
fn every_code_stops_what_it_belongs_to() {
    let scopes = [
        (Reason::WriterFailed, Scope::Recording),
        (Reason::DescriptorCacheFull, Scope::Pipeline),
        (Reason::DeviceLost, Scope::Source),
        (Reason::TimestampRegressed, Scope::Source),
        (Reason::DeviceReenumerated, Scope::Source),
        (Reason::AddressReused, Scope::Source),
        (Reason::MetadataFailed, Scope::Recording),
        (Reason::TransportFailed, Scope::Pipeline),
        (Reason::TransportEnded, Scope::Pipeline),
        (Reason::DriverReported, Scope::Pipeline),
        (Reason::ParticipantFailed, Scope::Participant),
        (Reason::ClockFailed, Scope::Recording),
        (Reason::Unknown, Scope::Pipeline),
    ];
    assert_eq!(scopes.len(), GOLDEN.len());
    for (reason, scope) in scopes {
        assert_eq!(reason.scope(), scope, "{reason:?}");
    }
}

const STOPS: [(StopReason, &str); 7] = [
    (StopReason::RecordStop, "record_stop"),
    (StopReason::HotkeyStop, "hotkey_stop"),
    (StopReason::StartFailed, "start_failed"),
    (StopReason::CaptureStop, "capture_stop"),
    (StopReason::AppExit, "app_exit"),
    (StopReason::RecordingFailed, "recording_failed"),
    (StopReason::Unknown, "unknown"),
];

#[test]
fn every_stop_code_is_pinned_and_only_capture_and_exit_interrupt() {
    for (stop, code) in STOPS {
        assert_eq!(serde_json::to_value(stop).unwrap(), code);
        assert_eq!(
            serde_json::from_value::<StopReason>(code.into()).unwrap(),
            stop
        );
        assert_eq!(
            stop.interrupts(),
            ["capture_stop", "app_exit"].contains(&code)
        );
    }
    assert_eq!(
        serde_json::from_value::<StopReason>("something_new".into()).unwrap(),
        StopReason::Unknown
    );
}

#[test]
fn a_code_from_a_newer_version_reads_as_unknown() {
    assert_eq!(
        serde_json::from_value::<Reason>("something_new".into()).unwrap(),
        Reason::Unknown
    );
}
