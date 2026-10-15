use super::*;
use serde_json::json;
/// a cue is a plain wav of its tones, louder with the volume and silent in its pauses
#[test]
fn a_cue_is_a_wav_as_loud_as_asked() {
    let peak = |wave: &[u8]| {
        wave[44..]
            .chunks(2)
            .map(|s| i16::from_le_bytes([s[0], s[1]]).unsigned_abs())
            .max()
            .unwrap()
    };
    let failed = tone_wave(&[250, 0, 250], 1.0);
    assert_eq!(&failed[..4], b"RIFF");
    assert_eq!(&failed[8..16], b"WAVEfmt ");
    let samples = (failed.len() - 44) / 2;
    assert_eq!(samples, 3 * 4410);
    let pause = &failed[44 + 4410 * 2..44 + 4410 * 4];
    assert!(pause.iter().all(|&b| b == 0));
    assert!(peak(&tone_wave(&[1000], 1.0)) > peak(&tone_wave(&[1000], 0.5)) * 3);
    assert_eq!(peak(&tone_wave(&[1000], 0.0)), 0);
}
#[test]
fn control_boundaries_preserve_usb_evidence_and_survive_failed_shutdown() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("trial.pcap");
    let raw = b"unchanged USB timestamps and payloads";
    std::fs::write(&path, raw).unwrap();
    std::fs::write(
        path.with_extension("json"),
        b"{\"complete\":true,\"clock\":{\"system_us\":123}}",
    )
    .unwrap();
    let anchor = motion_bench::capture::clock::Anchor::take();
    let control = json!({ "schema":"motion-bench.recording-control.v1", "path":path.display().to_string(), "start_requested":anchor, "stop_requested":anchor });
    let sidecar = std::fs::read(path.with_extension("json")).unwrap();
    let read = || {
        serde_json::from_slice::<Value>(
            &std::fs::read(path.with_extension("recording-control.json")).unwrap(),
        )
        .unwrap()
    };
    let mut finished = control.clone();
    finish_control(&mut finished, None).unwrap();
    assert_eq!(read(), finished);
    assert_eq!(read()["complete"], true);
    assert_eq!(read()["stop_requested"]["qpc_before"], anchor.qpc_before);
    let mut shutdown = control;
    finish_control(
        &mut shutdown,
        Some("recording ended on application shutdown"),
    )
    .unwrap();
    assert_eq!(read()["complete"], false);
    assert_eq!(read()["error"], "recording ended on application shutdown");
    // the journal is its own file, the recording's metadata and capture are left alone
    assert_eq!(std::fs::read(path.with_extension("json")).unwrap(), sidecar);
    assert_eq!(std::fs::read(&path).unwrap(), raw);
}

/// a side button starts a recording with the app in front too, a left click there doesn't
#[test]
fn the_button_works_in_the_app_unless_it_clicks_the_app() {
    let folder = tempfile::tempdir().unwrap();
    let (controller, requests) = Controller::for_test(folder.path());
    controller.ready();
    let anchor = Anchor::take();
    let press = |mask| {
        controller.buttons(0, false, 8, false, anchor.clone());
        controller.buttons(mask, false, 8, false, anchor.clone());
    };
    controller.learn(false);
    press(16);
    press(16);
    assert!(requests.try_recv().is_ok());
    controller.learn(false);
    press(1);
    assert_eq!(controller.snapshot().hotkey_mask, Some(1));
    press(1);
    assert!(requests.try_recv().is_err());
}

#[test]
fn the_binding_persists_as_one_mouse_button() {
    let folder = tempfile::tempdir().unwrap();
    for mask in [0, 3, 24] {
        assert!(Button::try_from(mask).is_err());
    }
    let (controller, _) = Controller::for_test(folder.path());
    let saved = || {
        crate::config::Store::open(folder.path().join("config.json"))
            .get()
            .recording
            .button
            .map(Button::mask)
    };
    controller.ready();
    controller.learn(false);
    controller.buttons(16, false, 8, false, Anchor::take());
    assert_eq!(saved(), Some(16));
    controller.disable().unwrap();
    assert_eq!(saved(), None);
}
#[test]
fn learning_never_toggles_and_edges_are_bounded_while_recording_is_busy() {
    let folder = tempfile::tempdir().unwrap();
    let (controller, requests) = Controller::for_test(folder.path());
    controller.ready();
    controller.learn(false);
    let anchor = Anchor::take();
    // let go of every button, then press `mask`
    let click = |mask| {
        controller.buttons(0, false, 8, true, anchor.clone());
        controller.buttons(mask, false, 8, true, anchor.clone());
    };
    controller.buttons(24, false, 8, true, anchor.clone());
    assert!(controller.snapshot().learning);
    controller.buttons(0, false, 8, true, anchor.clone());
    controller.buttons(8, false, 8, true, anchor.clone());
    assert!(controller.snapshot().error.unwrap().contains("aim hold"));
    assert!(controller.snapshot().learning);
    click(16);
    assert_eq!(controller.snapshot().hotkey_mask, Some(16));
    assert!(requests.try_recv().is_err());
    controller.buttons(16, false, 8, true, anchor.clone());
    assert!(requests.try_recv().is_err());
    click(16);
    assert_eq!(requests.try_recv().unwrap().qpc_before, anchor.qpc_before);
    // a press on its way is busy, a second one is refused until the worker takes the first
    assert!(controller.snapshot().busy);
    click(16);
    assert!(requests.try_recv().is_err());
    assert!(
        controller
            .snapshot()
            .error
            .unwrap()
            .contains("still finalizing")
    );
    controller.recording().unpress();
    controller.buttons(16, false, 8, true, anchor.clone());
    assert!(requests.try_recv().is_err());
    click(16);
    assert!(requests.try_recv().is_ok());
}
#[test]
fn loss_disconnect_and_aim_learning_never_trigger_a_recording_request() {
    let folder = tempfile::tempdir().unwrap();
    let (controller, requests) = Controller::for_test(folder.path());
    controller.update(|s| s.hotkey_mask = Some(16));
    controller.ready();
    controller.buttons(16, true, 8, true, Anchor::take());
    controller.buttons(16, false, 8, true, Anchor::take());
    assert!(requests.try_recv().is_err());
    controller.unavailable(Some("CATCH lost events".into()));
    controller.buttons(0, false, 8, true, Anchor::take());
    controller.buttons(16, false, 8, true, Anchor::take());
    assert!(requests.try_recv().is_err());
    assert!(!controller.snapshot().ready);
    assert!(controller.snapshot().error.unwrap().contains("lost events"));
}
#[test]
fn clicks_in_the_app_learn_but_do_not_record_or_retrigger_when_focus_changes() {
    let folder = tempfile::tempdir().unwrap();
    let (controller, requests) = Controller::for_test(folder.path());
    controller.ready();
    controller.learn(false);
    controller.buttons(1, false, 8, false, Anchor::take());
    assert_eq!(controller.snapshot().hotkey_mask, Some(1));
    controller.buttons(0, false, 8, false, Anchor::take());
    controller.buttons(1, false, 8, false, Anchor::take());
    controller.buttons(1, false, 8, true, Anchor::take());
    assert!(requests.try_recv().is_err());
    controller.buttons(0, false, 8, true, Anchor::take());
    controller.buttons(1, false, 8, true, Anchor::take());
    assert!(requests.try_recv().is_ok());
}
/// a button learned when the settings cannot be written is used until the app closes, the
/// settings say they were not saved, and nothing is recorded
#[test]
fn a_binding_that_cannot_be_saved_is_kept_for_the_session() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("blocked");
    std::fs::write(&path, b"file").unwrap();
    let (controller, requests) = Controller::for_test(&path);
    controller.ready();
    controller.learn(false);
    controller.buttons(16, false, 8, true, Anchor::take());
    let status = controller.snapshot();
    assert_eq!((status.hotkey_mask, status.learning), (Some(16), false));
    assert!(status.error.is_none());
    assert!(controller.settings().unsaved().is_some());
    assert_eq!(
        controller
            .settings()
            .get()
            .recording
            .button
            .map(Button::mask),
        Some(16)
    );
    assert!(requests.try_recv().is_err());
}
/// restoring a saved binding records nothing until the button is pressed
#[test]
fn a_restored_binding_records_nothing_by_itself() {
    let folder = tempfile::tempdir().unwrap();
    let (first, _) = Controller::for_test(folder.path());
    first.ready();
    first.learn(false);
    first.buttons(16, false, 8, true, Anchor::take());
    let (controller, requests) = Controller::for_test(folder.path());
    assert_eq!(controller.snapshot().hotkey_mask, Some(16));
    controller.ready();
    assert!(requests.try_recv().is_err());
    assert!(!controller.snapshot().busy);
}
