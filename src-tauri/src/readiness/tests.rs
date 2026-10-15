//! a command refuses with the words the snapshot shows for its control

#[cfg(feature = "developer-tools")]
use crate::commands::experiment::*;
use crate::{
    app::state::*,
    commands::{boxes::*, targets::*},
    recording,
};
use motion_bench::sync::LockExt;

use crate::capture::observers;
use crate::targets::vision;

use crate::recording::Recording;
use motion_bench::{
    analysis::views::plot::Views,
    capture::{
        pcap,
        session::{self, Capture},
    },
    coordination::readiness::{self, Readiness},
};
use std::{path::Path, sync::atomic::Ordering};

fn state(folder: &Path) -> Shared {
    let (controller, _) = recording::hotkey::Controller::for_test(folder);
    let mut bench = Bench::new(controller.clone());
    bench.library_loaded = true;
    Shared::new(controller, bench)
}

fn shown(state: &Shared) -> Readiness {
    state.bench.locked().snapshot(0, Views::Readouts).readiness
}

#[test]
fn starting_target_capture_without_a_model_says_what_the_snapshot_says() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    let blocker = shown(&state).screen_capture.unwrap();
    assert_eq!(blocker.need, readiness::Need::ModelMissing);
    assert_eq!(start_screen_capture(&state).unwrap_err(), blocker.message);
}

/// the gate's own refusal and the snapshot taken while it is held use the same words
#[test]
fn a_held_gate_refuses_with_the_busy_message_the_snapshot_shows() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    let held = Recording::claim(&state).unwrap();
    let shown = shown(&state);
    for blocker in [&shown.record, &shown.screen_capture, &shown.box_settings] {
        assert_eq!(blocker.as_ref().unwrap().need, readiness::Need::Busy);
    }
    let message = shown.record.unwrap().message;
    assert_eq!(stop_screen_capture(&state).unwrap_err(), message);
    let toggled = recording::toggle(
        &state,
        motion_bench::capture::clock::Anchor::take(),
        Some(true),
    );
    assert_eq!(toggled.unwrap_err(), message);
    drop(held);
    assert!(stop_screen_capture(&state).is_ok());
}

#[test]
fn a_closing_app_refuses_a_recording_as_the_snapshot_says() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    state.shutdown.store(true, Ordering::Relaxed);
    let blocker = shown(&state).record.unwrap();
    assert_eq!(blocker.need, readiness::Need::Closing);
    let toggled = recording::toggle(
        &state,
        motion_bench::capture::clock::Anchor::take(),
        Some(true),
    );
    assert_eq!(toggled.unwrap_err(), blocker.message);
}

#[cfg(feature = "developer-tools")]
#[test]
fn arming_without_medius_says_what_the_snapshot_says() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    let blocker = shown(&state).arm.unwrap();
    assert_eq!(blocker.need, readiness::Need::NoBox);
    assert_eq!(arm_experiment(&state).unwrap_err(), blocker.message);
}

/// a capture of a generated mouse, selected, its reports checked or not, maybe recording
pub(crate) fn monitoring(folder: &Path, checked: bool, recording: bool) -> Capture {
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
        // picked near the end, the mouse sends too few reports after it to be checked
        let pick = if checked { 1 } else { 45 };
        if data.selected.is_none() && data.health.reports == pick {
            data.select(
                "1:2:129",
                Some(motion_bench::testkit::traffic::mouse_decoder()),
            )
            .unwrap();
            if checked {
                data.trust_selection();
            }
            if recording {
                data.begin("test", folder, None).unwrap();
            }
        }
    })
    .unwrap();
    Capture::replayed(data, "USBPcap9")
}

fn with(folder: &Path, change: impl FnOnce(&mut Bench)) -> Shared {
    let state = state(folder);
    change(&mut state.bench.locked());
    state
}

fn start_recording(state: &Shared) -> Result<String, String> {
    recording::toggle(
        state,
        motion_bench::capture::clock::Anchor::take(),
        Some(true),
    )
}

fn firmware(action: &str) -> BoxCommand {
    BoxCommand {
        action: action.into(),
        mode: None,
        full: None,
        hz: None,
        force: None,
        percent: None,
        milliseconds: None,
        enabled: Some(true),
        key: None,
    }
}

/// a recording keeps the box, target capture and the recording button as they are
#[test]
fn a_recording_refuses_each_change_as_the_snapshot_says() {
    let folder = tempfile::tempdir().unwrap();
    let state = with(folder.path(), |bench| {
        bench.capture = Some(monitoring(folder.path(), true, true));
    });
    let shown = shown(&state);
    let refusals = [
        (&shown.box_settings, send_box(firmware("watch"), &state)),
        (&shown.box_connect, send_box(firmware("connect"), &state)),
        (&shown.screen_capture, start_screen_capture(&state)),
        (&shown.screen_capture, stop_screen_capture(&state)),
        (
            &shown.recording_button,
            recording::hotkey::learn_button(&state, false).map(|_| ()),
        ),
        (
            &shown.clear_recording_button,
            recording::hotkey::clear_button(&state),
        ),
        #[cfg(feature = "developer-tools")]
        (&shown.arm, arm_experiment(&state)),
    ];
    for (blocker, refused) in refusals {
        let blocker = blocker.as_ref().unwrap();
        assert_eq!(blocker.need, readiness::Need::Recording);
        assert_eq!(refused.unwrap_err(), blocker.message);
    }
    assert!(shown.record.is_none(), "Finish stays possible");
}

type Change<'a> = Box<dyn FnOnce(&mut Bench) + 'a>;

/// what each part of the bench makes Record wait for, in the snapshot and in the refusal
#[test]
fn record_names_the_part_in_its_way() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path();
    let cases: [(readiness::Need, Change); 5] = [
        (
            readiness::Need::MouseUnchecked,
            Box::new(|bench| bench.capture = Some(monitoring(path, false, false))),
        ),
        (
            readiness::Need::CaptureFailed,
            Box::new(|bench| {
                let capture = monitoring(path, true, false);
                capture.data.locked().health.ended = true;
                bench.capture = Some(capture);
            }),
        ),
        (
            readiness::Need::ObserverStarting,
            Box::new(|bench| {
                bench.capture = Some(monitoring(path, true, false));
                bench.observers =
                    Some(observers::Worker::with_status(observers::Status::default()));
            }),
        ),
        (
            readiness::Need::ScenarioUnnamed,
            Box::new(|bench| {
                bench.capture = Some(monitoring(path, true, false));
                let mut status = vision::Status::default();
                status.running = true;
                status.ready = true;
                bench.vision = Some(vision::Worker::with_status(
                    motion_bench::targets::vision::Config::default(),
                    status,
                ));
            }),
        ),
        (readiness::Need::CaptureStopped, Box::new(|_| {})),
    ];
    for (need, change) in cases {
        let state = with(path, change);
        let blocker = shown(&state).record.unwrap();
        assert_eq!(blocker.need, need);
        assert_eq!(start_recording(&state).unwrap_err(), blocker.message);
    }
    // the same bench with nothing in the way records
    let state = with(path, |bench| {
        bench.capture = Some(monitoring(path, true, false));
    });
    assert_eq!(shown(&state).record, None);
}
