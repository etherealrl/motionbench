use super::*;
use crate::sync::LockExt;
use medius::{Class, DeviceInfo, DeviceKind, MockBox};
use std::time::Instant;
#[test]
fn failed_auto_readback_releases_the_port_and_does_not_keep_querying() {
    let mock = MockBox::new();
    let device = Arc::new(boxed(mock.clone()));
    let published = Arc::new(Mutex::new(Some(device.clone())));
    let (_, rx) = mpsc::sync_channel(1);
    let (motion, _) = mpsc::sync_channel(1);
    let state = Arc::new(Mutex::new(Snapshot {
        connected: true,
        error: Some("health timeout. asking again every second".into()),
        ..Default::default()
    }));
    let mut worker = Worker {
        device: Some(device),
        rx,
        state: state.clone(),
        motion,
        catch: None,
        buttons: mpsc::channel().0,
        button_feed: None,
        clock: Arc::default(),
        read_to: Arc::default(),
        hold: Arc::default(),
        counters: (0, 0),
        published: published.clone(),
        behind: (false, None),
        boxes: Box::new(crate::hardware::boxes::Plugged),
        remembered: None,
        off: false,
        seen: None,
        found: Scan::default(),
    };
    worker.disable_failed_startup();
    mock.clear_recorded();
    for _ in 0..3 {
        worker.refresh();
    }
    assert_eq!(mock.recorded(), 0);
    assert!(worker.device.is_none());
    assert!(published.locked().is_none());
    let status = state.locked();
    assert!(!status.connected && !status.connecting);
    assert!(status.error.as_ref().unwrap().contains("retry"));
    assert!(!status.error.as_ref().unwrap().contains("asking again"));
}

fn settle(link: &Link, ready: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let started = Instant::now();
    loop {
        let s = link.snapshot();
        if ready(&s) || started.elapsed() > Duration::from_secs(20) {
            return s;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// the crate's mock box as the link now holds it: the anybox device over it, whose poller is left
/// off so a test drives the box itself and no background traffic reaches the mock
fn boxed(mock: MockBox) -> anybox::Device {
    let (device, _poller) = anybox::Device::mock_medius_manual(
        mock,
        anybox::ClickTiming::Fixed(Duration::from_millis(5)),
    )
    .expect("a mock box opens");
    device
}

fn mock() -> MockBox {
    MockBox::new()
        // the maintainer's box, so what anybox reads out of the box is the real thing
        .with_version(medius::Version {
            proto_ver: 9,
            fw_major: 3,
            fw_minor: 4,
            fw_patch: 5,
            mac: [0x3c, 0xdc, 0x75, 0x52, 0x3e, 0x50],
            name: "medius".into(),
        })
        .with_render(RenderMode::Off, false)
        .with_device_info(DeviceInfo {
            vid: 0x373b,
            pid: 0x11d9,
            bcd_device: 0,
            bcd_usb: 0x0200,
            has_serial: false,
            has_bos: false,
            kind: DeviceKind::Mouse,
            product: "VXE".into(),
        })
}

#[test]
fn a_box_shows_up_in_the_snapshot_as_what_it_says_it_is() {
    let link = Link::with_device(boxed(mock()));

    let s = settle(&link, |s| s.connected && s.render_mode.is_some());

    assert!(s.connected);
    assert_eq!(s.kind, "mouse");
    assert!(s.clone.starts_with("373B:11D9"));
    assert_eq!(s.render_mode, Some(RenderMode::Off));
    assert!(!s.render_full);
    // the box's own identity, which is what the window and the sidecar show
    assert_eq!(s.name, "medius");
    assert_eq!(s.firmware, "fw 3.4.5");
    assert_eq!(s.fw, (3, 4, 5));
    assert_eq!(s.mac, "3cdc75523e50");
    assert!(s.has_mouse);
}

/// OPTION is opcode 0x11 with payload id, value. RENDER is id 5, despiked is 2, full is 1
#[test]
fn a_render_change_goes_out_as_option_five() {
    let mock = mock();
    let link = Link::with_device(boxed(mock.clone()));
    settle(&link, |s| s.connected);

    link.send(Command::Render(RenderMode::Despiked, true));

    let started = Instant::now();
    let sent = loop {
        let frames = mock.recorded_frames();
        if frames.iter().any(|f| f.payload == [5, 2, 1]) {
            break true;
        }
        if started.elapsed() > Duration::from_secs(8) {
            break false;
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    assert!(
        sent,
        "no OPTION(RENDER despiked, full) frame reached the box"
    );
}

#[test]
fn the_sidecar_json_names_the_mode_the_run_was_made_under() {
    let link = Link::with_device(boxed(mock()));
    let s = settle(&link, |s| s.connected && s.render_mode.is_some());

    let json = s.json();

    assert_eq!(
        json["render"],
        serde_json::json!({ "mode": "off", "full": false, "ready": false, "effective": "relay" })
    );
    assert_eq!(json["clone"], "373B:11D9 VXE");
    assert_eq!(json["kind"], "mouse");
    assert!(json["firmware"]["device_major"].is_u64());
    assert!(json["firmware"]["protocol"].is_u64());
    assert_eq!(s.effective(), "relay");
}

#[test]
fn names_with_control_characters_round_trip_through_the_sidecar_json() {
    let name = "line\nbreak\ttab \"quoted\" back\\slash \u{1} \u{7f}";
    let s = Snapshot {
        name: name.into(),
        mac: name.into(),
        clone: name.into(),
        kind: name.into(),
        ..Snapshot::default()
    };
    let text = s.json().to_string();
    let back: serde_json::Value = serde_json::from_str(&text).unwrap();
    for key in ["name", "mac", "clone", "kind"] {
        assert_eq!(back[key], name, "{key}");
    }
}

#[test]
fn the_sidecar_json_keeps_every_field_under_its_name() {
    // each flag gets its own 3 bit code across three snapshots, so any two swapped flags differ in
    // at least one of them
    for bit in 0..3 {
        let flag = |code: u8| code >> bit & 1 == 1;
        let s = Snapshot {
            name: "box".into(),
            port: Some("COM5".into()),
            mac: "aa:bb".into(),
            clone: "373B:11D9 VXE".into(),
            kind: "mouse".into(),
            has_report_id: Some(true),
            fw: (3, 4, 5),
            host_fw: Some((6, 7, 8)),
            protocol: 9,
            native_period_us: 125,
            rate_confident: flag(0b001),
            render_mode: Some(RenderMode::Despiked),
            render_full: flag(0b010),
            render_ready: flag(0b011),
            emit_mode: Some(EmitPace::Fixed(4000)),
            resolved_hz: 4001,
            advertised_hz: 8000,
            force_hz: Some(2000),
            force_active: flag(0b100),
            spread_percent: 12,
            spread_span_us: 345,
            riding: Some(Some(Duration::from_millis(1500))),
            imperfect_allowed: flag(0b101),
            clone_imperfect: flag(0b110),
            ..Snapshot::default()
        };
        assert_eq!(
            s.json(),
            serde_json::json!({
                "name": "box", "port": "COM5", "mac": "aa:bb", "clone": "373B:11D9 VXE", "kind": "mouse", "has_mouse": false,
                "report_id": true,
                "firmware": {
                    "device_major": 3, "device_minor": 4, "device_patch": 5,
                    "host_major": 6, "host_minor": 7, "host_patch": 8, "protocol": 9,
                },
                "native_period_us": 125, "rate_confident": flag(0b001),
                "render": {
                    "mode": "despiked", "full": flag(0b010), "ready": flag(0b011),
                    "effective": s.effective(),
                },
                "emit": {
                    "mode": "fixed", "fixed_hz": 4000, "resolved_hz": 4001, "advertised_hz": 8000,
                    "force_hz": 2000, "force_active": flag(0b100),
                },
                "spread": { "percent": 12, "span_us": 345 },
                "riding_ms": 1500, "imperfect_allowed": flag(0b101),
                "clone_imperfect": flag(0b110),
            }),
            "bit {bit}"
        );
    }
    for (mode, name) in [
        (RenderMode::Off, "off"),
        (RenderMode::Stock, "stock"),
        (RenderMode::Despiked, "despiked"),
        (RenderMode::Unsmoothed, "unsmoothed"),
    ] {
        let s = Snapshot {
            render_mode: Some(mode),
            ..Snapshot::default()
        };
        assert_eq!(s.json()["render"]["mode"], name);
    }
    for (pace, name, fixed) in [
        (EmitPace::Learned, "learned", None),
        (EmitPace::Interval, "interval", None),
        (EmitPace::Fixed(1000), "fixed", Some(1000)),
    ] {
        let emit = Snapshot {
            emit_mode: Some(pace),
            ..Snapshot::default()
        }
        .json()["emit"]
            .clone();
        assert_eq!(emit["mode"], name);
        assert_eq!(emit["fixed_hz"], serde_json::json!(fixed));
    }
    let unknown = Snapshot::default().json();
    for (block, key) in [
        ("firmware", "host_major"),
        ("emit", "fixed_hz"),
        ("emit", "force_hz"),
    ] {
        assert!(unknown[block][key].is_null(), "{block}.{key}");
    }
    assert!(unknown["riding_ms"].is_null() && unknown["report_id"].is_null());
    assert_eq!(unknown["render"]["mode"], "unknown");
    assert_eq!(unknown["emit"]["mode"], "unknown");
}

#[test]
fn a_mode_without_an_armed_profile_is_still_a_relay() {
    let armed = Snapshot {
        render_mode: Some(RenderMode::Despiked),
        render_full: true,
        render_ready: true,
        ..Snapshot::default()
    };
    let unarmed = Snapshot {
        render_ready: false,
        ..armed.clone()
    };
    let not_full = Snapshot {
        render_full: false,
        ..armed.clone()
    };
    assert_eq!(armed.effective(), "rendered");
    assert_eq!(unarmed.effective(), "relay");
    assert_eq!(not_full.effective(), "relay");
}

/// the mock pushes motion frames as the box would; they come out the channel with the
/// stamp unwrapped past the u32 rollover
#[test]
fn what_the_box_saw_reaches_the_channel_with_the_clock_unwrapped() {
    let mock = mock();
    let link = Link::with_device(boxed(mock.clone()));
    settle(&link, |s| s.connected);

    link.send(Command::Catch(true));
    settle(&link, |s| s.catching);
    mock.push_motion(0, u32::MAX - 1000, 3, -2, 0, 0);
    mock.push_motion(1, 500, 1, 1, 0, 0);

    let first = link
        .motion
        .recv_timeout(Duration::from_secs(5))
        .expect("the first motion");
    let second = link
        .motion
        .recv_timeout(Duration::from_secs(5))
        .expect("the second motion");

    assert_eq!(
        (first.box_us, first.dx, first.dy),
        (u32::MAX as u64 - 1000, 3, -2)
    );
    assert_eq!(second.box_us, (1u64 << 32) + 500);
    assert_eq!((second.dx, second.dy), (1, 1));
}

#[test]
fn disconnect_forgets_the_box() {
    let mock = mock();
    let link = Link::with_device(boxed(mock.clone()));
    settle(&link, |s| s.connected);

    link.send(Command::Disconnect);

    let s = settle(&link, |s| !s.connected);
    assert!(!s.connected);
    // the box is let go: nothing asks it anything again, and the port is released off the worker's
    // thread (which only a real box can show, the mock has no port to lose)
    mock.clear_recorded();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(mock.recorded(), 0);
    assert!(s.name.is_empty());
}

/// the box's buttons reach the feed as held masks once it is up, two edges of one report as one
/// mask; a disconnect says it is lost
#[test]
fn the_buttons_come_through_the_feed_and_a_disconnect_loses_it() {
    use medius::{Direction, Usage};
    let mock = mock();
    let link = Link::with_device(boxed(mock.clone()));
    let feed = link.take_buttons().unwrap();
    assert!(link.take_buttons().is_none());
    assert!(matches!(
        feed.recv_timeout(Duration::from_secs(5)),
        Ok(Feed::Ready)
    ));
    let button = |id| Usage {
        class: Class::Button,
        id,
    };
    mock.push_usages(
        0,
        10,
        Class::Button,
        Direction::PRESS,
        &[button(0), button(3)],
    );
    let held = loop {
        match feed
            .recv_timeout(Duration::from_secs(5))
            .expect("a feed event")
        {
            Feed::Buttons { mask, .. } => break mask,
            _ => continue,
        }
    };
    // both buttons in the first mask, never the left one alone first
    assert_eq!(held, 0b1001);
    link.send(Command::Disconnect);
    let lost = loop {
        match feed
            .recv_timeout(Duration::from_secs(5))
            .expect("a feed event")
        {
            Feed::Lost(reason) => break reason,
            _ => continue,
        }
    };
    assert!(lost.contains("isn't connected"));
}

/// a box that restarts leaves its held buttons unknown: the feed says it lost them and comes
/// back up holding none
#[test]
fn a_box_restart_clears_the_held_buttons() {
    use medius::{Direction, Usage};
    let mock = mock();
    let (device, mut poller) = anybox::Device::mock_medius_manual(
        mock.clone(),
        anybox::ClickTiming::Fixed(Duration::from_millis(5)),
    )
    .expect("a mock box opens");
    let link = Link::with_device(device);
    let feed = link.take_buttons().unwrap();
    let next = || {
        feed.recv_timeout(Duration::from_secs(5))
            .expect("a feed event")
    };
    assert!(matches!(next(), Feed::Ready));
    let left = Usage {
        class: Class::Button,
        id: 0,
    };
    mock.push_usages(0, 10, Class::Button, Direction::PRESS, &[left]);
    assert!(matches!(next(), Feed::Buttons { mask: 1, .. }));
    mock.restart();
    let started = Instant::now();
    let lost = loop {
        poller.tick();
        match feed.recv_timeout(Duration::from_millis(100)) {
            Ok(Feed::Lost(reason)) => break reason,
            _ if started.elapsed() > Duration::from_secs(8) => panic!("no Lost after a restart"),
            _ => {}
        }
    };
    assert!(!lost.is_empty());
    let up = loop {
        poller.tick();
        match feed.recv_timeout(Duration::from_millis(100)) {
            Ok(Feed::Ready) => break true,
            _ if started.elapsed() > Duration::from_secs(8) => break false,
            _ => {}
        }
    };
    assert!(up);
    // the left button held before the restart is gone from what comes next
    let right = Usage {
        class: Class::Button,
        id: 1,
    };
    mock.push_usages(0, 20, Class::Button, Direction::PRESS, &[right]);
    let held = loop {
        poller.tick();
        match feed.recv_timeout(Duration::from_millis(100)) {
            Ok(Feed::Buttons { mask, .. }) if mask != 0 => break mask,
            _ if started.elapsed() > Duration::from_secs(8) => panic!("no press after a restart"),
            _ => {}
        }
    };
    assert_eq!(held, 0b10);
}

/// each button lands on its medius usage id, the bit the saved bindings keep, and a button past
/// the mask's 32 bits leaves it alone
#[test]
fn buttons_land_on_their_usage_ids() {
    use anybox::Button;
    let bits = [
        (Button::Left, 0),
        (Button::Right, 1),
        (Button::Middle, 2),
        (Button::Side1, 3),
        (Button::Side2, 4),
        (Button::Extra(7), 7),
    ];
    for (button, bit) in bits {
        assert_eq!(with_button(0, button, true), 1 << bit);
        assert_eq!(with_button(1 << bit, button, false), 0);
    }
    assert_eq!(with_button(0b10, Button::Extra(40), true), 0b10);
}

/// starting or stopping the box's input stream is no change of the box: the aim controller's
/// moves still go out, while the input rows on either side are told apart
#[test]
fn the_input_stream_toggling_leaves_moves_going_out() {
    let link = Link::with_device(boxed(mock()));
    settle(&link, |s| s.connected && !s.kind.is_empty());
    let port = link.injection_port().unwrap();
    let before = link.snapshot().discontinuities;
    link.send(Command::Catch(true));
    settle(&link, |s| s.catching);
    link.send(Command::Catch(false));
    let after = settle(&link, |s| !s.catching && s.pending_commands == 0);
    assert!(after.discontinuities > before);
    assert_eq!(port.state(), PortState::Usable);
    link.send(Command::Disconnect);
    settle(&link, |s| !s.connected);
    // a box let go of is closed, so the aim's port can't come back on it
    assert!(matches!(port.state(), PortState::Broken(_)));
}

/// a recording that ends sees the box's input stopped only together with the break in it, never
/// stopped with the rows on either side still one stream
#[test]
fn a_stopped_input_stream_is_never_seen_without_its_break() {
    let link = Link::with_device(boxed(mock()));
    settle(&link, |s| s.connected);
    for _ in 0..5 {
        link.send(Command::Catch(true));
        let running = settle(&link, |s| s.catching);
        assert!(running.catching, "the stream never started");
        link.send(Command::Catch(false));
        loop {
            let s = link.snapshot();
            if !s.catching {
                assert!(s.discontinuities > running.discontinuities);
                break;
            }
        }
    }
}

/// the host chip's rollover count outlives the input stream, so a stream started again after
/// the counter wrapped goes on in the same clock
#[test]
fn the_clock_rollover_survives_an_input_stream_restart() {
    let mock = mock();
    let link = Link::with_device(boxed(mock.clone()));
    settle(&link, |s| s.connected);
    link.send(Command::Catch(true));
    settle(&link, |s| s.catching);
    mock.push_motion(0, u32::MAX - 1000, 1, 0, 0, 0);
    link.motion.recv_timeout(Duration::from_secs(5)).unwrap();
    link.send(Command::Catch(false));
    settle(&link, |s| !s.catching);
    link.send(Command::Catch(true));
    settle(&link, |s| s.catching);
    mock.push_motion(1, 500, 1, 0, 0, 0);
    let after = link.motion.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(after.box_us, (1u64 << 32) + 500);
}

/// a box that restarts counts from zero again: its stamps go back rather than reading as a
/// rollover, and the rows on either side are told apart
#[test]
fn a_box_restart_starts_the_input_clock_again() {
    let mock = mock();
    let (device, mut poller) = anybox::Device::mock_medius_manual(
        mock.clone(),
        anybox::ClickTiming::Fixed(Duration::from_millis(5)),
    )
    .expect("a mock box opens");
    let link = Link::with_device(device);
    settle(&link, |s| s.connected);
    link.send(Command::Catch(true));
    settle(&link, |s| s.catching);
    mock.push_motion(0, u32::MAX - 1000, 1, 0, 0, 0);
    link.motion.recv_timeout(Duration::from_secs(5)).unwrap();
    let discontinuities = link.snapshot().discontinuities;
    mock.restart();
    let started = Instant::now();
    while link.snapshot().discontinuities == discontinuities {
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "no restart seen"
        );
        poller.tick();
        std::thread::sleep(Duration::from_millis(50));
    }
    // the reconnect comes after the disconnect, and a stamp before it would be unwrapped again
    for _ in 0..10 {
        poller.tick();
        std::thread::sleep(Duration::from_millis(50));
    }
    mock.push_motion(0, 500, 1, 0, 0, 0);
    let after = link.motion.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(after.box_us, 500);
}

/// what the box says it lost while streaming adds up as the input's losses
#[test]
fn what_the_box_lost_counts_against_its_input() {
    let mock = mock();
    let (device, mut poller) = anybox::Device::mock_medius_manual(
        mock.clone(),
        anybox::ClickTiming::Fixed(Duration::from_millis(5)),
    )
    .expect("a mock box opens");
    let catching = device.medius().unwrap();
    let link = Link::with_device(device);
    settle(&link, |s| s.connected);
    link.send(Command::Catch(true));
    settle(&link, |s| s.catching);
    let started = Instant::now();
    for (more, total) in [(5, 5), (3, 8)] {
        let mut state = catching.query_catch().unwrap();
        state.dropped += more;
        let _ = mock.clone().with_catch_state(state);
        while link.snapshot().catch_dropped < total {
            assert!(
                started.elapsed() < Duration::from_secs(8),
                "no drop seen: {} of {total}, {:?}",
                link.snapshot().catch_dropped,
                link.snapshot().error
            );
            poller.tick();
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    assert_eq!(link.snapshot().catch_dropped, 8);
}

/// a row's receipt is when anybox took it, not when the app got round to it, and the wheel lands
/// in dz
#[test]
fn a_row_keeps_the_moment_anybox_took_it() {
    let mock = mock();
    let device = boxed(mock.clone());
    let events = device.events_of(anybox::EventKinds::MOTION);
    std::thread::sleep(Duration::from_millis(300));
    mock.push_motion(0, 1000, 1, 2, 3, 0);
    let pushed = crate::capture::clock::qpc();
    // the app is busy elsewhere for a while
    std::thread::sleep(Duration::from_millis(300));
    let (rows, read) = mpsc::sync_channel(4);
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    let route = std::thread::spawn(move || {
        let state = Mutex::new(Snapshot::default());
        route_motion(
            &events,
            &stopping,
            &rows,
            &state,
            &Mutex::new((0, 0)),
            &AtomicI64::new(0),
        )
    });
    let row = read.recv_timeout(Duration::from_secs(5)).expect("a row");
    stop.store(true, Ordering::Relaxed);
    route.join().unwrap();
    let late_ms = (row.arrived_qpc - pushed) * 1000 / crate::capture::clock::frequency();
    assert!(
        late_ms < 100,
        "the receipt came {late_ms} ms after the push"
    );
    let mapped = crate::capture::clock::Anchor::take()
        .epoch_us(row.arrived_qpc, crate::capture::clock::frequency())
        .unwrap();
    assert!(mapped.abs_diff(row.arrived_us) < 1_000);
    assert_eq!((row.box_us, row.dx, row.dy, row.dz), (1000, 1, 2, 3));
}

/// after a link event the clock may have restarted or gone on: a restart reads as time going
/// back, a clock that went on past a rollover carries on, and neither reads as a rollover
#[test]
fn a_link_event_never_reads_as_a_rollover() {
    let mut clock = (0, 0);
    unwrapped(&mut clock, u32::MAX - 1000);
    let rolled = unwrapped(&mut clock, 500_000);
    assert_eq!(rolled, (1u64 << 32) + 500_000);

    let mut went_on = clock;
    unsure_of_restart(&mut went_on);
    assert_eq!(unwrapped(&mut went_on, 501_000), (1u64 << 32) + 501_000);

    let mut restarted = (u32::MAX - 1000, 0);
    unsure_of_restart(&mut restarted);
    assert_eq!(unwrapped(&mut restarted, 500), 500);
}

/// a thread held up inside the anchor makes a receipt late, never early: the paired shift takes
/// the lowest offset, so an early one would move it
#[test]
fn a_receipt_mapped_to_utc_is_never_early() {
    let frequency = 10_000_000;
    let anchor = crate::capture::clock::Anchor {
        qpc_before: 1_000_000,
        // 20 ms held up inside it
        qpc_after: 1_200_000,
        utc_us: 5_000_000,
    };
    let receipt = 900_000;
    let mapped = anchor.utc_never_early(receipt, frequency).unwrap();
    // the utc read happened somewhere in the bracket, so the receipt's true utc is at most this
    for read_at in [anchor.qpc_before, anchor.qpc_after] {
        let truth = anchor.utc_us - ((read_at - receipt) * 1_000_000 / frequency) as u64;
        assert!(mapped >= truth);
    }
    assert_eq!(mapped, 4_990_000);
}

/// a recording that ends waits until what anybox had by then is read, and a link with no input
/// stream has nothing to wait for
#[test]
fn the_input_is_read_up_to_the_end_of_a_recording() {
    let mock = mock();
    let link = Link::with_device(boxed(mock.clone()));
    settle(&link, |s| s.connected);
    let reader = link.input_reader();
    assert!(reader.caught_up(i64::MAX, Duration::ZERO));
    link.send(Command::Catch(true));
    settle(&link, |s| s.catching);
    mock.push_motion(0, 1000, 1, 0, 0, 0);
    link.motion.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(reader.caught_up(crate::capture::clock::qpc(), Duration::from_secs(2)));
    assert!(!reader.caught_up(i64::MAX, Duration::from_millis(20)));
}

fn idle_worker(device: Option<anybox::Device>, buttons: mpsc::Sender<Feed>) -> Worker {
    let device = device.map(Arc::new);
    let (_, rx) = mpsc::sync_channel(1);
    let (motion, _) = mpsc::sync_channel(1);
    Worker {
        published: Arc::new(Mutex::new(device.clone())),
        device,
        rx,
        state: Arc::default(),
        motion,
        catch: None,
        buttons,
        button_feed: None,
        clock: Arc::default(),
        read_to: Arc::default(),
        hold: Arc::default(),
        counters: (0, 0),
        behind: (false, None),
        boxes: Box::new(crate::hardware::boxes::Plugged),
        remembered: None,
        off: false,
        seen: None,
        found: Scan::default(),
    }
}

/// a button feed that stopped by itself is opened again on the next tick, and says so
#[test]
fn a_stopped_button_feed_opens_again_on_the_next_tick() {
    let (feed, read) = mpsc::channel();
    let mut worker = idle_worker(Some(boxed(mock())), feed);
    let finished = std::thread::spawn(|| {});
    while !finished.is_finished() {
        std::thread::yield_now();
    }
    worker.button_feed = Some((Arc::new(AtomicBool::new(false)), finished));
    worker.tick();
    assert!(matches!(
        read.recv_timeout(Duration::from_secs(5)),
        Ok(Feed::Ready)
    ));
    assert!(
        worker
            .button_feed
            .as_ref()
            .is_some_and(|(_, t)| !t.is_finished())
    );
}

/// letting the box go starts its clock again: another box's host chip counts from its own boot
#[test]
fn forgetting_the_box_starts_its_clock_again() {
    let mut worker = idle_worker(Some(boxed(mock())), mpsc::channel().0);
    *worker.clock.locked() = (1234, 3);
    let generation = worker.state.locked().generation;
    worker.forget();
    assert_eq!(*worker.clock.locked(), (0, 0));
    assert_eq!(worker.state.locked().generation, generation + 1);
    assert!(worker.device.is_none());
}

/// a restart of the box's device chip is a change of the box: moves aimed before it stop
#[test]
fn a_device_chip_restart_stops_moves_aimed_before_it() {
    let mock = mock();
    let link = Link::with_device(boxed(mock.clone()));
    settle(&link, |s| s.connected && !s.kind.is_empty());
    let port = link.injection_port().unwrap();
    let generation = link.snapshot().generation;
    mock.restart();
    settle(&link, |s| s.generation > generation);
    assert_eq!(port.state(), PortState::Away("the box reconnected"));
    // once the box is back, the aim's port takes it as it is now
    settle(&link, |s| s.connected && s.has_mouse && s.error.is_none());
    port.rebase();
    assert_eq!(port.state(), PortState::Usable);
}

fn edge(button: anybox::Button, pressed: bool) -> anybox::Event {
    anybox::Event::button(
        button,
        pressed,
        anybox::Stamp::new(Instant::now(), Duration::ZERO),
    )
}

/// what the box says about its link decides whether the buttons it sends count, and anything
/// that leaves them unknown counts as none held
#[test]
fn the_route_follows_the_box_and_clears_what_it_cannot_know() {
    use anybox::{Button, DropSource, Event};
    let mut route = Route::default();
    assert_eq!(route.on(&edge(Button::Side1, true)), Step::Held(8));
    assert_eq!(
        route.on(&Event::dropped(3, DropSource::Device)),
        Step::Held(0)
    );
    route.on(&edge(Button::Left, true));
    assert!(matches!(route.on(&Event::Disconnected), Step::Lost(_)));
    // while the box is gone nothing it says about buttons or drops brings the feed back
    assert_eq!(route.on(&edge(Button::Left, true)), Step::Nothing);
    assert_eq!(
        route.on(&Event::dropped(1, DropSource::Host)),
        Step::Nothing
    );
    assert_eq!(route.on(&Event::Changed), Step::Nothing);
    assert_eq!(route.on(&Event::Reconnected), Step::Up);
    assert_eq!(route.on(&edge(Button::Right, true)), Step::Held(2));
    assert_eq!(route.on(&Event::Changed), Step::Held(0));
    route.on(&edge(Button::Middle, true));
    assert_eq!(route.on(&Event::SessionReset), Step::Held(0));
    route.on(&edge(Button::Middle, true));
    assert_eq!(route.on(&Event::Reconnected), Step::Held(0));
    assert!(matches!(
        route.on(&Event::rejected(Action::ButtonEvents, 1)),
        Step::End(_)
    ));
    assert_eq!(route.on(&Event::rejected(Action::Move, 1)), Step::Nothing);
}

fn stamped(button: anybox::Button, box_us: Option<u32>, received: Instant) -> anybox::Event {
    let mut at = anybox::Stamp::new(received, Duration::ZERO);
    if let Some(us) = box_us {
        at = at.with_box_time(anybox::BoxTime::new(us, anybox::Clock::MediusHostChip));
    }
    anybox::Event::button(button, true, at)
}

/// one report's edges fold into one held set, by the box's clock where it has one and by the read
/// otherwise; whatever comes next is handed back, never lost
#[test]
fn a_report_is_gathered_and_the_next_event_kept() {
    use anybox::{Button, DropSource, Event};
    let gathered = |queue: Vec<Event>, first: &Event| {
        let mut route = Route::default();
        let step = route.on(first);
        let Event::Button { at, .. } = first else {
            unreachable!()
        };
        let mut queue = queue.into_iter();
        gather(&mut route, at, step, || queue.next())
    };
    let now = Instant::now();
    let later = now + Duration::from_millis(1);
    let first = stamped(Button::Left, Some(10), now);
    let (step, next) = gathered(
        vec![
            stamped(Button::Side1, Some(10), later),
            stamped(Button::Right, Some(11), later),
        ],
        &first,
    );
    assert_eq!(step, Step::Held(0b1001));
    assert!(matches!(
        next,
        Some(Event::Button {
            button: Button::Right,
            ..
        })
    ));
    let (step, next) = gathered(vec![Event::dropped(1, DropSource::Host)], &first);
    assert_eq!(step, Step::Held(1));
    assert!(matches!(next, Some(Event::Dropped { .. })));
    // no box clock: the read is what says two edges are one report
    let first = stamped(Button::Left, None, now);
    let (step, next) = gathered(
        vec![
            stamped(Button::Side1, None, now),
            stamped(Button::Right, None, later),
        ],
        &first,
    );
    assert_eq!(step, Step::Held(0b1001));
    assert!(next.is_some());
    let (step, next) = gathered(vec![], &first);
    assert_eq!(step, Step::Held(1));
    assert!(next.is_none());
}

/// a move the box refused after its call returned is a move lost
#[test]
fn a_refused_move_is_a_lost_move() {
    let link = Link::with_device(boxed(mock()));
    settle(&link, |s| s.connected && s.has_mouse && !s.kind.is_empty());
    let port = link.injection_port().unwrap();
    assert_eq!(port.state(), PortState::Usable);
    link.state.locked().refused_moves += 1;
    assert_eq!(port.state(), PortState::Lost);
    port.forgive_losses();
    assert_eq!(port.state(), PortState::Usable);
}

mod picking;
