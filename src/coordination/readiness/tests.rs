use super::*;
use serde_json::json;

/// monitoring a checked mouse with the observer, target capture with a named scenario, a
/// mouse box with its readback done and two different buttons: every action can go
fn ready() -> Facts {
    Facts {
        broken: false,
        closing: false,
        busy: false,
        library_loaded: true,
        recording: false,
        capture: Capture::Running,
        selection: Selection::Checked,
        observers: Some(Observers {
            ready: true,
            error: None,
        }),
        vision: Vision {
            started: true,
            ready: true,
            error: None,
            scenario_named: true,
            model_chosen: true,
            preparing: false,
        },
        box_link: BoxLink {
            connected: true,
            connecting: false,
            picking: false,
            has_mouse: true,
            pending_commands: 0,
            error: None,
        },
        buttons: None,
        recording_button: RecordingButton {
            mask: Some(8),
            learning: false,
        },
        aim: Aim {
            armed: false,
            learning: false,
            error: None,
            mask: 16,
        },
    }
}

type Unmet = fn(&mut Facts);
type Rule = (Need, Unmet);
type Action = fn(&Readiness) -> &Option<Blocker>;

fn broken(f: &mut Facts) {
    f.broken = true;
}
fn closing(f: &mut Facts) {
    f.closing = true;
}
fn busy(f: &mut Facts) {
    f.busy = true;
}
fn recording(f: &mut Facts) {
    f.recording = true;
}
fn library(f: &mut Facts) {
    f.library_loaded = false;
}
fn picking_recording(f: &mut Facts) {
    f.recording_button.learning = true;
}
fn picking_aim(f: &mut Facts) {
    f.aim.learning = true;
}
fn armed(f: &mut Facts) {
    f.aim.armed = true;
}
fn connecting(f: &mut Facts) {
    f.box_link.connecting = true;
}
fn not_picked(f: &mut Facts) {
    f.box_link.connected = false;
    f.box_link.picking = true;
}
fn disconnected(f: &mut Facts) {
    f.box_link.connected = false;
}
fn keyboard_box(f: &mut Facts) {
    f.box_link.has_mouse = false;
}
fn medius_error(f: &mut Facts) {
    f.box_link.error = Some("readback failed".into());
}
fn readback(f: &mut Facts) {
    f.box_link.pending_commands = 2;
}
fn buttons_lost(f: &mut Facts) {
    f.buttons = Some("the box stopped sending its buttons".into());
}
fn vision_stopped(f: &mut Facts) {
    f.vision.started = false;
}
fn vision_failed(f: &mut Facts) {
    f.vision.error = Some("model lost".into());
}
fn warming(f: &mut Facts) {
    f.vision.ready = false;
}
fn unnamed(f: &mut Facts) {
    f.vision.scenario_named = false;
}

// each action's checks in the order they apply
const RECORD: &[Rule] = &[
    (Need::Broken, broken),
    (Need::Closing, closing),
    (Need::Busy, busy),
    (Need::LibraryLoading, library),
    (Need::RecordingButtonPicking, picking_recording),
    (Need::CaptureStopped, |f| f.capture = Capture::Stopped),
    (Need::CaptureFailed, |f| f.capture = Capture::Failed),
    (Need::NoMouse, |f| f.selection = Selection::None),
    (Need::MouseUnchecked, |f| f.selection = Selection::Checking),
    (Need::ObserverFailed, |f| {
        f.observers = Some(Observers {
            ready: false,
            error: Some("observer lost".into()),
        })
    }),
    (Need::ObserverStarting, |f| {
        f.observers = Some(Observers {
            ready: false,
            error: None,
        })
    }),
    (Need::ScenarioUnnamed, unnamed),
    (Need::BoxConnecting, connecting),
    (Need::BoxBusy, readback),
    (Need::BoxFailed, medius_error),
    (Need::AimFailed, |f| {
        f.aim.armed = true;
        f.aim.error = Some("Medius stopped answering".into());
    }),
    (Need::ButtonsLost, |f| {
        f.aim.armed = true;
        buttons_lost(f);
    }),
];

const ARM: &[Rule] = &[
    (Need::Broken, broken),
    (Need::Closing, closing),
    (Need::Busy, busy),
    (Need::Recording, recording),
    (Need::LibraryLoading, library),
    (Need::RecordingButtonPicking, picking_recording),
    (Need::AimButtonPicking, picking_aim),
    (Need::BoxConnecting, connecting),
    (Need::BoxNotPicked, not_picked),
    (Need::NoBox, disconnected),
    (Need::BoxNoMouse, keyboard_box),
    (Need::BoxFailed, medius_error),
    (Need::BoxBusy, readback),
    (Need::ButtonsLost, buttons_lost),
    (Need::SameButton, |f| {
        f.recording_button.mask = Some(f.aim.mask)
    }),
    (Need::TargetCaptureStopped, vision_stopped),
    (Need::TargetCaptureFailed, vision_failed),
    (Need::TargetModelWarming, warming),
    (Need::ScenarioUnnamed, unnamed),
];

const RECORDING_BUTTON: &[Rule] = &[
    (Need::Broken, broken),
    (Need::Closing, closing),
    (Need::Busy, busy),
    (Need::Recording, recording),
    (Need::BoxNotPicked, not_picked),
    (Need::NoBox, disconnected),
    (Need::BoxNoMouse, keyboard_box),
    (Need::BoxFailed, medius_error),
    (Need::ButtonsLost, buttons_lost),
    (Need::Armed, armed),
    (Need::AimButtonPicking, picking_aim),
];

const CLEAR_RECORDING_BUTTON: &[Rule] = &[
    (Need::Broken, broken),
    (Need::Closing, closing),
    (Need::Busy, busy),
    (Need::Recording, recording),
    (Need::AimButtonPicking, picking_aim),
];

const AIM_BUTTON: &[Rule] = &[
    (Need::Broken, broken),
    (Need::Closing, closing),
    (Need::Busy, busy),
    (Need::Recording, recording),
    (Need::LibraryLoading, library),
    (Need::RecordingButtonPicking, picking_recording),
    (Need::BoxConnecting, connecting),
    (Need::BoxNotPicked, not_picked),
    (Need::NoBox, disconnected),
    (Need::BoxNoMouse, keyboard_box),
    (Need::BoxFailed, medius_error),
    (Need::BoxBusy, readback),
    (Need::ButtonsLost, buttons_lost),
    (Need::Armed, armed),
];

const FIRMWARE_CONNECT: &[Rule] = &[
    (Need::Broken, broken),
    (Need::Closing, closing),
    (Need::Busy, busy),
    (Need::Recording, recording),
    (Need::Armed, armed),
    (Need::BoxConnecting, connecting),
];

const FIRMWARE_SETTINGS: &[Rule] = &[
    (Need::Broken, broken),
    (Need::Closing, closing),
    (Need::Busy, busy),
    (Need::Recording, recording),
];

const SCREEN_CAPTURE: &[Rule] = &[
    (Need::Broken, broken),
    (Need::Closing, closing),
    (Need::Busy, busy),
    (Need::Recording, recording),
    (Need::ModelMissing, |f| f.vision.model_chosen = false),
    (Need::ModelPreparing, |f| f.vision.preparing = true),
];

/// target capture's start, the one action that starts from off
fn stopped() -> Facts {
    let mut facts = ready();
    facts.vision.started = false;
    facts
}

/// each action, its rules and the facts it starts from with nothing in its way
type Row = (&'static str, &'static [Rule], Action, fn() -> Facts);

fn actions() -> [Row; 8] {
    [
        ("record", RECORD, |r| &r.record, ready),
        ("arm", ARM, |r| &r.arm, ready),
        (
            "recording_button",
            RECORDING_BUTTON,
            |r| &r.recording_button,
            ready,
        ),
        (
            "clear_recording_button",
            CLEAR_RECORDING_BUTTON,
            |r| &r.clear_recording_button,
            ready,
        ),
        ("aim_button", AIM_BUTTON, |r| &r.aim_button, ready),
        ("box_connect", FIRMWARE_CONNECT, |r| &r.box_connect, ready),
        (
            "box_settings",
            FIRMWARE_SETTINGS,
            |r| &r.box_settings,
            ready,
        ),
        (
            "screen_capture",
            SCREEN_CAPTURE,
            |r| &r.screen_capture,
            stopped,
        ),
    ]
}

#[test]
fn a_target_model_warming_up_or_failed_never_holds_a_recording_back() {
    for unready in [warming, vision_failed] {
        let mut facts = ready();
        unready(&mut facts);
        assert_eq!(readiness(&facts).record, None);
    }
}

#[test]
fn every_action_goes_ahead_with_nothing_in_its_way() {
    for (name, _, action, start) in actions() {
        assert_eq!(action(&readiness(&start())), &None, "{name}");
    }
}

/// each rule wins over every rule after it; they are broken from the last one back, so a rule
/// sharing a field with a later one still sees its own value
#[test]
fn the_first_unmet_rule_wins() {
    for (name, rules, action, start) in actions() {
        for (index, (need, _)) in rules.iter().enumerate() {
            let mut facts = start();
            for (_, unmet) in rules[index..].iter().rev() {
                unmet(&mut facts);
            }
            let blocker = action(&readiness(&facts))
                .clone()
                .unwrap_or_else(|| panic!("{name}: {need:?} let it go ahead"));
            assert_eq!(blocker.need, *need, "{name}");
            assert_eq!(blocker.waits, need.waits(), "{name}: {need:?}");
            assert!(!blocker.message.is_empty());
        }
    }
}

/// an action that is on asks only to turn off: only closing and the gate stop that
#[test]
fn the_other_direction_needs_only_the_gate() {
    let toggles: [(&str, Unmet, Action); 5] = [
        ("finish recording", recording, |r| &r.record),
        ("disarm", armed, |r| &r.arm),
        ("cancel the recording button", picking_recording, |r| {
            &r.recording_button
        }),
        ("cancel the aim button", picking_aim, |r| &r.aim_button),
        ("stop target capture", |_| {}, |r| &r.screen_capture),
    ];
    for (name, on, action) in toggles {
        // everything else that could be in the way is
        let mut facts = Facts {
            library_loaded: false,
            capture: Capture::Failed,
            observers: Some(Observers {
                ready: false,
                error: Some("observer lost".into()),
            }),
            buttons: Some("gone".into()),
            ..Facts::default()
        };
        facts.vision.started = true;
        facts.vision.error = Some("model lost".into());
        facts.box_link.connecting = true;
        on(&mut facts);
        assert_eq!(action(&readiness(&facts)), &None, "{name}");
        facts.busy = true;
        let blocker = action(&readiness(&facts)).clone().unwrap();
        assert_eq!(blocker.need, Need::Busy, "{name}");
        assert_eq!(blocker.message, BUSY);
        facts.closing = true;
        assert_eq!(
            action(&readiness(&facts)).as_ref().unwrap().need,
            Need::Closing,
            "{name}"
        );
    }
}

#[test]
fn messages_carry_what_they_name() {
    let mut facts = ready();
    facts.observers.as_mut().unwrap().error = Some("observer lost".into());
    let record = readiness(&facts).record.unwrap();
    assert!(record.message.ends_with(": observer lost"));
    let mut facts = ready();
    keyboard_box(&mut facts);
    assert!(
        readiness(&facts)
            .arm
            .unwrap()
            .message
            .contains("no mouse behind the box")
    );
    let mut facts = ready();
    buttons_lost(&mut facts);
    assert_eq!(
        readiness(&facts).recording_button.unwrap().message,
        "the box stopped sending its buttons"
    );
}

// whether a need passes by itself, pinned so the wait-or-fix wording never flips unnoticed
const WAITS: [(Need, &str, bool); 28] = [
    (Need::Closing, "closing", true),
    (Need::Busy, "busy", true),
    (Need::LibraryLoading, "library_loading", true),
    (
        Need::RecordingButtonPicking,
        "recording_button_picking",
        false,
    ),
    (Need::AimButtonPicking, "aim_button_picking", false),
    (Need::Recording, "recording", false),
    (Need::Armed, "armed", false),
    (Need::CaptureStopped, "capture_stopped", false),
    (Need::CaptureFailed, "capture_failed", false),
    (Need::NoMouse, "no_mouse", false),
    (Need::MouseUnchecked, "mouse_unchecked", false),
    (Need::ObserverFailed, "observer_failed", false),
    (Need::ObserverStarting, "observer_starting", true),
    (Need::TargetCaptureStopped, "target_capture_stopped", false),
    (Need::TargetCaptureFailed, "target_capture_failed", false),
    (Need::TargetModelWarming, "target_model_warming", true),
    (Need::ScenarioUnnamed, "scenario_unnamed", false),
    (Need::ModelMissing, "model_missing", false),
    (Need::ModelPreparing, "model_preparing", true),
    (Need::BoxConnecting, "box_connecting", true),
    (Need::NoBox, "no_box", false),
    (Need::BoxNotPicked, "box_not_picked", false),
    (Need::BoxNoMouse, "box_no_mouse", false),
    (Need::BoxFailed, "box_failed", false),
    (Need::BoxBusy, "box_busy", true),
    (Need::ButtonsLost, "buttons_lost", false),
    (Need::SameButton, "same_button", false),
    (Need::AimFailed, "aim_failed", false),
];

#[test]
fn each_need_waits_or_asks_for_a_fix() {
    // the ui names the codes once, in its `Need` type
    let ui = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/ui/types.ts")).unwrap();
    for (need, code, waits) in WAITS {
        assert_eq!(need.waits(), waits, "{need:?}");
        assert_eq!(serde_json::to_value(need).unwrap(), code);
        assert!(
            ui.contains(&format!("| \"{code}\"")),
            "ui/types.ts lacks {code}"
        );
    }
}

/// what the ui reads: one key per action, null or the need, its words and whether it waits
#[test]
fn the_snapshot_shape() {
    let mut facts = ready();
    facts.vision.started = false;
    facts.box_link.pending_commands = 1;
    let value = serde_json::to_value(readiness(&facts)).unwrap();
    assert_eq!(
        value,
        json!({
            "record": {
                "need": "box_busy",
                "message": "waiting for the box to answer",
                "waits": true,
            },
            "arm": {
                "need": "box_busy",
                "message": "waiting for the box to answer",
                "waits": true,
            },
            "recording_button": null,
            "clear_recording_button": null,
            "aim_button": {
                "need": "box_busy",
                "message": "waiting for the box to answer",
                "waits": true,
            },
            "box_connect": null,
            "box_settings": null,
            "screen_capture": null,
        })
    );
}

/// an optional part that is off never blocks, whatever it last said
#[test]
fn a_part_that_is_off_is_not_in_the_way() {
    let mut facts = ready();
    facts.box_link = BoxLink {
        connected: false,
        error: Some("the port went away".into()),
        ..BoxLink::default()
    };
    facts.vision = Vision {
        started: false,
        ready: false,
        scenario_named: false,
        ..Vision::default()
    };
    facts.observers = None;
    facts.aim.error = Some("Medius stopped answering".into());
    let decided = readiness(&facts);
    assert_eq!(decided.record, None);
    assert_eq!(decided.box_connect, None);
    assert_eq!(decided.box_settings, None);
    assert_eq!(decided.clear_recording_button, None);
}

/// target capture that runs during a recording cannot be stopped either, Stop is no way round
#[test]
fn a_recording_keeps_running_target_capture_too() {
    let mut facts = ready();
    facts.recording = true;
    assert_eq!(
        readiness(&facts).screen_capture.unwrap().need,
        Need::Recording
    );
}

#[test]
fn a_broken_app_refuses_every_action_but_disarming() {
    let mut f = ready();
    f.broken = true;
    let all = readiness(&f);
    for blocker in [
        all.record,
        all.arm,
        all.recording_button,
        all.clear_recording_button,
        all.aim_button,
        all.box_connect,
        all.box_settings,
        all.screen_capture,
    ] {
        assert_eq!(blocker.map(|b| b.need), Some(Need::Broken));
    }
    f.aim.armed = true;
    assert_eq!(readiness(&f).arm, None);
}

/// the mouse alone records with a box connected that has none behind it, which still can't pick
/// a button or arm
#[test]
fn a_box_with_no_mouse_leaves_recording_the_mouse_alone() {
    let mut facts = ready();
    keyboard_box(&mut facts);
    let decided = readiness(&facts);
    assert_eq!(decided.record, None);
    assert!(decided.recording_button.is_some());
    assert!(decided.arm.is_some());
}
