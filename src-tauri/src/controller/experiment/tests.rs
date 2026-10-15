use super::*;
use motion_bench::hardware::medius::api::{
    Class, DeviceInfo, DeviceKind, Direction, MockBox, Usage,
};

/// the crate's mock box as the link holds it now: an anybox device over it, its poller left
/// off so the test drives the box itself
fn boxed(mock: MockBox) -> anybox::Device {
    let (device, _poller) = anybox::Device::mock_medius_manual(
        mock,
        anybox::ClickTiming::Fixed(Duration::from_millis(5)),
    )
    .expect("a mock box opens");
    device
}
use motion_bench::{
    hardware::medius::{Command, Link},
    targets::vision::{Detection, Frame},
};
use std::time::{Duration, Instant};

fn wait(mut ready: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ready() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "worker did not settle"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
pub(crate) fn setup() -> (MockBox, Link, Worker, Source, Arc<AtomicBool>) {
    let mock = MockBox::new().with_device_info(DeviceInfo {
        vid: 0x373b,
        pid: 0x11d9,
        bcd_device: 0,
        bcd_usb: 0x0200,
        has_serial: false,
        has_bos: false,
        kind: DeviceKind::Mouse,
        product: "test".into(),
    });
    let link = Link::with_device(boxed(mock.clone()));
    wait(|| link.snapshot().kind == "mouse" && link.snapshot().render_mode.is_some());
    // the buttons come through the router, as in the app; its settings live in a folder
    // left for the system to clear
    let folder = tempfile::tempdir().unwrap().keep();
    let (controller, _requests) = hotkey::Controller::for_test(&folder);
    let buttons = Buttons::new(8);
    let (feed, routed) = (link.take_buttons().unwrap(), buttons.clone());
    std::thread::spawn(move || routed.route(feed, controller, Arc::new(AtomicBool::new(false))));
    let focused = Arc::new(AtomicBool::new(true));
    let focus = focused.clone();
    let worker = Worker::start_with_focus(
        link.injection_port().unwrap(),
        move || focus.load(Ordering::Relaxed),
        buttons,
    );
    wait(|| worker.snapshot().ready);
    let mut state = vision::Status::default();
    state.running = true;
    state.ready = true;
    let source = Source {
        config: motion_bench::targets::vision::Config {
            scenario: "single-target test".into(),
            ..Default::default()
        },
        status: Arc::new(Mutex::new(state)),
    };
    (mock, link, worker, source, focused)
}
fn buttons(mock: &MockBox, held: bool) {
    let usages = if held {
        vec![Usage::new(Class::Button, 3)]
    } else {
        vec![]
    };
    mock.push_usages(
        0,
        123,
        Class::Button,
        if held {
            Direction::PRESS
        } else {
            Direction::RELEASE
        },
        &usages,
    );
}
#[test]
fn arming_needs_only_a_named_scenario() {
    let (_mock, _link, worker, mut source, _) = setup();
    source.config.scenario = "  ".into();
    assert_eq!(
        worker.arm(Config::default(), source).unwrap_err(),
        "name the scenario in Capture before arming"
    );
}
fn frame(source: &Source, targets: usize, age_us: u64) {
    frame_at(source, targets, age_us, [1800.0, 720.0]);
}
fn frame_at(source: &Source, targets: usize, age_us: u64, center: [f64; 2]) {
    let now = wall_us();
    let anchor = motion_bench::capture::clock::Anchor::take();
    let presented = anchor.midpoint()
        - (age_us as i128 * motion_bench::capture::clock::frequency() as i128 / 1_000_000) as i64;
    let mut status = source.status.locked();
    status.last = Some(Frame {
        clock_anchor: anchor.clone(),
        source_qpc: presented,
        received_qpc: anchor.midpoint(),
        completed_qpc: anchor.midpoint(),
        source_us: now - age_us,
        received_us: now,
        completed_us: now,
        preprocess_ms: 0.0,
        pipeline_ms: 0.0,
        inference_ms: 0.0,
        skipped: 0,
        detections: (0..targets)
            .map(|_| Detection {
                bounds: [
                    center[0] - 20.0,
                    center[1] - 20.0,
                    center[0] + 20.0,
                    center[1] + 20.0,
                ],
                confidence: 1.0,
                class: 0,
            })
            .collect(),
    });
    if let Some(wakeup) = &status.experiment_wakeup {
        let _ = wakeup.try_send(());
    }
}
fn move_frame(worker: &Worker, source: &Source) {
    let began = Instant::now();
    while worker.snapshot().commands == 0 {
        frame(source, 1, 0);
        std::thread::sleep(Duration::from_millis(5));
        assert!(
            began.elapsed() < Duration::from_secs(5),
            "{}",
            serde_json::to_string(&worker.snapshot()).unwrap()
        );
    }
}
#[test]
fn catches_a_custom_mouse_button_and_moves_only_while_held_and_focused() {
    let (mock, _link, worker, source, focused) = setup();
    worker.buttons.learn_aim(false).unwrap();
    mock.push_usages(
        0,
        1,
        Class::Key,
        Direction::PRESS,
        &[Usage::new(Class::Key, 4)],
    );
    std::thread::sleep(Duration::from_millis(10));
    assert!(worker.snapshot().learning);
    buttons(&mock, true);
    wait(|| worker.snapshot().binding_seq == 1);
    assert_eq!(worker.snapshot().hotkey_mask, 8);
    buttons(&mock, false);
    wait(|| worker.snapshot().held_mask == 0);
    worker.arm(Config::default(), source.clone()).unwrap();
    assert!(worker.arm(Config::default(), source.clone()).is_err());
    frame(&source, 1, 0);
    std::thread::sleep(Duration::from_millis(5));
    assert_eq!(worker.snapshot().commands, 0);
    focused.store(false, Ordering::Relaxed);
    buttons(&mock, true);
    wait(|| worker.snapshot().phase == "Waiting for the game to be in front");
    assert_eq!(worker.snapshot().commands, 0);
    focused.store(true, Ordering::Relaxed);
    move_frame(&worker, &source);
    assert!(worker.snapshot().counts[0] > 0);
    let moves = mock
        .recorded_frames()
        .into_iter()
        .filter(|f| f.payload.len() == 6 && f.payload[0] == 0)
        .collect::<Vec<_>>();
    assert!(
        moves
            .iter()
            .any(|f| f.payload[5] == 0 && f.payload[1..5] != [0; 4]),
        "normal MOVE must respect firmware rendering"
    );
    buttons(&mock, false);
    wait(|| worker.snapshot().phase == "Hold the hotkey");
    let count = worker.snapshot().commands;
    for _ in 0..3 {
        frame(&source, 1, 0);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(worker.snapshot().commands, count);
    worker.disarm().unwrap();
    buttons(&mock, true);
    frame(&source, 1, 0);
    std::thread::sleep(Duration::from_millis(5));
    assert_eq!(worker.snapshot().commands, count);
}
#[test]
fn centered_target_pauses_injections_and_hysteresis_waits_for_a_new_offset() {
    let (mock, _link, worker, source, _focus) = setup();
    worker.arm(Config::default(), source.clone()).unwrap();
    buttons(&mock, true);
    move_frame(&worker, &source);
    frame_at(&source, 1, 0, [1280.0, 720.0]);
    wait(|| worker.snapshot().phase == "Settled in deadzone");
    assert!(!worker.snapshot().active);
    let commands = worker.snapshot().commands;
    for center in [[1285.0, 720.0], [1275.0, 720.0], [1280.0, 727.0]] {
        frame_at(&source, 1, 0, center);
        let target = source.config.counts(center[0], center[1]);
        wait(|| worker.snapshot().target_counts == Some(target));
        assert_eq!(worker.snapshot().commands, commands);
        assert_eq!(worker.snapshot().phase, "Settled in deadzone");
    }
    std::thread::sleep(Duration::from_millis(5));
    frame_at(&source, 1, 0, [1320.0, 720.0]);
    wait(|| worker.snapshot().commands > commands);
    assert_eq!(worker.snapshot().phase, "Tracking");
}
#[test]
fn ambiguity_staleness_and_disconnect_stop_new_injections() {
    let (mock, link, worker, source, _focus) = setup();
    worker.arm(Config::default(), source.clone()).unwrap();
    buttons(&mock, true);
    frame(&source, 2, 0);
    wait(|| worker.snapshot().phase == "Waiting for exactly one target");
    assert_eq!(worker.snapshot().commands, 0);
    frame(&source, 1, 100_000);
    wait(|| worker.snapshot().phase == "Waiting for a fresh target frame");
    assert_eq!(worker.snapshot().commands, 0);
    move_frame(&worker, &source);
    // a box let go of is closed, so nothing can come back on it and the aim ends
    link.send(Command::Disconnect);
    wait(|| worker.snapshot().error.is_some());
    assert!(!worker.snapshot().armed);
    let count = worker.snapshot().commands;
    frame(&source, 1, 0);
    let _ = worker.wakeup.try_send(());
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(worker.snapshot().commands, count);
}
#[test]
fn native_presentation_time_controls_freshness_despite_bad_epoch_mapping() {
    let (mock, _link, worker, source, _focus) = setup();
    worker.arm(Config::default(), source.clone()).unwrap();
    buttons(&mock, true);
    let began = Instant::now();
    while worker.snapshot().commands == 0 {
        frame(&source, 1, 5_000_000);
        {
            let mut state = source.status.locked();
            let time = motion_bench::capture::clock::qpc();
            let frame = state.last.as_mut().unwrap();
            frame.source_qpc = time;
            frame.received_qpc = time;
            frame.completed_qpc = time;
        }
        let _ = worker.wakeup.try_send(());
        std::thread::sleep(Duration::from_millis(5));
        assert!(began.elapsed() < Duration::from_secs(5));
    }
    frame(&source, 1, 0);
    {
        let mut state = source.status.locked();
        let time = motion_bench::capture::clock::qpc() - motion_bench::capture::clock::frequency();
        let frame = state.last.as_mut().unwrap();
        frame.source_qpc = time;
    }
    let _ = worker.wakeup.try_send(());
    wait(|| worker.snapshot().phase == "Waiting for a fresh target frame");
    assert!(!worker.snapshot().active);
}
#[test]
fn repeated_notifications_do_not_reuse_a_frame_or_accumulate_paused_time() {
    let (mock, _link, worker, source, _focus) = setup();
    worker.arm(Config::default(), source.clone()).unwrap();
    buttons(&mock, true);
    move_frame(&worker, &source);
    std::thread::sleep(Duration::from_millis(5));
    let count = worker.snapshot().commands;
    for _ in 0..10 {
        let _ = worker.wakeup.try_send(());
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(worker.snapshot().commands, count);
    buttons(&mock, false);
    wait(|| worker.snapshot().held_mask == 0 && worker.snapshot().phase == "Hold the hotkey");
    std::thread::sleep(Duration::from_millis(60));
    buttons(&mock, true);
    frame(&source, 1, 0);
    wait(|| worker.snapshot().active);
    assert_eq!(worker.snapshot().commands, count);
}
/// the held buttons of a lost feed are unknown, so an armed controller waits for the feed and
/// then for a press of the aim button; with no feed it can't be armed at all
#[test]
fn a_lost_button_feed_waits_and_the_aim_comes_back() {
    use motion_bench::hardware::medius::Feed;
    let (_mock, link, original, source, _focus) = setup();
    original.stop();
    let folder = tempfile::tempdir().unwrap();
    let (controller, _requests) = hotkey::Controller::for_test(folder.path());
    let buttons = Buttons::new(8);
    let (feed, read) = std::sync::mpsc::channel();
    let routed = buttons.clone();
    std::thread::spawn(move || routed.route(read, controller, Arc::new(AtomicBool::new(false))));
    let worker = Worker::start_with_focus(link.injection_port().unwrap(), || true, buttons.clone());
    assert!(worker.arm(Config::default(), source.clone()).is_err());
    feed.send(Feed::Ready).unwrap();
    wait(|| worker.snapshot().ready);
    worker.arm(Config::default(), source.clone()).unwrap();
    assert!(buttons.aim().armed);
    feed.send(Feed::Lost("the box disconnected".into()))
        .unwrap();
    let _ = worker.wakeup.try_send(());
    wait(|| worker.snapshot().away);
    let status = worker.snapshot();
    assert!(status.armed && status.error.is_none());
    // back, the held buttons are a guess until the box reports some, so it waits for a
    // press of the aim button
    feed.send(Feed::Ready).unwrap();
    let _ = worker.wakeup.try_send(());
    wait(|| worker.snapshot().phase == "Press the aim button again");
    assert!(!worker.snapshot().away);
    assert!(worker.snapshot().armed && buttons.aim().armed);
    feed.send(Feed::Buttons {
        mask: 8,
        at: motion_bench::capture::clock::Anchor::take(),
    })
    .unwrap();
    wait(|| {
        frame(&source, 1, 0);
        worker.snapshot().phase != "Press the aim button again"
    });
    assert!(worker.snapshot().armed);
}
/// a worker replaced while a handle to it was still out: dropping that handle later leaves the
/// new worker armed and woken by frames
#[test]
fn a_late_drop_of_a_stopped_worker_leaves_its_successor_alone() {
    let (_mock, link, first, source, _focus) = setup();
    let buttons = first.buttons.clone();
    first.arm(Config::default(), source.clone()).unwrap();
    let first = Arc::new(first);
    let late = first.clone();
    first.stop();
    drop(first);
    let worker = Worker::start_with_focus(link.injection_port().unwrap(), || true, buttons.clone());
    wait(|| worker.snapshot().ready);
    worker.arm(Config::default(), source.clone()).unwrap();
    drop(late);
    assert!(source.status.locked().experiment_wakeup.is_some());
    assert!(buttons.aim().armed);
}
/// a write to the box that does not return holds only the io lock: the status still reads,
/// and a disarm meanwhile lets no move out after it
#[test]
fn a_stuck_box_write_blocks_no_snapshot_and_no_move_follows_a_disarm() {
    let (mock, _link, worker, source, _focus) = setup();
    worker.arm(Config::default(), source.clone()).unwrap();
    buttons(&mock, true);
    move_frame(&worker, &source);
    let moves = || {
        mock.recorded_frames()
            .into_iter()
            // a move carries counts; a discard is the same frame with only its flag set
            .filter(|f| f.payload.len() == 6 && f.payload[5] == 0 && f.payload[1..5] != [0; 4])
            .count()
    };
    let worker = Arc::new(worker);
    let stuck = worker.io.locked();
    frame_at(&source, 1, 0, [1900.0, 720.0]);
    std::thread::sleep(Duration::from_millis(30));
    let asked = Instant::now();
    let status = worker.snapshot();
    assert!(asked.elapsed() < Duration::from_millis(20));
    assert!(status.armed);
    let disarming = worker.clone();
    let disarm = std::thread::spawn(move || disarming.disarm());
    wait(|| !worker.snapshot().armed);
    let before = moves();
    drop(stuck);
    disarm.join().unwrap().unwrap();
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(moves(), before);
}
/// the box restarts, and the aim hears of it at once rather than on its next frame
fn restart(mock: &MockBox, link: &Link, worker: &Worker) {
    let generation = link.snapshot().generation;
    mock.restart();
    wait(|| link.snapshot().generation > generation);
    let _ = worker.wakeup.try_send(());
}
/// a box that restarts while armed leaves the aim waiting, and once it's back the aim moves
/// again only after a fresh press of the aim button
#[test]
fn a_restarted_box_comes_back_to_a_fresh_press() {
    let (mock, link, worker, source, _focus) = setup();
    worker.arm(Config::default(), source.clone()).unwrap();
    buttons(&mock, true);
    move_frame(&worker, &source);
    restart(&mock, &link, &worker);
    wait(|| worker.snapshot().away);
    wait(|| {
        let _ = worker.wakeup.try_send(());
        worker.snapshot().phase == "Press the aim button again"
    });
    let status = worker.snapshot();
    assert!(status.armed && !status.away && status.error.is_none());
    let count = status.commands;
    buttons(&mock, false);
    buttons(&mock, true);
    let began = Instant::now();
    while worker.snapshot().commands == count {
        frame(&source, 1, 0);
        std::thread::sleep(Duration::from_millis(5));
        assert!(
            began.elapsed() < Duration::from_secs(5),
            "no move after the fresh press"
        );
    }
}
/// a box that goes during a recording ends the aim and fails its journal, which says what
/// went out
#[test]
fn a_box_gone_during_a_recording_ends_the_aim() {
    let (mock, link, worker, source, _focus) = setup();
    let directory = tempfile::tempdir().unwrap();
    let pcap = directory.path().join("trial.pcap");
    worker.arm(Config::default(), source.clone()).unwrap();
    worker.begin(&pcap).unwrap();
    buttons(&mock, true);
    move_frame(&worker, &source);
    restart(&mock, &link, &worker);
    wait(|| worker.snapshot().error.is_some());
    let status = worker.snapshot();
    assert!(!status.armed && !status.away);
    assert!(status.error.unwrap().contains("during the recording"));
    let evidence: serde_json::Value =
        serde_json::from_slice(&std::fs::read(pcap.with_extension("injection.json")).unwrap())
            .unwrap();
    assert_eq!(evidence["complete"], false);
}
#[test]
fn injection_evidence_finalizes_and_preserves_failed_shutdown() {
    let (mock, _link, worker, source, _focus) = setup();
    let directory = std::env::temp_dir().join(format!("motion-bench-experiment-{}", wall_us()));
    std::fs::create_dir(&directory).unwrap();
    let pcap = directory.join("trial.pcap");
    worker.arm(Config::default(), source.clone()).unwrap();
    worker.begin(&pcap).unwrap();
    assert!(worker.snapshot().recording);
    buttons(&mock, true);
    move_frame(&worker, &source);
    worker.disarm().unwrap();
    let evidence: serde_json::Value =
        serde_json::from_slice(&std::fs::read(pcap.with_extension("injection.json")).unwrap())
            .unwrap();
    assert_eq!(evidence["complete"], true);
    assert_eq!(evidence["config"]["hotkey_mask"], 8);
    let rows = std::fs::read_to_string(pcap.with_extension("injections.jsonl")).unwrap();
    assert_eq!(
        rows.lines().count() as u64,
        evidence["commands"].as_u64().unwrap()
    );
    for row in rows.lines() {
        let row: serde_json::Value = serde_json::from_str(row).unwrap();
        assert_eq!(row["held_mask"], 8);
        assert!(row["dx"].as_i64().unwrap() > 0);
        assert!(row["completed_qpc"].as_i64().unwrap() >= row["sent_qpc"].as_i64().unwrap());
    }
    worker.arm(Config::default(), source.clone()).unwrap();
    let failed = directory.join("failed.pcap");
    worker.begin(&failed).unwrap();
    {
        let mut journal = worker.journal.locked();
        let sink = journal.as_mut().unwrap();
        sink.writer =
            BufWriter::new(File::open(failed.with_extension("injections.jsonl")).unwrap());
        sink.writer.write_all(b"evidence").unwrap();
    }
    assert!(worker.disarm().is_err());
    let failed: serde_json::Value =
        serde_json::from_slice(&std::fs::read(failed.with_extension("injection.json")).unwrap())
            .unwrap();
    assert_eq!(failed["complete"], false);
    assert!(failed["error"].is_string());
    assert!(worker.snapshot().error.is_some());
    drop(worker);
    std::fs::remove_dir_all(directory).unwrap();
}
