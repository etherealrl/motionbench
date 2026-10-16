//! every way a recording starts and ends, driven through the coordinator over a replayed
//! capture in a temporary folder

use super::*;
use crate::readiness_tests::monitoring;
use motion_bench::analysis::views::plot::Views;
use motion_bench::capture::pcap;
use motion_bench::sync::LockExt;
use std::time::Duration;

/// monitoring a checked mouse with the observers, recordings kept in `folder`
fn state(folder: &Path) -> Shared {
    let (controller, _) = hotkey::Controller::for_test(folder);
    let mut bench = Bench::new(controller.clone());
    bench.library_loaded = true;
    bench.capture = Some(monitoring(folder, true, false));
    bench.observers = Some(observers::Worker::with_status(observers::Status {
        raw_ready: true,
        ..Default::default()
    }));
    Shared::new(controller, bench).in_folder(folder)
}

/// more of the mouse's reports, after the ones `monitoring` replayed
fn reports(state: &Shared, from_us: u64) {
    use motion_bench::testkit::traffic::{Packet, Traffic, mouse_payload, ticks};
    let data = state.bench.locked().capture.as_ref().unwrap().data.clone();
    let mut traffic = Traffic::default();
    traffic.extend(
        ticks(1000, from_us, 100)
            .map(|t| (t, Packet::report(1, 2, 0x81, &mouse_payload(1, -1, 7)))),
    );
    pcap::read_pcap(traffic.pcap().bytes(), |record| {
        data.locked().ingest(&record)
    })
    .unwrap();
}

/// until a drive on another thread is done and no recording is left
fn settled(state: &Shared) {
    let waited = std::time::Instant::now();
    while state.recording.busy() || state.recording.recording() {
        assert!(waited.elapsed() < Duration::from_secs(10), "never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn start(state: &Shared) -> String {
    toggle(state, Anchor::take(), Some(true)).unwrap()
}

fn json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// the metadata, the journal and the observers' file of a saved recording
fn saved(folder: &Path, stem: &str) -> (Value, Value, Value) {
    let pcap = folder.join(format!("{stem}.pcap"));
    (
        json(&pcap.with_extension("json")),
        json(&pcap.with_extension("recording-control.json")),
        json(&pcap.with_extension("observers.json")),
    )
}

#[test]
fn the_record_button_starts_and_stops_a_recording_and_lists_it() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    let stem = start(&state);
    assert!(state.recording.recording() && !state.recording.busy());
    let journal = json(&folder.path().join(format!("{stem}.recording-control.json")));
    assert!(journal["start_requested"].is_object() && journal["stop_requested"].is_null());
    reports(&state, 2_000_000);

    let path = toggle(&state, Anchor::take(), Some(false)).unwrap();
    assert_eq!(
        path,
        folder
            .path()
            .join(format!("{stem}.pcap"))
            .display()
            .to_string()
    );
    assert!(!state.recording.busy() && !state.recording.recording());
    let (metadata, journal, observers) = saved(folder.path(), &stem);
    assert_eq!(metadata["stop_reason"], "record_stop");
    assert_eq!(metadata["outcome"], "complete");
    assert_eq!(journal["complete"], true);
    assert!(journal["stop_requested"].is_object() && journal["stop_cause"].is_null());
    assert_eq!(observers["complete"], true);
    let bench = state.bench.locked();
    assert_eq!(bench.library.runs().len(), 1);
    assert!(folder.path().join("library.json").is_file());
    // capture goes on, with the observers started again
    assert!(bench.capture.is_some() && bench.observers.is_some());
    assert!(
        bench.last_health.reports > 0,
        "the health it ended with is kept"
    );
    let status = bench.snapshot(0, Views::Readouts).recording_hotkey;
    assert_eq!(
        status.message.unwrap(),
        "saved to Runs, USB monitoring keeps running"
    );
}

#[test]
fn the_recording_button_toggles_and_its_stop_says_so() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    assert!(state.recording.press());
    // a second press waits for the first
    assert!(!state.recording.press());
    assert!(state.recording.busy());
    let stem = toggle(&state, Anchor::take(), None).unwrap();
    assert!(!state.recording.busy());
    reports(&state, 2_000_000);
    assert!(state.recording.press());
    toggle(&state, Anchor::take(), None).unwrap();
    let (metadata, ..) = saved(folder.path(), &stem);
    assert_eq!(metadata["stop_reason"], "hotkey_stop");
}

#[test]
fn a_press_waiting_for_the_worker_keeps_the_record_button_and_claims_out() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    assert!(state.recording.press());
    assert_eq!(
        toggle(&state, Anchor::take(), Some(true)).unwrap_err(),
        Refusal::Busy.message()
    );
    assert!(Recording::claim(&state).is_err());
    // a press refused before it reached the coordinator gives itself back
    state.bench.locked().library_loaded = false;
    assert!(toggle(&state, Anchor::take(), None).is_err());
    assert!(!state.recording.busy());
}

#[test]
fn stopping_capture_ends_the_recording_with_it_and_lists_it() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    let stem = start(&state);
    reports(&state, 2_000_000);
    let path = stop_capture(&state).unwrap().unwrap();
    assert!(path.ends_with(&format!("{stem}.pcap")));
    let (metadata, journal, _) = saved(folder.path(), &stem);
    assert_eq!(metadata["stop_reason"], "capture_stop");
    assert_eq!(journal["complete"], true);
    let bench = state.bench.locked();
    assert!(bench.capture.is_none() && bench.observers.is_none());
    assert_eq!(bench.library.runs().len(), 1);
    assert_eq!(
        bench.recording_hotkey.snapshot().message.unwrap(),
        "saved to Runs"
    );
}

#[test]
fn stopping_capture_with_no_recording_stops_only_capture() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    assert_eq!(stop_capture(&state), Ok(None));
    assert!(state.bench.locked().capture.is_none());
    assert!(!state.recording.busy());
    assert_eq!(stop_capture(&state).unwrap_err(), "capture is not running");
}

#[test]
fn a_recording_seen_failing_is_ended_at_once_and_listed() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    let stem = start(&state);
    reports(&state, 2_000_000);
    let data = state.bench.locked().capture.as_ref().unwrap().data.clone();
    data.locked().fail(Reason::WriterFailed, "the disk is full");
    watch(&state, &data.locked());
    // the pump looks again every 2 ms, a stop on its way is not started twice
    watch(&state, &data.locked());
    settled(&state);
    // the stop is driven to its end before it's announced, on the same thread
    let waited = std::time::Instant::now();
    let error = loop {
        if let Some(error) = state.recording_hotkey.snapshot().error {
            break error;
        }
        assert!(
            waited.elapsed() < Duration::from_secs(10),
            "never announced"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(
        error.starts_with("the recording failed and was stopped"),
        "{error}"
    );
    let (metadata, journal, _) = saved(folder.path(), &stem);
    assert_eq!(metadata["stop_reason"], "recording_failed");
    assert_eq!(metadata["outcome"], "failed");
    assert!(journal["stop_requested"].is_object());
    assert_eq!(state.library().runs().len(), 1);
    assert!(!state.recording.busy());
}

#[test]
fn closing_the_app_commits_the_recording_and_lists_nothing() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    let stem = start(&state);
    reports(&state, 2_000_000);
    close(&state, Duration::from_secs(10));
    assert!(state.recording.closed());
    let (metadata, journal, observers) = saved(folder.path(), &stem);
    assert_eq!(metadata["stop_reason"], "app_exit");
    assert_eq!(
        journal["stop_cause"],
        "application closed without an explicit finish"
    );
    assert_eq!(observers["complete"], true);
    let bench = state.bench.locked();
    assert!(bench.library.runs().is_empty() && bench.capture.is_none());
    assert!(!folder.path().join("library.json").exists());
}

/// the same recording ended four ways keeps the same participants; only how it stopped differs
#[test]
fn every_way_a_recording_ends_drains_it_alike() {
    let ended: [fn(&Shared); 4] = [
        |state| drop(toggle(state, Anchor::take(), Some(false))),
        |state| drop(stop_capture(state)),
        |state| close(state, Duration::from_secs(1)),
        |state| {
            let data = state.bench.locked().capture.as_ref().unwrap().data.clone();
            // a failure of the recording itself
            data.locked()
                .fail(Reason::ClockFailed, "the clock went back");
            watch(state, &data.locked());
            settled(state);
        },
    ];
    let mut participants = Vec::new();
    for end in ended {
        let folder = tempfile::tempdir().unwrap();
        let state = state(folder.path());
        let stem = start(&state);
        reports(&state, 2_000_000);
        end(&state);
        let (metadata, ..) = saved(folder.path(), &stem);
        let mut ids: Vec<String> = metadata["participants"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                format!(
                    "{}: {}",
                    p["id"].as_str().unwrap(),
                    if p["error"].is_null() { "ok" } else { "failed" }
                )
            })
            .collect();
        ids.sort();
        participants.push(ids);
    }
    assert!(
        participants.iter().all(|ids| *ids == participants[0]),
        "{participants:?}"
    );
    assert_eq!(participants[0], ["observers: ok", "recording_control: ok"]);
}

#[test]
fn a_claim_keeps_starts_and_stops_out_and_a_failure_meanwhile_ends_the_recording_after_it() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    let stem = start(&state);
    reports(&state, 2_000_000);
    let claim = Recording::claim(&state).unwrap();
    assert_eq!(
        toggle(&state, Anchor::take(), Some(false)).unwrap_err(),
        Refusal::Busy.message()
    );
    assert_eq!(stop_capture(&state).unwrap_err(), Refusal::Busy.message());
    assert!(!state.recording.press());
    let data = state.bench.locked().capture.as_ref().unwrap().data.clone();
    data.locked().fail(Reason::WriterFailed, "the disk is full");
    watch(&state, &data.locked());
    std::thread::sleep(Duration::from_millis(50));
    assert!(state.recording.recording(), "it waits for the release");
    drop(claim);
    settled(&state);
    let (metadata, ..) = saved(folder.path(), &stem);
    assert_eq!(metadata["stop_reason"], "recording_failed");
}

#[test]
fn the_box_input_breaks_the_pairing_only_of_a_recording_it_took_part_in() {
    let folder = tempfile::tempdir().unwrap();
    let mut data = Data::new(true);
    let firmware = medius::Snapshot {
        link_drops: 2,
        ..Default::default()
    };
    // nothing recorded, nothing to break
    box_input(&mut data, [], &firmware, (0, 0), "lost");
    assert!(data.health.pairing_valid);
    let mut capture = monitoring(folder.path(), true, true);
    let data = Arc::get_mut(&mut capture.data).unwrap().get_mut().unwrap();
    box_input(data, [], &firmware, (2, 0), "lost");
    data.take_part(MEDIUS_INPUT);
    box_input(data, [], &firmware, (2, 0), "lost");
    assert!(
        data.health.pairing_valid,
        "nothing lost since it was last seen"
    );
    box_input(data, [], &firmware, (1, 0), "lost");
    assert!(!data.health.pairing_valid);
    assert_eq!(data.health.serial_dropped, 1);
}

/// a recording that ends before the box's input caught up can't vouch for its pairing
#[test]
fn input_still_behind_at_the_end_breaks_the_pairing() {
    let folder = tempfile::tempdir().unwrap();
    let link = medius::Link::spawn();
    for (caught_up, valid) in [(true, true), (false, false)] {
        let mut capture = monitoring(folder.path(), true, true);
        let data = Arc::get_mut(&mut capture.data).unwrap().get_mut().unwrap();
        data.take_part(MEDIUS_INPUT);
        last_box_input(&link, data, (0, 0), caught_up);
        assert_eq!(data.health.pairing_valid, valid);
    }
}

#[test]
fn target_capture_takes_part_and_its_events_are_built_in_the_hand_off() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    with_vision(&state);
    let stem = start(&state);
    assert!(
        state
            .bench
            .locked()
            .vision
            .as_ref()
            .unwrap()
            .snapshot()
            .recording
    );
    reports(&state, 2_000_000);
    toggle(&state, Anchor::take(), Some(false)).unwrap();
    let vision = json(&folder.path().join(format!("{stem}.vision.json")));
    assert_eq!(vision["complete"], true);
    let (metadata, ..) = saved(folder.path(), &stem);
    assert!(
        metadata["participants"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "vision")
    );
    assert!(folder.path().join(format!("{stem}.events.json")).is_file());
    let message = state.recording_hotkey.snapshot().message.unwrap();
    assert!(
        message.starts_with("saved to Runs, no target events"),
        "{message}"
    );
}

#[test]
fn a_target_model_still_warming_up_never_stops_the_usb_recording() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    with_vision(&state);
    {
        let bench = state.bench.locked();
        let vision = bench.vision.as_ref().unwrap();
        vision.status.locked().ready = false;
    }
    let stem = start(&state);
    reports(&state, 2_000_000);
    toggle(&state, Anchor::take(), Some(false)).unwrap();
    let (metadata, ..) = saved(folder.path(), &stem);
    assert_eq!(metadata["outcome"], "degraded", "{metadata}");
    let vision = metadata["participants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "vision")
        .unwrap()
        .clone();
    assert_eq!(
        vision["error"],
        "screen recording could not start: the target model wasn't ready yet"
    );
    assert!(!folder.path().join(format!("{stem}.events.json")).exists());
    let located =
        motion_bench::evidence::reader::layout::locate(&folder.path().join(format!("{stem}.pcap")))
            .unwrap();
    let (metadata, status) = motion_bench::evidence::reader::status::assess(&located);
    assert!(
        motion_bench::evidence::reader::eligibility::eligibility(
            &metadata,
            &status,
            &located.streams(),
            motion_bench::evidence::reader::eligibility::TARGETS
        )
        .is_err()
    );
}

#[cfg(feature = "developer-tools")]
#[test]
fn an_armed_aim_takes_part_and_stays_armed_after_the_recording() {
    let (_mock, _link, worker, source, _focus) = crate::controller::experiment::tests::setup();
    worker
        .arm(motion_bench::controller::linear::Config::default(), source)
        .unwrap();
    let worker = Arc::new(worker);
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    state.bench.locked().experiment = Some(worker.clone());
    let stem = start(&state);
    assert!(worker.snapshot().recording);
    reports(&state, 2_000_000);
    toggle(&state, Anchor::take(), Some(false)).unwrap();
    // the journal is closed, disarming is left to the user
    let status = worker.snapshot();
    assert!(status.armed && !status.recording);
    let (metadata, ..) = saved(folder.path(), &stem);
    assert!(
        metadata["participants"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "injection")
    );
    assert!(
        folder
            .path()
            .join(format!("{stem}.injection.json"))
            .is_file()
    );
}

#[test]
fn closing_drops_a_waiting_press_and_still_commits_the_recording() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    let stem = start(&state);
    reports(&state, 2_000_000);
    assert!(state.recording.press());
    close(&state, Duration::from_secs(10));
    assert!(state.recording.closed());
    let (metadata, ..) = saved(folder.path(), &stem);
    assert_eq!(metadata["stop_reason"], "app_exit");
    // the worker taking the press afterwards is refused, nothing starts
    assert_eq!(
        toggle(&state, Anchor::take(), None).unwrap_err(),
        Refusal::Closing.message()
    );
}

#[test]
fn a_capture_stop_refused_during_a_stop_leaves_capture_running() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    start(&state);
    reports(&state, 2_000_000);
    // a stop on its way, not driven yet
    let effects = state
        .recording
        .request(Event::Stop(StopReason::RecordStop), Anchor::take(), false)
        .unwrap();
    assert_eq!(stop_capture(&state).unwrap_err(), Refusal::Busy.message());
    announce(&state, drive(&state, effects)).unwrap();
    assert!(state.bench.locked().capture.is_some());
    assert!(!state.recording.busy());
}

#[test]
fn a_failure_seen_during_the_start_keeps_when_it_was_seen() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    let effects = {
        let bench = state.bench.locked();
        state
            .recording
            .request(Event::Start(parts(&bench)), Anchor::take(), false)
            .unwrap()
    };
    let attempt = state.recording.attempt();
    let failure = Failure::new(Reason::WriterFailed, "the disk is full");
    assert!(
        state
            .recording
            .report(Event::Failed(attempt, failure))
            .is_empty()
    );
    let driven = drive(&state, effects);
    let stem = driven.stem.clone().unwrap();
    assert!(announce(&state, driven).is_err());
    let (metadata, journal, _) = saved(folder.path(), &stem);
    assert_eq!(metadata["stop_reason"], "recording_failed");
    let stopped = journal["stop_requested"]["qpc_before"].as_i64().unwrap();
    assert!(stopped < journal["start_completed"]["qpc_before"].as_i64().unwrap());
}

#[test]
fn a_late_start_announcement_never_overwrites_the_end_of_its_recording() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    let effects = {
        let bench = state.bench.locked();
        state
            .recording
            .request(Event::Start(parts(&bench)), Anchor::take(), false)
            .unwrap()
    };
    // started, but its words are held back while the recording ends
    let started = drive(&state, effects);
    reports(&state, 2_000_000);
    toggle(&state, Anchor::take(), Some(false)).unwrap();
    let stem = started.stem.clone().unwrap();
    assert_eq!(announce(&state, started), Ok(Some(stem)));
    let message = state.recording_hotkey.snapshot().message.unwrap();
    assert!(message.starts_with("saved to Runs"), "{message}");
}

/// target capture running and ready, its model loaded
fn with_vision(state: &Shared) {
    let config = motion_bench::targets::vision::Config {
        scenario: "gridshot".into(),
        ..Default::default()
    };
    let mut status = crate::targets::vision::Status::default();
    status.running = true;
    status.ready = true;
    status.event_ready = true;
    status.model_sha256 = Some("a model's digest".into());
    status.clock = Some(motion_bench::targets::vision::Clock {
        initial_bracket_us: 0.0,
        qpc_origin: 0,
        qpc_frequency: 10_000_000,
        epoch_origin_us: 0,
    });
    state.bench.locked().vision = Some(crate::targets::vision::Worker::with_status(config, status));
}

#[test]
fn a_recording_that_failed_gets_no_target_events() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    with_vision(&state);
    let stem = start(&state);
    reports(&state, 2_000_000);
    let data = state.bench.locked().capture.as_ref().unwrap().data.clone();
    data.locked().fail(Reason::WriterFailed, "the disk is full");
    assert!(toggle(&state, Anchor::take(), Some(false)).is_err());
    assert!(!folder.path().join(format!("{stem}.events.json")).exists());
    // listed all the same, as failed
    assert_eq!(state.library().runs().len(), 1);
}

#[test]
fn a_start_that_cannot_make_its_files_says_so() {
    let folder = tempfile::tempdir().unwrap();
    let mut state = state(folder.path());
    // a file where the recordings folder should be
    let blocked = folder.path().join("blocked");
    std::fs::write(&blocked, b"").unwrap();
    state.folder = blocked;
    let error = toggle(&state, Anchor::take(), Some(true)).unwrap_err();
    let status = state.recording_hotkey.snapshot();
    assert_eq!(status.message.unwrap(), "Recording request failed");
    assert_eq!(status.error.unwrap(), error);
    assert!(!state.recording.busy() && !state.recording.recording());
}

#[test]
fn a_toggle_asking_for_the_state_it_is_in_is_refused() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    assert_eq!(
        toggle(&state, Anchor::take(), Some(false)).unwrap_err(),
        Refusal::NotRecording.message()
    );
    start(&state);
    assert_eq!(
        toggle(&state, Anchor::take(), Some(true)).unwrap_err(),
        Refusal::AlreadyRecording.message()
    );
}

#[test]
fn capture_that_failed_on_its_own_is_stopped() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    let data = state.bench.locked().capture.as_ref().unwrap().data.clone();
    {
        let mut data = data.locked();
        data.fail(Reason::TransportEnded, "USBPcap stream ended unexpectedly");
        data.health.ended = true;
    }
    watch(&state, &data.locked());
    let waited = std::time::Instant::now();
    while state.bench.locked().capture.is_some() || state.recording.busy() {
        assert!(waited.elapsed() < Duration::from_secs(10), "never stopped");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(state.bench.locked().last_health.ended);
}

#[test]
fn the_buttons_cannot_be_learned_or_cleared_while_a_claim_is_held() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    let claim = Recording::claim(&state).unwrap();
    assert!(hotkey::learn_button(&state, false).is_err());
    assert!(hotkey::clear_button(&state).is_err());
    drop(claim);
    assert!(hotkey::clear_button(&state).is_ok());
}

#[cfg(feature = "developer-tools")]
#[test]
fn an_aim_that_is_not_armed_takes_no_part_and_a_capture_stop_disarms_one_that_is() {
    let (_mock, _link, worker, source, _focus) = crate::controller::experiment::tests::setup();
    let worker = Arc::new(worker);
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    state.bench.locked().experiment = Some(worker.clone());
    let stem = start(&state);
    reports(&state, 2_000_000);
    toggle(&state, Anchor::take(), Some(false)).unwrap();
    let (metadata, ..) = saved(folder.path(), &stem);
    assert!(
        !metadata["participants"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "injection")
    );
    assert!(
        !folder
            .path()
            .join(format!("{stem}.injection.json"))
            .exists()
    );
    // armed with no recording, stopping capture disarms it
    worker
        .arm(motion_bench::controller::linear::Config::default(), source)
        .unwrap();
    stop_capture(&state).unwrap();
    assert!(!worker.snapshot().armed);
}

/// a lock poisoned on purpose, the way a panic while holding it would
fn poison<T: Send + 'static>(lock: Arc<Mutex<T>>) {
    let _ = std::thread::spawn(move || {
        let _held = lock.locked();
        panic!("poisoned on purpose");
    })
    .join();
}

#[test]
fn a_broken_app_refuses_everything_but_still_closes_and_commits() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    let stem = start(&state);
    reports(&state, 2_000_000);
    poison(state.bench.clone());
    let broken = motion_bench::coordination::readiness::BROKEN;
    assert_eq!(
        toggle(&state, Anchor::take(), Some(false)).unwrap_err(),
        broken
    );
    assert_eq!(stop_capture(&state).unwrap_err(), broken);
    assert_eq!(Recording::claim(&state).err().unwrap(), broken);
    // the app's state may be half written, nothing of it is listed
    let pcap = folder.path().join(format!("{stem}.pcap"));
    assert_eq!(add_to_runs(&state, &pcap).unwrap_err(), broken);
    let shown = state.bench.locked().snapshot(0, Views::Readouts).readiness;
    assert_eq!(
        shown.record.unwrap().need,
        motion_bench::coordination::readiness::Need::Broken
    );
    close(&state, Duration::from_secs(5));
    assert!(state.recording.closed());
    let (metadata, ..) = saved(folder.path(), &stem);
    assert_eq!(metadata["stop_reason"], "app_exit");
}

#[test]
fn a_recording_whose_state_a_panic_left_half_written_is_saved_as_failed() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    let stem = start(&state);
    reports(&state, 2_000_000);
    let data = state.bench.locked().capture.as_ref().unwrap().data.clone();
    poison(data);
    close(&state, Duration::from_secs(5));
    let (metadata, ..) = saved(folder.path(), &stem);
    assert_eq!(metadata["outcome"], "failed");
    assert!(!folder.path().join(format!("{stem}.active.json")).exists());
}

#[cfg(feature = "developer-tools")]
#[test]
fn a_broken_app_disarms_the_aim_and_still_lets_it_be_disarmed() {
    let (_mock, _link, worker, source, _focus) = crate::controller::experiment::tests::setup();
    worker
        .arm(motion_bench::controller::linear::Config::default(), source)
        .unwrap();
    let worker = Arc::new(worker);
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    state.bench.locked().experiment = Some(worker.clone());
    poison(state.bench.clone());
    // the snapshot pump notices it like this, the first look disarms
    assert!(broken(&state));
    let waited = std::time::Instant::now();
    while worker.snapshot().armed {
        assert!(waited.elapsed() < Duration::from_secs(5), "never disarmed");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(crate::commands::experiment::disarm_experiment(&state).is_ok());
}

#[cfg(feature = "developer-tools")]
#[test]
fn a_stuck_aim_controller_is_lost_at_the_stop_deadline_and_fails_the_recording() {
    let (_mock, _link, worker, source, _focus) = crate::controller::experiment::tests::setup();
    worker
        .arm(motion_bench::controller::linear::Config::default(), source)
        .unwrap();
    let worker = Arc::new(worker);
    let folder = tempfile::tempdir().unwrap();
    let mut state = state(folder.path());
    state.stop_within = Duration::from_millis(300);
    state.bench.locked().experiment = Some(worker.clone());
    let stem = start(&state);
    reports(&state, 2_000_000);
    let stuck = worker.hold_box();
    let asked = std::time::Instant::now();
    assert!(toggle(&state, Anchor::take(), Some(false)).is_err());
    assert!(
        asked.elapsed() < Duration::from_secs(3),
        "{:?}",
        asked.elapsed()
    );
    assert!(!state.recording.busy());
    let (metadata, ..) = saved(folder.path(), &stem);
    assert_eq!(metadata["outcome"], "failed");
    let injection = json(&folder.path().join(format!("{stem}.injection.json")));
    assert_eq!(injection["error"], "the aim controller didn't stop in time");
    drop(stuck);
}

/// a panic in each effect of a whole recording in turn, ended the way `how` says: the app is
/// broken, nothing stays busy and every recording has its manifest or its marker, never neither
fn a_panic_in_any_effect_leaves_the_evidence_or_its_marker(
    aim: bool,
    vision: bool,
    how: impl Fn(&Shared),
) {
    // the pump ends a recording that failed, a broken app refuses the stop
    let pump = |state: &Shared| {
        if state.recording.closed() {
            return;
        }
        let data = state
            .bench
            .locked()
            .capture
            .as_ref()
            .map(|capture| capture.data.clone());
        if let Some(data) = data {
            watch(state, &data.locked());
        }
        let waited = std::time::Instant::now();
        while state.recording.busy() {
            // a slow CI runner took more than 10 s once, a real hang never ends at all
            assert!(waited.elapsed() < Duration::from_secs(60), "never ended");
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    for site in 0..100 {
        let folder = tempfile::tempdir().unwrap();
        let state = state(folder.path());
        if vision {
            with_vision(&state);
        }
        #[cfg(feature = "developer-tools")]
        let _armed = aim.then(|| {
            let (mock, link, worker, source, focus) = crate::controller::experiment::tests::setup();
            worker
                .arm(motion_bench::controller::linear::Config::default(), source)
                .unwrap();
            let worker = Arc::new(worker);
            state.bench.locked().experiment = Some(worker.clone());
            (mock, link, worker, focus)
        });
        #[cfg(not(feature = "developer-tools"))]
        let _ = aim;
        panics::SEEN.with(|seen| seen.set(0));
        panics::AT.with(|at| at.set(Some(site)));
        let _ = toggle(&state, Anchor::take(), Some(true));
        pump(&state);
        reports(&state, 2_000_000);
        how(&state);
        pump(&state);
        let panicked = panics::SEEN.with(|seen| seen.get()) > site;
        panics::AT.with(|at| at.set(None));
        assert!(
            !state.recording.busy() || state.recording.closed(),
            "busy after a panic at {site}"
        );
        assert_eq!(state.recording.is_broken(), panicked, "site {site}");
        for file in std::fs::read_dir(folder.path()).unwrap().flatten() {
            let name = file.file_name().to_string_lossy().to_string();
            let Some(stem) = name.strip_suffix(".pcap") else {
                continue;
            };
            let manifest = folder.path().join(format!("{stem}.json")).is_file();
            let marker = folder.path().join(format!("{stem}.active.json")).exists();
            assert!(
                manifest != marker,
                "site {site}: {stem} {manifest} {marker}"
            );
        }
        if !panicked {
            assert!(site >= 3, "only {site} sites");
            return;
        }
    }
    panic!("no end to the sites");
}

#[test]
fn a_panic_in_any_effect_of_a_stop_leaves_the_evidence_or_its_marker() {
    for vision in [false, true] {
        a_panic_in_any_effect_leaves_the_evidence_or_its_marker(false, vision, |state| {
            if state.recording.recording() {
                let _ = toggle(state, Anchor::take(), Some(false));
            }
        });
    }
}

#[test]
fn a_panic_in_any_effect_of_the_exit_leaves_the_evidence_or_its_marker() {
    a_panic_in_any_effect_leaves_the_evidence_or_its_marker(false, true, |state| {
        close(state, Duration::from_secs(3));
    });
}

#[cfg(feature = "developer-tools")]
#[test]
fn a_panic_in_any_effect_with_the_aim_armed_leaves_the_evidence_or_its_marker() {
    a_panic_in_any_effect_leaves_the_evidence_or_its_marker(true, true, |state| {
        if state.recording.recording() {
            let _ = toggle(state, Anchor::take(), Some(false));
        }
    });
}

#[cfg(feature = "developer-tools")]
#[test]
fn closing_with_a_stuck_box_returns_within_its_bound() {
    let (_mock, _link, worker, source, _focus) = crate::controller::experiment::tests::setup();
    worker
        .arm(motion_bench::controller::linear::Config::default(), source)
        .unwrap();
    let worker = Arc::new(worker);
    let folder = tempfile::tempdir().unwrap();
    let mut state = state(folder.path());
    state.stop_within = Duration::from_millis(300);
    state.bench.locked().experiment = Some(worker.clone());
    let stem = start(&state);
    reports(&state, 2_000_000);
    let _stuck = worker.hold_box();
    let asked = std::time::Instant::now();
    close(&state, Duration::from_secs(10));
    assert!(
        asked.elapsed() < Duration::from_secs(11),
        "{:?}",
        asked.elapsed()
    );
    // what is stuck does not take the time the commit needs, the recording is saved as failed
    assert!(state.recording.closed());
    let (metadata, ..) = saved(folder.path(), &stem);
    assert_eq!(metadata["outcome"], "failed");
    assert!(!folder.path().join(format!("{stem}.active.json")).exists());
}

#[test]
fn closing_while_a_failure_drains_on_another_thread_waits_for_it() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    let stem = start(&state);
    reports(&state, 2_000_000);
    let data = state.bench.locked().capture.as_ref().unwrap().data.clone();
    data.locked().fail(Reason::WriterFailed, "the disk is full");
    // the drain runs on a thread of its own, the exit finds it on its way
    watch(&state, &data.locked());
    close(&state, Duration::from_secs(5));
    assert!(state.recording.closed());
    let (metadata, ..) = saved(folder.path(), &stem);
    assert_eq!(metadata["stop_reason"], "recording_failed");
    assert_eq!(metadata["outcome"], "failed");
}

#[test]
fn target_events_a_finished_recording_is_missing_are_built_when_the_library_is_read_again() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    with_vision(&state);
    let stem = start(&state);
    reports(&state, 2_000_000);
    toggle(&state, Anchor::take(), Some(false)).unwrap();
    // as if the app had closed while it was saved, before its hand-off
    let events = folder.path().join(format!("{stem}.events.json"));
    std::fs::remove_file(&events).unwrap();
    let interrupted = start(&state);
    reports(&state, 3_000_000);
    close(&state, Duration::from_secs(10));
    let library = crate::library::Library::new(folder.path().to_path_buf());
    let warnings = library.restore();
    let runs = library.runs();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(runs.len(), 2);
    assert!(events.is_file());
    // one the exit cut short has no events to build
    assert!(
        !folder
            .path()
            .join(format!("{interrupted}.events.json"))
            .exists()
    );
}

#[test]
fn the_snapshot_pump_keeps_going_on_a_broken_app() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    start(&state);
    poison(state.bench.clone());
    for _ in 0..3 {
        crate::pump(&state);
    }
    assert!(broken(&state));
    assert_eq!(
        state
            .bench
            .locked()
            .snapshot(0, Views::Readouts)
            .readiness
            .record
            .unwrap()
            .need,
        motion_bench::coordination::readiness::Need::Broken
    );
}

#[cfg(feature = "developer-tools")]
#[test]
fn a_disarm_stuck_on_the_box_retires_its_controller() {
    let (_mock, _link, worker, source, _focus) = crate::controller::experiment::tests::setup();
    worker
        .arm(motion_bench::controller::linear::Config::default(), source)
        .unwrap();
    let worker = Arc::new(worker);
    let folder = tempfile::tempdir().unwrap();
    let mut state = state(folder.path());
    state.stop_within = Duration::from_millis(300);
    state.bench.locked().experiment = Some(worker.clone());
    let stuck = worker.hold_box();
    // no recording: stopping capture disarms, and the box never answers
    assert_eq!(stop_capture(&state), Ok(None));
    // the next arm makes a new controller, the stuck disarm can only reach its own journal
    assert!(state.bench.locked().experiment.is_none());
    drop(stuck);
}

#[test]
fn a_poisoned_capture_alone_breaks_the_app_and_its_recording_is_saved_as_failed() {
    let folder = tempfile::tempdir().unwrap();
    let state = state(folder.path());
    let stem = start(&state);
    reports(&state, 2_000_000);
    let data = state.bench.locked().capture.as_ref().unwrap().data.clone();
    poison(data.clone());
    assert!(!state.bench.is_poisoned());
    // the pump looks, sees it, and the failed recording is ended as any failed one is
    assert!(broken(&state));
    watch(&state, &data.locked());
    settled(&state);
    let (metadata, ..) = saved(folder.path(), &stem);
    assert_eq!(metadata["stop_reason"], "recording_failed");
    assert_eq!(metadata["outcome"], "failed");
}

#[cfg(feature = "developer-tools")]
#[test]
fn a_stuck_box_leaves_the_healthy_participants_their_time() {
    let (_mock, _link, worker, source, _focus) = crate::controller::experiment::tests::setup();
    worker
        .arm(motion_bench::controller::linear::Config::default(), source)
        .unwrap();
    let worker = Arc::new(worker);
    let folder = tempfile::tempdir().unwrap();
    let mut state = state(folder.path());
    // the box holds the whole deadline, which has to leave a slow CI runner time to stop the observers
    state.stop_within = Duration::from_secs(3);
    state.bench.locked().experiment = Some(worker.clone());
    let stem = start(&state);
    reports(&state, 2_000_000);
    let stuck = worker.hold_box();
    let _ = toggle(&state, Anchor::take(), Some(false));
    let (metadata, ..) = saved(folder.path(), &stem);
    let observers = metadata["participants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "observers")
        .unwrap()
        .clone();
    assert!(observers["error"].is_null(), "{observers}");
    drop(stuck);
}
