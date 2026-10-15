//! whether each control may act now, or the first thing in its way. the snapshot carries it
//! and every command asks it again before acting, so a refusal and the interface say the same

use serde::Serialize;
use std::borrow::Cow;

/// what the app knows right now, gathered by the desktop under its lock
#[derive(Clone, Debug, Default)]
pub struct Facts {
    /// a panic left the app's state half written, nothing but closing and disarming may run
    pub broken: bool,
    pub closing: bool,
    /// another action holds the recording gate
    pub busy: bool,
    pub library_loaded: bool,
    pub recording: bool,
    pub capture: Capture,
    pub selection: Selection,
    /// none when the observer is not running
    pub observers: Option<Observers>,
    pub vision: Vision,
    pub box_link: BoxLink,
    /// why the box's buttons are gone, none while their feed is up
    pub buttons: Option<String>,
    pub recording_button: RecordingButton,
    pub aim: Aim,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Capture {
    #[default]
    Stopped,
    Running,
    Failed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Selection {
    #[default]
    None,
    /// chosen, its first reports are still being checked
    Checking,
    Checked,
}

#[derive(Clone, Debug, Default)]
pub struct Observers {
    pub ready: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Vision {
    /// a worker exists, running or failed
    pub started: bool,
    pub ready: bool,
    pub error: Option<String>,
    pub scenario_named: bool,
    /// a model is set in the saved settings; whether the file exists is for the start to find
    pub model_chosen: bool,
    /// an earlier model is still being prepared in the background
    pub preparing: bool,
}

#[derive(Clone, Debug, Default)]
pub struct BoxLink {
    pub connected: bool,
    pub connecting: bool,
    /// more than one box is plugged in and none is picked yet
    pub picking: bool,
    /// the device the box clones has a mouse
    pub has_mouse: bool,
    pub pending_commands: u64,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct RecordingButton {
    pub mask: Option<u32>,
    pub learning: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Aim {
    pub armed: bool,
    pub learning: bool,
    pub error: Option<String>,
    pub mask: u32,
}

/// what is missing, the key tests and the ui can rely on whatever the words say
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Need {
    Broken,
    Closing,
    Busy,
    LibraryLoading,
    RecordingButtonPicking,
    AimButtonPicking,
    Recording,
    Armed,
    CaptureStopped,
    CaptureFailed,
    NoMouse,
    MouseUnchecked,
    ObserverFailed,
    ObserverStarting,
    TargetCaptureStopped,
    TargetCaptureFailed,
    TargetModelWarming,
    ScenarioUnnamed,
    ModelMissing,
    ModelPreparing,
    BoxConnecting,
    NoBox,
    BoxNotPicked,
    BoxNoMouse,
    BoxFailed,
    BoxBusy,
    ButtonsLost,
    SameButton,
    AimFailed,
}

impl Need {
    /// whether it passes by itself, otherwise the user has something to fix
    pub fn waits(self) -> bool {
        matches!(
            self,
            Need::Closing
                | Need::Busy
                | Need::LibraryLoading
                | Need::ObserverStarting
                | Need::TargetModelWarming
                | Need::ModelPreparing
                | Need::BoxConnecting
                | Need::BoxBusy
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Blocker {
    pub need: Need,
    pub message: Cow<'static, str>,
    pub waits: bool,
}

impl Blocker {
    fn new(need: Need, message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            need,
            message: message.into(),
            waits: need.waits(),
        }
    }
}

/// the first thing in the way of each action, none when it can go ahead. an action that toggles
/// asks for its other direction while it is on: Finish while recording, Disarm while armed, Stop
/// while target capture runs, Cancel while a button is being picked
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Readiness {
    pub record: Option<Blocker>,
    pub arm: Option<Blocker>,
    pub recording_button: Option<Blocker>,
    pub clear_recording_button: Option<Blocker>,
    pub aim_button: Option<Blocker>,
    pub box_connect: Option<Blocker>,
    pub box_settings: Option<Blocker>,
    pub screen_capture: Option<Blocker>,
}

/// the gate's refusal, the same words the snapshot shows while it is held
pub const BUSY: &str = "the last action is still finishing, wait for it";
pub const CLOSING: &str = "the app is closing";
pub const BROKEN: &str = "something inside the app broke, so restart it before doing anything else, your recordings are kept";

pub fn readiness(facts: &Facts) -> Readiness {
    Readiness {
        record: record(facts),
        arm: arm(facts),
        recording_button: recording_button(facts),
        clear_recording_button: clear_recording_button(facts),
        aim_button: aim_button(facts),
        box_connect: box_connect(facts),
        box_settings: box_settings(facts),
        screen_capture: screen_capture(facts),
    }
}

// the first unmet check returns its blocker
macro_rules! need {
    ($unmet:expr, $need:ident, $message:expr) => {
        if $unmet {
            return Some(Blocker::new(Need::$need, $message));
        }
    };
}
macro_rules! then {
    ($blocker:expr) => {
        if let blocked @ Some(_) = $blocker {
            return blocked;
        }
    };
}

fn gate(f: &Facts) -> Option<Blocker> {
    need!(f.broken, Broken, BROKEN);
    need!(f.closing, Closing, CLOSING);
    need!(f.busy, Busy, BUSY);
    None
}

fn record(f: &Facts) -> Option<Blocker> {
    then!(gate(f));
    if f.recording {
        return None;
    }
    need!(
        !f.library_loaded,
        LibraryLoading,
        "wait for the recordings library to load"
    );
    need!(
        f.recording_button.learning,
        RecordingButtonPicking,
        "finish picking the recording button first"
    );
    need!(
        f.capture == Capture::Stopped,
        CaptureStopped,
        "start monitoring to record"
    );
    need!(
        f.capture == Capture::Failed,
        CaptureFailed,
        "USB capture failed, restart monitoring to record"
    );
    need!(
        f.selection == Selection::None,
        NoMouse,
        "select a mouse to record"
    );
    need!(
        f.selection == Selection::Checking,
        MouseUnchecked,
        "move the selected mouse first, its reports are checked before recording starts"
    );
    if let Some(observers) = &f.observers {
        if let Some(error) = &observers.error {
            return Some(Blocker::new(
                Need::ObserverFailed,
                format!("the input observer failed, restart monitoring to record: {error}"),
            ));
        }
        need!(
            !observers.ready,
            ObserverStarting,
            "waiting for the input observer"
        );
    }
    // a model that is still warming up or failed only costs the recording its target events, the
    // usb evidence never waits on a GPU
    let vision = &f.vision;
    if vision.started {
        need!(
            !vision.scenario_named,
            ScenarioUnnamed,
            "name the scenario in target capture setup, or stop target capture to record mouse reports only"
        );
    }
    // a recording of the mouse alone doesn't need one behind the box
    let link = &f.box_link;
    need!(
        link.connecting,
        BoxConnecting,
        "waiting for the box to connect"
    );
    need!(
        link.pending_commands > 0,
        BoxBusy,
        "waiting for the box to answer"
    );
    need!(
        link.connected && link.error.is_some(),
        BoxFailed,
        "the box has an error, reconnect or disconnect it to record"
    );
    if f.aim.armed {
        need!(
            f.aim.error.is_some(),
            AimFailed,
            "the aim controller failed, disarm and arm it again"
        );
        then!(buttons(f));
    }
    None
}

fn arm(f: &Facts) -> Option<Blocker> {
    // disarming always stays possible, a broken app included
    if f.aim.armed {
        need!(f.closing, Closing, CLOSING);
        need!(f.busy, Busy, BUSY);
        return None;
    }
    then!(gate(f));
    need!(
        f.recording,
        Recording,
        "arm the aim before starting a recording"
    );
    need!(
        !f.library_loaded,
        LibraryLoading,
        "wait for the recordings library to load"
    );
    need!(
        f.recording_button.learning,
        RecordingButtonPicking,
        "finish picking the recording button first"
    );
    need!(
        f.aim.learning,
        AimButtonPicking,
        "finish picking the aim button first"
    );
    then!(box_ready(f, "connect a box to arm"));
    then!(buttons(f));
    need!(
        f.recording_button.mask == Some(f.aim.mask),
        SameButton,
        "the aim button and the recording button have to be different"
    );
    let vision = &f.vision;
    need!(
        !vision.started,
        TargetCaptureStopped,
        "start target capture with a named scenario to arm"
    );
    need!(
        vision.error.is_some(),
        TargetCaptureFailed,
        "restart target capture to arm"
    );
    need!(
        !vision.ready,
        TargetModelWarming,
        "the target model is warming up"
    );
    need!(
        !vision.scenario_named,
        ScenarioUnnamed,
        "name the scenario in target capture setup to arm"
    );
    None
}

fn recording_button(f: &Facts) -> Option<Blocker> {
    then!(gate(f));
    if f.recording_button.learning {
        return None;
    }
    need!(
        f.recording,
        Recording,
        "finish recording before changing its button"
    );
    let link = &f.box_link;
    need!(link.picking, BoxNotPicked, NOT_PICKED);
    need!(!link.connected, NoBox, "connect a box to pick a button");
    if !link.has_mouse {
        return Some(not_mouse_box());
    }
    need!(
        link.error.is_some(),
        BoxFailed,
        "the box has an error, reconnect it to pick a button"
    );
    then!(buttons(f));
    need!(
        f.aim.armed,
        Armed,
        "disarm the aim before changing the recording button"
    );
    need!(
        f.aim.learning,
        AimButtonPicking,
        "finish picking the aim button first"
    );
    None
}

fn clear_recording_button(f: &Facts) -> Option<Blocker> {
    then!(gate(f));
    need!(
        f.recording,
        Recording,
        "finish recording before clearing its button"
    );
    need!(
        f.aim.learning,
        AimButtonPicking,
        "finish picking the aim button first"
    );
    None
}

fn aim_button(f: &Facts) -> Option<Blocker> {
    then!(gate(f));
    if f.aim.learning {
        return None;
    }
    need!(
        f.recording,
        Recording,
        "finish recording before changing the aim button"
    );
    need!(
        !f.library_loaded,
        LibraryLoading,
        "wait for the recordings library to load"
    );
    need!(
        f.recording_button.learning,
        RecordingButtonPicking,
        "finish picking the recording button first"
    );
    then!(box_ready(f, "connect a box to pick a button"));
    then!(buttons(f));
    need!(f.aim.armed, Armed, "disarm before changing the aim button");
    None
}

fn box_connect(f: &Facts) -> Option<Blocker> {
    then!(gate(f));
    need!(
        f.recording,
        Recording,
        "finish recording before connecting, disconnecting or switching the box"
    );
    need!(
        f.aim.armed,
        Armed,
        "disarm the aim before connecting, disconnecting or switching the box"
    );
    need!(
        f.box_link.connecting,
        BoxConnecting,
        "waiting for the box to connect"
    );
    None
}

fn box_settings(f: &Facts) -> Option<Blocker> {
    then!(gate(f));
    need!(
        f.recording,
        Recording,
        "finish recording before changing the box's settings"
    );
    None
}

fn screen_capture(f: &Facts) -> Option<Blocker> {
    then!(gate(f));
    need!(
        f.recording,
        Recording,
        "finish recording before changing target capture"
    );
    if f.vision.started {
        return None;
    }
    need!(
        !f.vision.model_chosen,
        ModelMissing,
        "choose a target model to start"
    );
    need!(
        f.vision.preparing,
        ModelPreparing,
        "the last model is still being prepared, start again once it finishes"
    );
    None
}

/// a connected mouse box with its readback done, what the aim and its button drive
fn box_ready(f: &Facts, disconnected: &'static str) -> Option<Blocker> {
    let link = &f.box_link;
    need!(
        link.connecting,
        BoxConnecting,
        "waiting for the box to connect"
    );
    need!(link.picking, BoxNotPicked, NOT_PICKED);
    need!(!link.connected, NoBox, disconnected);
    if !link.has_mouse {
        return Some(not_mouse_box());
    }
    need!(
        link.error.is_some(),
        BoxFailed,
        "the box has an error, reconnect it in Capture setup"
    );
    need!(
        link.pending_commands > 0,
        BoxBusy,
        "waiting for the box to answer"
    );
    None
}

const NOT_PICKED: &str =
    "there's more than one box plugged in, so pick the one to use in Capture setup";

fn buttons(f: &Facts) -> Option<Blocker> {
    f.buttons
        .clone()
        .map(|lost| Blocker::new(Need::ButtonsLost, lost))
}

fn not_mouse_box() -> Blocker {
    Blocker::new(
        Need::BoxNoMouse,
        "there's no mouse behind the box, plug one into it",
    )
}

#[cfg(test)]
mod tests;
