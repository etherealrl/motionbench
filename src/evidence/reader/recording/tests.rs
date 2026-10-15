use super::*;
use crate::evidence::reader::status::{Degradation, Participant};
use crate::evidence::reason::Reason;
use crate::testkit::fixtures::{recorded, sidecar};
use crate::testkit::traffic::{
    Descriptors, Packet, TempDir, Traffic, mouse_decoder, mouse_payload, ticks,
};
use std::path::PathBuf;

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn mouse_train(count: u64) -> Traffic {
    let mut traffic = Traffic::default();
    traffic.describe(0, 1, 2, &Descriptors::mouse(0x373b, 0x11d9));
    traffic.extend(
        ticks(1000, 1_000_000, count)
            .map(|t| (t, Packet::report(1, 2, 0x81, &mouse_payload(1, -1, 7)))),
    );
    traffic
}

/// the traffic as `run.pcap` in `folder`, with `metadata` beside it when there is any
fn write(folder: &TempDir, traffic: &Traffic, metadata: Option<serde_json::Value>) -> PathBuf {
    std::fs::create_dir_all(folder.path()).unwrap();
    let path = folder.path().join("run.pcap");
    std::fs::write(&path, traffic.pcap().bytes()).unwrap();
    if let Some(metadata) = metadata {
        std::fs::write(path.with_extension("json"), metadata.to_string()).unwrap();
    }
    path
}

/// the real 1 kHz capture with its metadata: complete, its only mouse picked, its saved layout
/// decoding it
#[test]
fn the_fixture_recording_opens_complete_with_its_saved_layout() {
    let opened = open_recording(&fixture("mouse-1000hz.pcap"), None).unwrap();
    assert_eq!(opened.status, Status::Complete);
    let key = opened.data().selected.unwrap();
    assert_eq!((key.bus, key.device, key.endpoint), (1, 31, 0x81));
    let Some(Decoder::WindowsDescriptor(saved)) = &opened.streams[0].decoder else {
        panic!("{:?}", opened.streams[0].decoder);
    };
    let Decoder::Descriptor(test_layout) = mouse_decoder() else {
        unreachable!()
    };
    assert_eq!(saved, &test_layout);
    assert_eq!(opened.data().health.reports, 400);
}

#[test]
fn a_failed_capture_still_opens_with_its_failure() {
    let folder = TempDir::new("open-failed");
    let mut traffic = mouse_train(100);
    traffic.add(
        1_050_500,
        Packet::report(1, 2, 0x81, &mouse_payload(1, 1, 7)),
    );
    let path = write(&folder, &traffic, Some(sidecar("complete")));
    let opened = open_recording(&path, None).unwrap();
    assert_eq!(
        opened.status,
        Status::Failed {
            reason: Some(Reason::TimestampRegressed),
            message: "USBPcap timestamp regressed".into()
        }
    );
    assert_eq!(opened.data().health.reports, 100);
    // without metadata, the failure the writer left in its marker says how it ended
    std::fs::remove_file(path.with_extension("json")).unwrap();
    std::fs::write(
        path.with_extension("incomplete.json"),
        r#"{"reason":"device_lost","error":"unplugged"}"#,
    )
    .unwrap();
    assert!(matches!(
        open_recording(&path, None).unwrap().status,
        Status::Failed {
            reason: Some(Reason::DeviceLost),
            ..
        }
    ));
}

#[test]
fn streams_and_participants_come_from_the_files_beside_the_pcap() {
    let folder = TempDir::new("open-streams");
    let mut metadata = sidecar("degraded");
    metadata["health"]["pairing_valid"] = false.into();
    metadata["participants"] = serde_json::json!([
        {"id": "observers", "outcome": "failed", "error": "raw input lost events"},
        {"id": "recording_control", "outcome": "complete", "error": null},
    ]);
    let path = write(&folder, &mouse_train(50), Some(metadata));
    let opened = open_recording(&path, None).unwrap();
    assert_eq!(
        opened.status,
        Status::Degraded(vec![
            Degradation::PairingInvalid,
            Degradation::ParticipantFailed
        ])
    );
    assert_eq!(opened.streams.len(), 1);
    let wire = &opened.streams[0];
    assert_eq!(wire.id, "usb");
    assert_eq!(wire.artifact, path);
    assert_eq!(wire.role, Role::SelectedOutput);
    assert_eq!(wire.clock, Clock::UsbpcapUtc);
    assert_eq!(wire.endpoint.map(|k| k.endpoint), Some(0x81));
    assert!(matches!(wire.decoder, Some(Decoder::Descriptor(_))));
    assert_eq!(
        opened.participants,
        vec![
            Participant {
                name: "observers".into(),
                failure: Some("raw input lost events".into())
            },
            Participant {
                name: "recording_control".into(),
                failure: None
            },
        ]
    );

    std::fs::write(path.with_extension("input.csv"), "box_us,dx,dy,dz\n").unwrap();
    let opened = open_recording(&path, None).unwrap();
    assert_eq!(opened.streams[1].id, "medius_input");
    assert_eq!(opened.streams[1].artifact, path.with_extension("input.csv"));
    assert_eq!(opened.streams[1].role, Role::PhysicalInput);
    assert_eq!(opened.streams[1].clock, Clock::MediusBox);
}

#[test]
fn a_capture_made_elsewhere_or_unreadable_is_the_only_error() {
    let folder = TempDir::new("open-unreadable");
    std::fs::create_dir_all(folder.path()).unwrap();
    assert!(open_recording(&folder.path().join("missing.pcap"), None).is_err());
    // a capture with nothing beside it was not recorded here
    let bare = write(&folder, &mouse_train(10), None);
    assert!(
        open_recording(&bare, None)
            .err()
            .unwrap()
            .contains("not recorded by Motion Bench")
    );
    let path = folder.path().join("truncated.pcap");
    std::fs::write(&path, [0xd4, 0xc3, 0xb2]).unwrap();
    std::fs::write(path.with_extension("json"), sidecar("complete").to_string()).unwrap();
    assert!(open_recording(&path, None).is_err());
}

#[test]
fn descriptors_are_read_once_and_an_override_beats_the_saved_decoder() {
    let folder = TempDir::new("open-golden");
    let capture = recorded(folder.path(), "descriptors.pcap", &sidecar("complete"));
    let mut records = 0;
    pcap::read_pcap(&std::fs::read(&capture).unwrap()[..], |record| {
        if matches!(record, Record::Packet { .. }) {
            records += 1;
        }
    })
    .unwrap();
    // the descriptor pass does not count twice
    let opened = open_recording(&capture, None).unwrap();
    assert_eq!(opened.data().health.records, records);

    let mut unavailable = sidecar("complete");
    unavailable["decoder"] = "Unavailable".into();
    let path = write(&folder, &mouse_train(50), Some(unavailable));
    assert!(matches!(
        open_recording(&path, None).unwrap().streams[0].decoder,
        Some(Decoder::Unavailable)
    ));
    let opened = open_recording(&path, Some(mouse_decoder())).unwrap();
    assert!(matches!(
        opened.streams[0].decoder,
        Some(Decoder::Descriptor(_))
    ));
}

#[test]
fn a_capture_cut_short_opens_as_failed_with_what_came_before_the_cut() {
    let folder = TempDir::new("open-torn");
    let path = write(&folder, &mouse_train(100), Some(sidecar("complete")));
    let bytes = std::fs::read(&path).unwrap();
    std::fs::write(&path, &bytes[..bytes.len() - 5]).unwrap();
    let opened = open_recording(&path, None).unwrap();
    assert!(matches!(
        &opened.status,
        Status::Failed { reason: None, message } if message.contains("cut short")
    ));
    assert_eq!(opened.data().health.reports, 99);
    // without metadata, what the writer left in its marker still comes first
    std::fs::remove_file(path.with_extension("json")).unwrap();
    std::fs::write(
        path.with_extension("incomplete.json"),
        r#"{"reason":"writer_failed","error":"disk full"}"#,
    )
    .unwrap();
    assert!(matches!(
        open_recording(&path, None).unwrap().status,
        Status::Failed {
            reason: Some(Reason::WriterFailed),
            ..
        }
    ));
}

#[test]
fn a_device_that_went_away_keeps_what_it_sent_before() {
    let folder = TempDir::new("open-unplugged");
    let mut traffic = mouse_train(100);
    traffic.add(1_200_000, Packet::disconnect(1, 2, 0x81));
    let path = write(&folder, &traffic, None);
    std::fs::write(
        path.with_extension("incomplete.json"),
        r#"{"reason":"device_lost","error":"selected device disconnected"}"#,
    )
    .unwrap();
    let opened = open_recording(&path, None).unwrap();
    assert!(matches!(
        opened.status,
        Status::Failed {
            reason: Some(Reason::DeviceLost),
            ..
        }
    ));
    assert_eq!(opened.data().devices().len(), 1);
    assert_eq!(opened.streams[0].endpoint.map(|k| k.device), Some(2));
    assert_eq!(opened.data().health.reports, 100);
}

/// a sidecar from a newer version is not trusted for anything, its saved layout and device
/// included: the recording opens failed, decoded only by what the capture itself holds
#[test]
fn a_newer_sidecar_is_not_read_at_all() {
    let folder = TempDir::new("open-newer");
    let mut newer = sidecar("complete");
    newer["schema"] = "motion-bench.recording.v3".into();
    let path = write(&folder, &mouse_train(50), Some(newer));
    let opened = open_recording(&path, None).unwrap();
    assert!(matches!(opened.metadata, Metadata::Unsupported(_)));
    assert!(matches!(opened.status, Status::Failed { .. }));
    assert!(matches!(
        opened.streams[0].decoder,
        Some(Decoder::Unavailable)
    ));
}
