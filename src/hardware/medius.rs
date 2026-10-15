//! Medius firmware readback runs on a worker because queries can block for a second.
//! injection and CATCH workers share the SDK transport; the UI reads confirmed snapshots

use crate::sync::LockExt;
// the crate's own re-export, so the app and anybox always speak to one version of it
use crate::hardware::boxes::{Boxes, Listed, Opened, SavedBox, Scan, Unusable};
use anybox::medius;
pub use anybox::medius as api;
use anybox::medius::Device;
pub use anybox::medius::{EmitPace, RenderMode};
use anybox::{Action, Event, EventKinds, Events};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub enum Command {
    /// the app started: the remembered box if it's there, otherwise the only one plugged in
    Start(Option<SavedBox>),
    Connect,
    Disconnect,
    /// switch to the listed box with this key and remember it
    Pick(String),
    /// look for boxes again without connecting to any
    Refresh,

    Render(RenderMode, bool),
    Emit(EmitPace, Option<u16>),
    Spread(u16),
    Riding(Option<Duration>),
    Imperfect(bool),
    /// stream every report the real mouse makes, as the box's host chip sees it
    Catch(bool),
}

/// the box's buttons, the one thing every box this app may drive gives: the feed comes up, a
/// change of the held buttons, or the feed is lost. a lost feed's held buttons are unknown
pub enum Feed {
    Ready,
    Buttons {
        mask: u32,
        at: crate::capture::clock::Anchor,
    },
    Lost(String),
}

/// one real mouse report as the box saw it on usb3, before anything the box does to it
pub struct Motion {
    pub arrived_us: u64,
    pub arrived_qpc: i64,
    /// the host chip's microseconds since it booted, unwrapped past the 32 bit rollover
    pub box_us: u64,
    pub dx: i16,
    pub dy: i16,
    pub dz: i16,
}

/// plain fields copied out of the box's replies, so the ui never holds a library type it has to
/// lock a thread for
#[derive(Clone, Default)]
pub struct Snapshot {
    pub pending_commands: u64,
    pub input_drops: u64,
    pub link_drops: u32,
    pub host_drops: u32,
    pub tx_drops: u16,
    pub session: u16,
    pub resets: u16,
    /// the box's input stream started or stopped, or the box changed: input rows on either
    /// side of it are not one stream
    pub discontinuities: u64,
    /// the box itself changed: a connection, a disconnection, a session or reset the box
    /// reports, or a restart or reconnect of its link. moves aimed before it must not go out
    pub generation: u64,
    /// the serial port the box answered on, such as COM5
    pub port: Option<String>,
    pub connected: bool,
    pub connecting: bool,
    /// more than one usable box is plugged in and none is picked yet
    pub picking: bool,
    pub error: Option<String>,
    /// which kind of box it is, as anybox names it, such as Medius or MAKCU V4
    pub family: String,
    pub name: String,
    pub firmware: String,
    pub fw: (u8, u8, u8),
    pub protocol: u8,
    /// the host chip runs its own image and can differ from the device chip's
    pub host_firmware: String,
    pub host_fw: Option<(u8, u8, u8)>,
    pub mac: String,
    pub clone: String,
    /// the cloned device's own ids and name, as the box reports them, so the endpoint the box
    /// clones can be told apart from the others on the wire
    pub clone_vid: Option<u16>,
    pub clone_pid: Option<u16>,
    pub clone_product: String,
    pub kind: String,
    /// the device the box clones has a mouse, whatever kind the box files it under: a mouse
    /// with a full keyboard on it reads as a keyboard
    pub has_mouse: bool,
    /// the cloned mouse puts a report id first, so the wire's bytes 1..5 are not x and y. the
    /// box's own dx and dy are decoded properly whatever the layout
    pub has_report_id: Option<bool>,
    /// what the box can do besides moving, as it said when last read
    pub can_buttons: bool,
    pub can_motion: bool,
    pub link_up: bool,
    pub mouse_attached: bool,
    pub clone_configured: bool,
    pub native_period_us: u16,
    pub poll_period_us: u16,
    pub rate_confident: bool,
    pub render_mode: Option<RenderMode>,
    pub render_full: bool,
    pub render_ready: bool,
    pub emit_mode: Option<EmitPace>,
    pub resolved_hz: u16,
    pub force_hz: Option<u16>,
    pub advertised_hz: u16,
    pub force_active: bool,
    pub spread_percent: u16,
    pub spread_span_us: u32,
    pub riding: Option<Option<Duration>>,
    pub imperfect_allowed: bool,
    pub over_capacity: bool,
    pub clone_imperfect: bool,
    pub catching: bool,
    pub catch_dropped: u64,
    /// moves the box refused after the call that sent them returned, as a MAKCU does when its
    /// queue is full
    pub refused_moves: u64,
    /// the boxes the last scan saw, the connected one among them
    pub boxes: Vec<Listed>,
    /// the ports the last scan couldn't use, and why
    pub unusable: Vec<Unusable>,
    /// which of them is connected
    pub key: Option<String>,
    /// the box to take at the next start, for the desktop to save
    pub remember: Option<SavedBox>,
    /// what happened that isn't an error: the remembered box wasn't there and another was taken
    pub notice: Option<String>,
}

impl Snapshot {
    /// the box's state as one json object for the recording sidecar, so a run says which mode it
    /// was made under
    pub fn json(&self) -> serde_json::Value {
        let render = match self.render_mode {
            Some(RenderMode::Off) => "off",
            Some(RenderMode::Stock) => "stock",
            Some(RenderMode::Despiked) => "despiked",
            Some(RenderMode::Unsmoothed) => "unsmoothed",
            None => "unknown",
        };
        let (emit, fixed) = match self.emit_mode {
            Some(EmitPace::Learned) => ("learned", None),
            Some(EmitPace::Interval) => ("interval", None),
            Some(EmitPace::Fixed(hz)) => ("fixed", Some(hz)),
            None => ("unknown", None),
        };
        let riding = match self.riding {
            Some(Some(window)) => Some(window.as_millis() as u64),
            _ => None,
        };
        let host = self.host_fw;
        // effective sits beside the fields it is derived from
        serde_json::json!({
            "name": self.name, "port": self.port, "mac": self.mac, "clone": self.clone, "kind": self.kind, "has_mouse": self.has_mouse,
            "report_id": self.has_report_id,
            "firmware": {
                "device_major": self.fw.0, "device_minor": self.fw.1, "device_patch": self.fw.2,
                "host_major": host.map(|h| h.0), "host_minor": host.map(|h| h.1),
                "host_patch": host.map(|h| h.2), "protocol": self.protocol,
            },
            "native_period_us": self.native_period_us, "rate_confident": self.rate_confident,
            "render": {
                "mode": render, "full": self.render_full, "ready": self.render_ready,
                "effective": self.effective(),
            },
            "emit": {
                "mode": emit, "fixed_hz": fixed, "resolved_hz": self.resolved_hz,
                "advertised_hz": self.advertised_hz, "force_hz": self.force_hz,
                "force_active": self.force_active,
            },
            "spread": { "percent": self.spread_percent, "span_us": self.spread_span_us },
            "riding_ms": riding, "imperfect_allowed": self.imperfect_allowed,
            "clone_imperfect": self.clone_imperfect,
        })
    }

    /// the box as a recording keeps it: what it is and could do when the recording started, and
    /// the Medius settings only on a Medius
    pub fn sidecar(&self) -> serde_json::Value {
        let ferrum_api = self
            .boxes
            .iter()
            .find(|row| Some(&row.key) == self.key.as_ref())
            .and_then(|row| row.ferrum_api)
            .map(|api| format!("{api:?}"));
        serde_json::json!({
            "family": self.family, "name": self.name, "port": self.port, "firmware": self.firmware,
            "key": self.key, "ferrum_api": ferrum_api,
            "capabilities": { "buttons": self.can_buttons, "move": self.has_mouse, "motion": self.can_motion },
            "input": {
                "requested": self.catching,
                "timing": self.catching.then_some("medius host chip"),
            },
            "medius": (self.family == "Medius").then(|| self.json()),
        })
    }

    /// the box as the window shows it, the same for every box, with the Medius part only on a
    /// Medius
    pub fn view(&self) -> serde_json::Value {
        let link = if self.connecting {
            "connecting"
        } else if self.connected {
            "connected"
        } else {
            "none"
        };
        let medius = (self.family == "Medius").then(|| {
            let mut medius = self.json();
            medius["health"] = serde_json::json!({
                "link_drops": self.link_drops, "host_drops": self.host_drops, "tx_drops": self.tx_drops,
                "session": self.session, "resets": self.resets,
            });
            medius
        });
        serde_json::json!({
            "link": link, "error": self.error, "pending_commands": self.pending_commands,
            "family": self.family, "name": self.name, "port": self.port, "firmware": self.firmware,
            "has_mouse": self.has_mouse,
            "boxes": self.boxes.iter().map(|row| serde_json::json!({
                "key": row.key, "label": row.label(), "model": row.model, "usable": row.usable,
            })).collect::<Vec<_>>(),
            "unusable": self.unusable.iter().map(|row| serde_json::json!({"port": row.port, "why": row.why})).collect::<Vec<_>>(),
            "key": self.key, "remembered": self.remember.as_ref().map(|saved| &saved.key),
            "picking": self.picking, "notice": self.notice,
            "features": { "physical_input": self.why_no_physical_input() },
            "watch": self.catching,
            "losses": self.input_drops + self.catch_dropped,
            "medius": medius,
        })
    }

    /// why a recording can't hold the mouse's own movement through this box, none when it can
    fn why_no_physical_input(&self) -> Option<&'static str> {
        if self.family != "Medius" {
            Some(
                "only a Medius stamps your mouse's movement with its own clock, and that's what lines it up with the USB reports",
            )
        } else if !self.has_mouse {
            Some("there's no mouse behind the box right now")
        } else if !self.can_motion {
            Some("the box isn't giving your mouse's movement right now")
        } else {
            None
        }
    }

    /// what the box is actually doing to the hand's motion right now. nothing is rendered until
    /// a profile has armed, whatever mode says, and the profile is never persisted, so a run
    /// labelled by its mode alone can be a relay recording wearing the wrong name
    pub fn effective(&self) -> &'static str {
        let rendering = self.render_full
            && self.render_ready
            && !matches!(self.render_mode, Some(RenderMode::Off) | None);
        if rendering { "rendered" } else { "relay" }
    }
}

pub struct Link {
    tx: SyncSender<Command>,
    state: Arc<Mutex<Snapshot>>,
    pub motion: Receiver<Motion>,
    /// taken once by whoever routes the buttons
    buttons: Mutex<Option<Receiver<Feed>>>,
    /// the box, shared with the aim's port
    device: Arc<Mutex<Option<Arc<anybox::Device>>>>,
    read_to: Arc<AtomicI64>,
    hold: Arc<AtomicBool>,
}

/// a recording that ends waits on this, so input and losses already queued at its end are in it
/// rather than counted after it was saved
pub struct InputReader {
    read_to: Arc<AtomicI64>,
    state: Arc<Mutex<Snapshot>>,
}

impl InputReader {
    /// whether what was queued at `qpc` was read within `within`. no stream has nothing to wait
    /// for
    pub fn caught_up(&self, qpc: i64, within: Duration) -> bool {
        let until = std::time::Instant::now() + within;
        loop {
            if !self.state.locked().catching || self.read_to.load(Ordering::Acquire) >= qpc {
                return true;
            }
            if std::time::Instant::now() >= until {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

/// whether the aim's moves can go to the box right now
#[derive(Debug, PartialEq)]
pub enum PortState {
    Usable,
    /// gone for now, and it can come back
    Away(&'static str),
    /// some moves never made it, which only matters to a recording that says what went out
    Lost,
    /// the box can't be written to any more, so the aim has to end
    Broken(String),
}

#[derive(Clone)]
pub struct InjectionPort {
    device: Arc<anybox::Device>,
    /// the box the link holds now, which stops being this one once it's let go of
    current: Arc<Mutex<Option<Arc<anybox::Device>>>>,
    state: Arc<Mutex<Snapshot>>,
    /// the generation and the loss count the aim's moves are good for, shared by every clone so
    /// a comeback moves them all on
    baseline: Arc<Mutex<(u64, u64)>>,
}
impl InjectionPort {
    pub fn state(&self) -> PortState {
        let held = self
            .current
            .locked()
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &self.device));
        if !held {
            return PortState::Broken("the box was let go of, arm again".into());
        }
        let state = self.state.locked();
        let (generation, losses) = *self.baseline.locked();
        if !state.connected {
            PortState::Away("the box isn't connected")
        } else if !state.has_mouse {
            PortState::Away("there's no mouse behind the box")
        } else if state.error.is_some() {
            PortState::Away("the box isn't answering")
        } else if state.generation != generation {
            PortState::Away("the box reconnected")
        } else if injection_losses(&state) > losses {
            PortState::Lost
        } else {
            PortState::Usable
        }
    }
    /// back: the same box, connected and answering with a mouse behind it, whatever happened to
    /// it while it was gone
    pub fn back(&self) -> bool {
        let held = self
            .current
            .locked()
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &self.device));
        let state = self.state.locked();
        held && state.connected && state.has_mouse && state.error.is_none()
    }
    /// the moves lost so far are let go of, the generation stays what it was
    pub fn forgive_losses(&self) {
        let losses = injection_losses(&self.state.locked());
        self.baseline.locked().1 = losses;
    }
    /// the box as it is now is what the aim's moves are good for
    pub fn rebase(&self) {
        let state = self.state.locked();
        *self.baseline.locked() = (state.generation, injection_losses(&state));
    }
    fn usable(&self) -> Result<(), PortState> {
        match self.state() {
            PortState::Usable => Ok(()),
            other => Err(other),
        }
    }
    pub fn move_relative(&self, dx: i16, dy: i16) -> Result<(), PortState> {
        self.usable()?;
        self.device.move_by(dx, dy).map_err(|e| self.failed(e))
    }
    pub fn discard_pending(&self) -> Result<(), PortState> {
        self.usable()?;
        self.discard_unchecked()
    }
    /// drops what the box still holds of earlier moves. only a Medius holds any: it paces a
    /// move to the mouse's report rate, splits it over reports and spreads it over the command
    /// interval, and with riding on waits for a physical move. the others send a move as it comes
    pub fn discard_unchecked(&self) -> Result<(), PortState> {
        let holds_moves = self
            .device
            .snapshot()
            .capabilities
            .get(Action::DiscardPendingMoves)
            .is_supported();
        if !holds_moves {
            return Ok(());
        }
        self.device
            .discard_pending_moves()
            .map_err(|e| self.failed(e))
    }

    /// a box that went during the call, or whose mouse went, can come back; anything else can't
    fn failed(&self, error: anybox::Error) -> PortState {
        let gone = match &error {
            anybox::Error::Disconnected => true,
            anybox::Error::Unsupported { action, .. } => !self
                .device
                .snapshot()
                .capabilities
                .get(*action)
                .is_supported(),
            _ => false,
        };
        if gone {
            PortState::Away("the box went away")
        } else {
            PortState::Broken(error.to_string())
        }
    }
}

impl Link {
    pub fn spawn() -> Self {
        Self::start(None, Box::new(crate::hardware::boxes::Plugged))
    }

    /// a device that is already open, which is how the tests hand in a mock box
    pub fn with_device(device: anybox::Device) -> Self {
        Self::start(Some(device), Box::new(crate::hardware::boxes::Plugged))
    }

    /// boxes found and opened through `boxes`, how the tests pick among mock boxes
    pub fn with_boxes(boxes: Box<dyn Boxes>) -> Self {
        Self::start(None, boxes)
    }

    fn start(device: Option<anybox::Device>, boxes: Box<dyn Boxes>) -> Self {
        let device = device.map(Arc::new);
        let (tx, rx) = mpsc::sync_channel(64);
        let (motion_tx, motion) = mpsc::sync_channel(32_768);
        // button changes come at the pace of a hand, and a full queue would lose a release
        let (buttons_tx, buttons) = mpsc::channel();
        let state = Arc::new(Mutex::new(Snapshot::default()));
        let shared = Arc::clone(&state);
        let published = Arc::new(Mutex::new(device.clone()));
        let port = published.clone();
        let read_to = Arc::new(AtomicI64::new(0));
        let reading = read_to.clone();
        let hold = Arc::new(AtomicBool::new(false));
        let holding = hold.clone();
        std::thread::spawn(move || {
            Worker {
                device,
                rx,
                state: shared,
                motion: motion_tx,
                catch: None,
                buttons: buttons_tx,
                button_feed: None,
                clock: Arc::default(),
                read_to: reading,
                hold: holding,
                counters: (0, 0),
                published: port,
                behind: (false, None),
                boxes,
                remembered: None,
                off: false,
                seen: None,
                found: Scan::default(),
            }
            .run()
        });
        Link {
            tx,
            state,
            motion,
            buttons: Mutex::new(Some(buttons)),
            device: published,
            read_to,
            hold,
        }
    }

    /// while on, the box in use stays the one, even when the remembered one comes back
    pub fn hold_still(&self, on: bool) {
        self.hold.store(on, Ordering::Relaxed);
    }

    /// what tells when the box's input has been read up to a moment
    pub fn input_reader(&self) -> InputReader {
        InputReader {
            read_to: self.read_to.clone(),
            state: self.state.clone(),
        }
    }

    /// the button feed, for the one thread that routes it
    pub fn take_buttons(&self) -> Option<Receiver<Feed>> {
        self.buttons.locked().take()
    }

    pub fn send(&self, command: Command) {
        self.state.locked().pending_commands += 1;
        if let Err(error) = self.tx.try_send(command) {
            let mut state = self.state.locked();
            state.pending_commands = state.pending_commands.saturating_sub(1);
            state.error = Some(format!("Medius command queue: {error}"));
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.state.locked().clone()
    }
    pub fn injection_port(&self) -> Result<InjectionPort, String> {
        let state = self.snapshot();
        if !state.connected
            || !state.has_mouse
            || state.error.is_some()
            || state.pending_commands > 0
        {
            return Err("connect a box with a mouse behind it and wait for its readback".into());
        }
        Ok(InjectionPort {
            device: self
                .device
                .locked()
                .clone()
                .ok_or("the box isn't connected")?,
            current: self.device.clone(),
            state: self.state.clone(),
            baseline: Arc::new(Mutex::new((state.generation, injection_losses(&state)))),
        })
    }
}

/// losses on the box's serial link; the input stream's own losses say nothing about moves
fn injection_losses(state: &Snapshot) -> u64 {
    state.link_drops as u64 + state.host_drops as u64 + state.tx_drops as u64 + state.refused_moves
}

struct Worker {
    device: Option<Arc<anybox::Device>>,
    rx: Receiver<Command>,
    state: Arc<Mutex<Snapshot>>,
    motion: SyncSender<Motion>,
    /// the stop flag of the catch thread, which owns the stream and unsubscribes by dropping it
    catch: Option<(Arc<AtomicBool>, std::thread::JoinHandle<()>)>,
    buttons: mpsc::Sender<Feed>,
    /// the button feed's thread, the same way; a finished one is opened again on the next tick
    button_feed: Option<(Arc<AtomicBool>, std::thread::JoinHandle<()>)>,
    /// the host chip clock's last stamp and rollovers, kept across input stream restarts so a
    /// stream started again goes on in the same clock; a new device starts it again
    clock: Arc<Mutex<(u32, u64)>>,
    /// the qpc up to which the box's input has been read out of anybox
    read_to: Arc<AtomicI64>,
    /// a recording or the aim is using the box, so it isn't swapped for another by itself
    hold: Arc<AtomicBool>,
    /// the link's restarts and reconnects as last seen
    counters: (u64, u64),
    published: Arc<Mutex<Option<Arc<anybox::Device>>>>,
    /// whether a mouse was behind the box and what it presented, when last read
    behind: (bool, Option<(u16, u16)>),
    boxes: Box<dyn Boxes>,
    remembered: Option<SavedBox>,
    /// let go of on purpose: nothing connects by itself until Connect or a pick
    off: bool,
    /// the ports a search would look at when last asked, none before the first search
    seen: Option<Vec<String>>,
    /// what the last scan saw
    found: Scan,
}

impl Worker {
    fn run(mut self) {
        if let Some(row) = self.device.as_deref().map(Listed::of) {
            self.update(|s| {
                s.connected = true;
                s.key = Some(row.key.clone());
                s.boxes = vec![row];
            });
            self.identify();
            self.refresh();
            self.buttons(true);
        }
        loop {
            let message = self.rx.recv_timeout(Duration::from_secs(1));
            let is_command = message.is_ok();
            match message {
                Ok(Command::Start(remembered)) => {
                    self.remembered = remembered;
                    let saved = self.remembered.clone();
                    self.update(|s| s.remember = saved);
                    self.settle();
                    self.disable_failed_startup();
                }
                Ok(Command::Connect) => {
                    self.off = false;
                    self.settle();
                }
                Ok(Command::Disconnect) => {
                    self.forget();
                    self.off = true;
                }
                Ok(Command::Pick(key)) => self.pick(&key),
                Ok(Command::Refresh) => self.list(Some(self.boxes.scan())),

                Ok(Command::Catch(on)) => self.catch(on),
                Ok(Command::Render(mode, full)) => self.apply(|d| d.set_render(mode, full)),
                Ok(Command::Emit(pace, force)) => self.apply(|d| d.set_emit_pace(pace, force)),
                Ok(Command::Spread(percent)) => self.apply(|d| d.set_spread(percent)),
                Ok(Command::Riding(window)) => self.apply(|d| d.set_movement_riding(window)),
                Ok(Command::Imperfect(allow)) => self.apply(|d| d.allow_imperfect_clones(allow)),
                Err(RecvTimeoutError::Timeout) => self.tick(),
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if is_command {
                self.update(|s| s.pending_commands = s.pending_commands.saturating_sub(1));
            }
        }
    }

    /// once a second: the box's state again, and a button feed that stopped by itself opened
    /// again
    fn tick(&mut self) {
        self.watch_for_boxes();
        self.describe();
        self.refresh();
        if self.device.is_some()
            && self
                .button_feed
                .as_ref()
                .is_none_or(|(_, thread)| thread.is_finished())
        {
            self.buttons(true);
        }
    }

    /// the box is let go: its streams stop, its clock and state with it, and the port is released
    /// on a thread of its own, which can wait a couple of seconds for a box that stopped answering
    fn forget(&mut self) {
        *self.published.locked() = None;
        self.catch(false);
        self.buttons(false);
        let device = self.device.take();
        *self.clock.locked() = (0, 0);
        self.update(|s| *s = fresh(s));
        // the state is already gone for the window, so the wait for the box is only the worker's,
        // and it is bounded inside anybox. a Connect right after this must not find a busy port
        if let Some(device) = device
            && let Err(error) = device.close()
        {
            self.update(|s| {
                s.error = Some(format!("the box did not let go of what it held: {error}"))
            });
        }
    }

    fn update(&self, change: impl FnOnce(&mut Snapshot)) {
        change(&mut self.state.locked());
    }

    /// a box just opened becomes the one: the link's counters and clock start from it, and its
    /// identity, settings and buttons are read
    fn adopt(&mut self, device: anybox::Device) {
        let device = Arc::new(device);
        *self.published.locked() = Some(device.clone());
        let counters = device.medius().map(|d| {
            let counters = d.counters();
            (counters.restarts, counters.reconnects)
        });
        self.counters = counters.unwrap_or((0, 0));
        let row = Listed::of(&device);
        self.device = Some(device);
        *self.clock.locked() = (0, 0);
        self.update(|s| {
            let notice = s.notice.take();
            *s = Snapshot {
                port: Some(row.port.clone()),
                key: Some(row.key.clone()),
                connecting: s.connecting,
                notice,
                ..fresh(s)
            };
        });
        // the scan saw this port as busy or silent before it opened, so its row goes now
        self.list(None);
        self.identify();
        self.refresh();
        self.buttons(true);
        // only now, so a recording that starts meanwhile sees no box rather than half of one
        self.update(|s| s.connected = true);
    }

    /// opens `listed` and makes it the one, if the box that answers is that box
    fn open(&mut self, listed: &Listed) -> Result<(), Opened> {
        match self.boxes.open(listed) {
            Opened::Device(device) => {
                if crate::hardware::boxes::key(&device.snapshot().info) != listed.key {
                    let _ = device.close();
                    return Err(Opened::Absent(format!(
                        "another box answered on {}, so pick again",
                        listed.port
                    )));
                }
                self.adopt(device);
                Ok(())
            }
            other => Err(other),
        }
    }

    /// the remembered box if it's there, otherwise the only usable one, otherwise nothing: with
    /// more than one the user picks. it holds the readback reservation the whole time, so a
    /// recording or an arm can't start while a box is being found and opened
    fn settle(&mut self) {
        if self.device.is_some() {
            return;
        }
        self.update(|s| {
            s.pending_commands += 1;
            s.connecting = true;
            s.picking = false;
            s.error = None;
            s.notice = None;
        });
        self.choose();
        self.update(|s| {
            s.pending_commands = s.pending_commands.saturating_sub(1);
            s.connecting = false;
        });
    }

    fn choose(&mut self) {
        self.seen = Some(self.boxes.candidates());
        self.list(Some(self.boxes.scan()));
        let listed = self.state.locked().boxes.clone();
        let remembered = self.remembered.clone();
        if let Some(saved) = &remembered {
            if let Some(row) = listed.iter().find(|row| row.key == saved.key) {
                let opened = self.open(row);
                // it can have moved to another port since
                if self.opened(opened) {
                    self.remember(row);
                }
                return;
            }
            // a scan can't use a box another program holds, nor see a Ferrum or the first version of the YSC 8K
            if let Some(row) = saved.listed() {
                match self.open(&row) {
                    Ok(()) => return,
                    Err(Opened::Blocked(why)) => {
                        self.update(|s| s.error = Some(why));
                        return;
                    }
                    Err(_) => {}
                }
            }
        }
        let usable: Vec<Listed> = listed.into_iter().filter(|row| row.usable).collect();
        match usable.as_slice() {
            [only] => {
                let opened = self.open(only);
                if !self.opened(opened) {
                    return;
                }
                if remembered.is_none() {
                    self.remember(only);
                    return;
                }
                let label = only.label();
                self.update(|s| s.notice = Some(format!("using {label} for now")))
            }
            // the ports the scan couldn't use say why on their own
            [] => {}
            _ => self.update(|s| s.picking = true),
        }
    }

    fn remember(&mut self, listed: &Listed) {
        self.remembered = Some(listed.saved());
        self.update(|s| s.remember = Some(listed.saved()));
    }

    /// the user's pick, which replaces whatever box is connected
    fn pick(&mut self, key: &str) {
        let (row, current) = {
            let state = self.state.locked();
            let row = state.boxes.iter().find(|row| row.key == key).cloned();
            (row, state.key.clone())
        };
        let Some(row) = row else {
            self.update(|s| {
                s.notice =
                    Some("that box isn't in the list anymore, so look again and pick it".into())
            });
            return;
        };
        // the box connected in place of a missing one, kept from now on
        if current.as_deref() == Some(key) {
            self.remember(&row);
            return;
        }
        // connecting before the old box goes, so the window sees one switch and not a gap
        self.update(|s| s.connecting = true);
        self.forget();
        self.off = false;
        let opened = self.open(&row);
        self.update(|s| {
            s.connecting = false;
            s.picking = false;
        });
        if self.opened(opened) {
            self.remember(&row);
        }
    }

    /// whether the box opened, and why not where it didn't
    fn opened(&self, result: Result<(), Opened>) -> bool {
        match result {
            Ok(()) => true,
            Err(Opened::Blocked(why) | Opened::Absent(why)) => {
                self.update(|s| s.error = Some(why));
                false
            }
            Err(Opened::Device(_)) => false,
        }
    }

    /// the published list: the connected box first, then what the scan saw, without the port this
    /// app holds, which a scan only sees as busy
    fn list(&mut self, found: Option<Scan>) {
        if let Some(found) = found {
            self.found = found;
        }
        let held = self.device.as_deref().map(Listed::of);
        let port = held.as_ref().map(|row| row.port.clone());
        let mut boxes: Vec<Listed> = Vec::new();
        for row in held.into_iter().chain(self.found.boxes.iter().cloned()) {
            if !boxes.iter().any(|listed| listed.key == row.key) {
                boxes.push(row);
            }
        }
        let unusable: Vec<Unusable> = self
            .found
            .unusable
            .iter()
            .filter(|row| Some(&row.port) != port.as_ref())
            .cloned()
            .collect();
        self.update(|s| {
            s.boxes = boxes;
            s.unusable = unusable;
        });
    }

    /// while no box is open and none was let go of on purpose, a box plugged in later connects by
    /// itself. only a change in where a box could be runs a search, which costs nothing to ask
    fn watch_for_boxes(&mut self) {
        if self.off || self.seen.is_none() {
            return;
        }
        let candidates = self.boxes.candidates();
        if self.seen.as_ref() == Some(&candidates) {
            return;
        }
        if self.device.is_none() {
            self.settle();
        } else if !self.hold.load(Ordering::Relaxed) {
            // held, the change is looked at again once the hold ends. the scan runs here and not
            // when the picker opens, because it opens every box and a MAKCU takes a while
            self.seen = Some(candidates);
            self.list(Some(self.boxes.scan()));
            self.come_back();
        }
    }

    /// a box standing in for the remembered one gives way to it as soon as the last scan sees it
    fn come_back(&mut self) {
        let Some(saved) = self.remembered.clone() else {
            return;
        };
        if self.state.locked().key.as_deref() == Some(saved.key.as_str()) {
            return;
        }
        let Some(row) = self
            .found
            .boxes
            .iter()
            .find(|row| row.key == saved.key)
            .cloned()
        else {
            return;
        };
        self.update(|s| {
            s.pending_commands += 1;
            s.connecting = true;
        });
        self.forget();
        let opened = self.open(&row);
        if self.opened(opened) {
            self.remember(&row);
        } else {
            self.settle();
        }
        self.update(|s| {
            s.pending_commands = s.pending_commands.saturating_sub(1);
            s.connecting = false;
        });
    }

    /// a box that opened at startup but doesn't answer is let go, rather than asked again every
    /// second for as long as the app runs
    fn disable_failed_startup(&mut self) {
        let error = self.state.locked().error.clone();
        if let (Some(error), true) = (error, self.device.is_some()) {
            self.forget();
            self.off = true;
            self.update(|s| {
                s.error = Some(format!(
                    "the box didn't answer when the app started, so it was let go: {}. connect it to retry",
                    error.replace(". asking again every second", "")
                ))
            });
        }
    }

    /// the crate's own device, which the settings, the health poll and the box's input stream
    /// use. read fresh: anybox keeps the same one across a reconnect, and only its close takes it
    /// away
    fn medius(&self) -> Option<Device> {
        self.device.as_ref()?.medius()
    }

    fn catch(&mut self, on: bool) {
        if let Some((stop, thread)) = self.catch.take() {
            stop.store(true, Ordering::Relaxed);
            let _ = thread.join();
        }
        self.update(|s| {
            s.catching = false;
            if on {
                s.discontinuities += 1;
            }
        });
        if !on {
            return;
        }
        // only a Medius stamps its input with its own clock, which is what pairs it with the wire
        let Some(device) = self.device.as_ref().filter(|device| {
            device.medius().is_some()
                && device
                    .snapshot()
                    .capabilities
                    .get(Action::MotionEvents)
                    .is_supported()
        }) else {
            return;
        };
        let events = device.events_of(EventKinds::MOTION);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        self.update(|s| s.catching = true);
        let (tx, state, clock, read_to) = (
            self.motion.clone(),
            Arc::clone(&self.state),
            Arc::clone(&self.clock),
            Arc::clone(&self.read_to),
        );
        let thread = std::thread::spawn(move || {
            let error = route_motion(&events, &stop, &tx, &state, &clock, &read_to);
            // in one go: a recording that ends sees the stream stopped only with the break in it
            let mut state = state.locked();
            state.catching = false;
            state.discontinuities += 1;
            if let Some(error) = error.filter(|_| !stop.load(Ordering::Relaxed)) {
                state.error = Some(error.into());
            }
        });
        self.catch = Some((stop_flag, thread));
    }

    fn apply(&mut self, change: impl FnOnce(&Device) -> Result<(), medius::Error>) {
        let Some(device) = self.medius() else {
            return;
        };
        if let Err(e) = change(&device) {
            self.update(|s| s.error = Some(e.to_string()));
        }
        self.refresh();
    }

    /// the things that do not change while connected: who the box is and what it clones. the
    /// crate's own replies carry every field the app keeps, including the firmware string, the
    /// protocol number and the sidecar's 12 hex digit MAC, none of which anybox has
    fn identify(&mut self) {
        if let Some(device) = &self.device {
            self.behind = behind(device);
            let has_mouse = self.behind.0;
            let snapshot = device.snapshot();
            let can = |action| snapshot.capabilities.get(action).is_supported();
            let (can_buttons, can_motion) = (can(Action::ButtonEvents), can(Action::MotionEvents));
            let family = snapshot.info.family.to_string();
            // the ids of what the box presents, from any box that says them; a Medius answers
            // again below with its own reply
            let presents = self.behind.1;
            self.update(|s| {
                s.has_mouse = has_mouse;
                s.can_buttons = can_buttons;
                s.can_motion = can_motion;
                s.family = family;
                s.clone_vid = presents.map(|(vid, _)| vid);
                s.clone_pid = presents.map(|(_, pid)| pid);
            });
        }
        let Some(device) = self.medius() else {
            let Some(info) = self.device.as_ref().map(|d| d.snapshot().info.clone()) else {
                return;
            };
            self.update(|s| {
                s.name = match &info.model {
                    Some(model) => format!("{} {model}", info.family),
                    None => info.family.to_string(),
                };
                s.firmware = info.firmware.map(|f| f.to_string()).unwrap_or_default();
            });
            return;
        };
        match device.query_version() {
            Ok(v) => self.update(|s| {
                s.name = v.name.clone();
                s.firmware = v.to_string();
                s.fw = (v.fw_major, v.fw_minor, v.fw_patch);
                s.protocol = v.proto_ver;
                s.mac = v.mac_hex();
            }),
            Err(e) => self.update(|s| s.error = Some(format!("version: {e}"))),
        }
        match device.device_info() {
            Ok(d) => self.update(|s| {
                s.clone = d.to_string();
                s.kind = d.kind.to_string();
                s.clone_vid = d.is_cloned().then_some(d.vid);
                s.clone_pid = d.is_cloned().then_some(d.pid);
                s.clone_product = d.product.clone();
            }),
            Err(e) => self.update(|s| s.error = Some(format!("device info: {e}"))),
        }
        match device.firmware_info() {
            Ok(fw) => self.update(|s| {
                s.host_firmware = fw
                    .host
                    .as_ref()
                    .map_or("host chip not answering".to_string(), |h| h.to_string());
                s.host_fw = fw.host.as_ref().map(|h| (h.major, h.minor, h.patch));
            }),
            Err(e) => self.update(|s| s.error = Some(format!("firmware: {e}"))),
        }
        match device.caps() {
            Ok(caps) => self.update(|s| {
                s.has_report_id = caps.has_mouse().then_some(caps.mouse.has_report_id);
            }),
            Err(e) => self.update(|s| s.error = Some(format!("caps: {e}"))),
        }
    }

    /// whether a mouse is behind the box, which can change while it stays connected: a MAKCU
    /// routes none for a moment after a replug, and a mouse can be plugged in or pulled out
    fn describe(&mut self) {
        let Some(device) = &self.device else {
            return;
        };
        // what the box clones goes with the mouse behind it, so a mouse plugged in or swapped
        // is read again
        if behind(device) != self.behind {
            self.identify();
        }
    }

    /// the things that move: health, the learnt rate, and every option the panel can set. stops
    /// at the first failed query so a box that is not answering costs one timeout, not six
    fn refresh(&mut self) {
        let Some(device) = self.medius() else {
            return;
        };
        let result = (|| -> Result<(), String> {
            let h = device.query_health().map_err(|e| format!("health: {e}"))?;
            let stats = device.query_stats().map_err(|e| format!("stats: {e}"))?;
            let r = device.query_rate().map_err(|e| format!("rate: {e}"))?;
            let render = device.query_render().map_err(|e| format!("render: {e}"))?;
            let emit = device.query_emit_pace().map_err(|e| format!("emit: {e}"))?;
            let spread = device.query_spread().map_err(|e| format!("spread: {e}"))?;
            let riding = device
                .query_movement_riding()
                .map_err(|e| format!("riding: {e}"))?;
            let imperfect = device
                .query_imperfect()
                .map_err(|e| format!("imperfect: {e}"))?;
            let counters = device.counters();
            let restarted = (counters.restarts, counters.reconnects) != self.counters;
            self.counters = (counters.restarts, counters.reconnects);
            self.update(|s| {
                if s.connected
                    && (s.session != stats.session || s.resets != stats.reset_count || restarted)
                {
                    s.discontinuities += 1;
                    s.generation += 1;
                }
                s.session = stats.session;
                s.resets = stats.reset_count;
                s.link_drops = stats.link_rx_drops;
                s.host_drops = stats.host_rx_drops;
                s.tx_drops = stats.tx_drops;
                s.imperfect_allowed = imperfect.allowed;
                s.over_capacity = imperfect.over_capacity;
                s.clone_imperfect = imperfect.clone_imperfect;
                s.link_up = h.link_up;
                s.mouse_attached = h.mouse_attached;
                s.clone_configured = h.clone_configured;
                s.native_period_us = r.native_period_us;
                s.poll_period_us = r.poll_period_us;
                s.rate_confident = r.confident;
                s.render_mode = Some(render.mode);
                s.render_full = render.full;
                s.render_ready = render.ready;
                s.emit_mode = Some(emit.mode);
                s.resolved_hz = emit.resolved_hz;
                s.force_hz = emit.force_hz;
                s.advertised_hz = emit.advertised_hz;
                s.force_active = emit.force_active;
                s.spread_percent = spread.percent;
                s.spread_span_us = spread.span_us;
                s.riding = Some(riding);
                s.error = None;
            });
            Ok(())
        })();
        if let Err(text) = result {
            // a forced rate re-clones the box and the port is gone for a few seconds. the
            // library's reader reconnects by itself, so this keeps the device and keeps asking
            self.update(|s| {
                s.error = Some(format!("{text}. asking again every second"));
            });
        }
    }
}

impl Worker {
    /// the button feed, open while a box is connected; it says when it comes up and when it is
    /// lost, the held buttons of a lost one being unknown
    fn buttons(&mut self, on: bool) {
        if let Some((stop, thread)) = self.button_feed.take() {
            stop.store(true, Ordering::Relaxed);
            let _ = thread.join();
        }
        let Some(device) = self.device.as_ref().filter(|_| on) else {
            let _ = self.buttons.send(Feed::Lost(NOT_CONNECTED.into()));
            return;
        };
        // a MAKCU with no mouse behind it has none, and the next tick looks again
        if !device
            .snapshot()
            .capabilities
            .get(Action::ButtonEvents)
            .is_supported()
        {
            let _ = self.buttons.send(Feed::Lost(NO_BUTTONS.into()));
            return;
        }
        let events = device.events_of(EventKinds::BUTTONS);
        let stop = Arc::new(AtomicBool::new(false));
        let (stopping, feed, state) = (stop.clone(), self.buttons.clone(), self.state.clone());
        let thread = std::thread::spawn(move || route_buttons(&events, &stopping, &feed, &state));
        self.button_feed = Some((stop, thread));
    }
}

const NOT_CONNECTED: &str = "the box isn't connected";
const NO_BUTTONS: &str = "the box won't send its buttons";

/// the box's button edges as the held set the feed carries. edges of one report are one change,
/// so two buttons going down together reach the binding learner together
/// the button feed is the one reader of anybox's link events, so it is also what tells the rest
/// the box changed under them, and what counts the moves the box refused later
fn route_buttons(
    events: &Events,
    stopping: &AtomicBool,
    feed: &mpsc::Sender<Feed>,
    state: &Mutex<Snapshot>,
) {
    let _ = feed.send(Feed::Ready);
    let mut route = Route::default();
    // anybox has no peek, so an event read while gathering one report's edges waits here
    let mut next: Option<Event> = None;
    while !stopping.load(Ordering::Relaxed) {
        let event = match next
            .take()
            .map_or_else(|| events.recv_timeout(Duration::from_millis(50)), Ok)
        {
            Ok(event) => event,
            Err(anybox::RecvTimeoutError::Timeout) => continue,
            Err(anybox::RecvTimeoutError::Closed) => {
                let _ = feed.send(Feed::Lost(NOT_CONNECTED.into()));
                return;
            }
        };
        match event {
            Event::Disconnected | Event::Reconnected | Event::SessionReset => {
                let mut state = state.locked();
                state.generation += 1;
                state.discontinuities += 1;
            }
            Event::Rejected {
                action: Action::Move | Action::DiscardPendingMoves,
                count,
                ..
            } => state.locked().refused_moves += count,
            _ => {}
        }
        let at = crate::capture::clock::Anchor::take();
        let mut step = route.on(&event);
        if let (Event::Button { at: first, .. }, Step::Held(_)) = (&event, &step) {
            // anybox hands each edge of a report over on its own, a moment apart. a sleep here
            // lasts a whole timer tick on Windows, so the rest of the report is waited for by
            // spinning, briefly
            let until = std::time::Instant::now() + REPORT_SPREAD;
            let queued = || loop {
                if let Ok(event) = events.try_recv() {
                    return Some(event);
                }
                if std::time::Instant::now() >= until {
                    return None;
                }
                std::thread::yield_now();
            };
            (step, next) = gather(&mut route, first, step, queued);
        }
        match step {
            Step::Held(mask) => {
                let _ = feed.send(Feed::Buttons { mask, at });
            }
            Step::Lost(why) => {
                let _ = feed.send(Feed::Lost(why.into()));
            }
            Step::Up => {
                let _ = feed.send(Feed::Ready);
            }
            Step::End(why) => {
                let _ = feed.send(Feed::Lost(why.into()));
                return;
            }
            Step::Nothing => {}
        }
    }
}

/// the box's own input as rows on its host chip clock, until the stream stops or can't go on.
/// what ends it by itself says why when it's worth an error
fn route_motion(
    events: &Events,
    stopping: &AtomicBool,
    rows: &SyncSender<Motion>,
    state: &Mutex<Snapshot>,
    clock: &Mutex<(u32, u64)>,
    read_to: &AtomicI64,
) -> Option<&'static str> {
    while !stopping.load(Ordering::Relaxed) {
        // taken before the queue is looked at, so an empty queue says everything that was in it
        // by then has been read. not an event anybox is still handing over, nor a loss the box
        // only reports on its once a second poll
        let before = crate::capture::clock::qpc();
        let event = match events.try_recv() {
            Ok(event) => event,
            Err(anybox::TryRecvError::Closed) => return None,
            Err(_) => {
                read_to.store(before, Ordering::Release);
                match events.recv_timeout(Duration::from_millis(20)) {
                    Ok(event) => event,
                    Err(anybox::RecvTimeoutError::Timeout) => continue,
                    Err(anybox::RecvTimeoutError::Closed) => return None,
                }
            }
        };
        match event {
            Event::Motion {
                dx, dy, wheel, at, ..
            } => {
                // only a box clock pairs with the wire
                let Some(anybox::BoxTime {
                    micros,
                    clock: anybox::Clock::MediusHostChip,
                    ..
                }) = at.box_time
                else {
                    return Some("the box doesn't stamp its input with its own clock");
                };
                let box_us = unwrapped(&mut clock.locked(), micros);
                let arrived_qpc = at.received_qpc.unwrap_or_else(crate::capture::clock::qpc);
                // the paired shift is the lowest receipt offset seen, so a receipt must never
                // come out early
                let arrived_us = crate::capture::clock::Anchor::take()
                    .utc_never_early(arrived_qpc, crate::capture::clock::frequency())
                    .unwrap_or_else(crate::capture::session::wall_us);
                // a Medius reports i16, so this never clamps there
                let narrow = |v: i32| v.clamp(i16::MIN.into(), i16::MAX.into()) as i16;
                let row = Motion {
                    arrived_us,
                    arrived_qpc,
                    box_us,
                    dx: narrow(dx),
                    dy: narrow(dy),
                    dz: narrow(wheel),
                };
                if rows.try_send(row).is_err() {
                    state.locked().input_drops += 1;
                }
            }
            Event::Dropped { count, .. } => state.locked().catch_dropped += count,
            Event::Disconnected | Event::Reconnected | Event::SessionReset => {
                unsure_of_restart(&mut clock.locked());
                state.locked().discontinuities += 1;
            }
            Event::Rejected {
                action: Action::MotionEvents,
                ..
            } => return Some("the box won't send its input"),
            _ => {}
        }
    }
    None
}

/// the host chip stamps in u32 microseconds since it booted, which rolls over every 71 minutes. a
/// stamp far below the last one is the rollover, not a step back; the count outlives a stream, so
/// a stream started again goes on in the same clock, unless it was off for over half a rollover,
/// which nothing here can tell
fn unwrapped(clock: &mut (u32, u64), micros: u32) -> u64 {
    let (last, epochs) = clock;
    if micros < *last && *last - micros > 1 << 31 {
        *epochs += 1;
    }
    *last = micros;
    (*epochs << 32) | u64::from(micros)
}

/// the box may have restarted, or not: a Medius reconnects for a clone swap too, with its clock
/// going on. a restarted clock counts from zero, which the rollover rule would read as time going
/// forward, so the next stamp never counts as a rollover. a restart then shows as time going back,
/// and a clock that went on carries on
fn unsure_of_restart(clock: &mut (u32, u64)) {
    clock.0 = 0;
}

/// the state with a box let go: the counters only ever go up, so rows and moves on either side
/// are told apart, and what was found and what's remembered stay
fn fresh(s: &Snapshot) -> Snapshot {
    Snapshot {
        discontinuities: s.discontinuities + 1,
        generation: s.generation + 1,
        pending_commands: s.pending_commands,
        connecting: s.connecting,
        boxes: s.boxes.clone(),
        remember: s.remember.clone(),
        unusable: s.unusable.clone(),
        ..Snapshot::default()
    }
}

/// how long the edges of one report can take to follow each other out of anybox
const REPORT_SPREAD: Duration = Duration::from_micros(200);

/// the rest of the report `first` began, folded into the route; the first event of another
/// report is handed back for the next turn
fn gather(
    route: &mut Route,
    first: &anybox::Stamp,
    mut step: Step,
    mut queued: impl FnMut() -> Option<Event>,
) -> (Step, Option<Event>) {
    while let Some(event) = queued() {
        match &event {
            Event::Button { at, .. } if same_report(first, at) => step = route.on(&event),
            _ => return (step, Some(event)),
        }
    }
    (step, None)
}

/// what one event does to the feed
#[derive(Debug, PartialEq)]
enum Step {
    Held(u32),
    Lost(&'static str),
    Up,
    /// the stream is no good, the next tick opens another
    End(&'static str),
    Nothing,
}

/// the held buttons as the events tell them, and whether the box is there to tell them
#[derive(Default)]
struct Route {
    held: u32,
    down: bool,
}

impl Route {
    fn on(&mut self, event: &Event) -> Step {
        match event {
            Event::Button {
                button, pressed, ..
            } if !self.down => {
                self.held = with_button(self.held, *button, *pressed);
                Step::Held(self.held)
            }
            Event::Disconnected => {
                self.down = true;
                self.held = 0;
                Step::Lost("the box disconnected, waiting for it to come back")
            }
            Event::Reconnected if self.down => {
                self.down = false;
                Step::Up
            }
            // the held buttons are unknown after these, and none held is the safe guess: a
            // release that never comes would keep the aim moving
            Event::Dropped { .. } | Event::SessionReset | Event::Reconnected | Event::Changed
                if !self.down =>
            {
                self.held = 0;
                Step::Held(0)
            }
            Event::Rejected {
                action: Action::ButtonEvents,
                ..
            } => {
                self.held = 0;
                Step::End(NO_BUTTONS)
            }
            _ => Step::Nothing,
        }
    }
}

/// whether a mouse is behind the box, and the ids of what it presents when the box says
fn behind(device: &anybox::Device) -> (bool, Option<(u16, u16)>) {
    let snapshot = device.snapshot();
    (
        snapshot.capabilities.get(Action::Move).is_supported(),
        snapshot.info.peripheral.map(|ids| (ids.vid, ids.pid)),
    )
}

/// two stamps of one report: the box's own clock where it has one, otherwise the read both came
/// in
fn same_report(a: &anybox::Stamp, b: &anybox::Stamp) -> bool {
    match (a.box_time, b.box_time) {
        (Some(a), Some(b)) => a == b,
        _ => a.received == b.received,
    }
}

/// the held set with one edge in it, each button at its medius usage id as the bindings keep it
fn with_button(held: u32, button: anybox::Button, pressed: bool) -> u32 {
    let id = match button {
        anybox::Button::Left => 0,
        anybox::Button::Right => 1,
        anybox::Button::Middle => 2,
        anybox::Button::Side1 => 3,
        anybox::Button::Side2 => 4,
        anybox::Button::Extra(id) => u32::from(id),
        _ => return held,
    };
    let Some(bit) = 1u32.checked_shl(id) else {
        return held;
    };
    if pressed { held | bit } else { held & !bit }
}

impl Drop for Worker {
    fn drop(&mut self) {
        for (stop, thread) in [self.catch.take(), self.button_feed.take()]
            .into_iter()
            .flatten()
        {
            stop.store(true, Ordering::Relaxed);
            let _ = thread.join();
        }
        // the aim's port holds the box too, so letting go of it here would not close it
        if let Some(device) = self.device.take() {
            let _ = device.close();
        }
    }
}

#[cfg(test)]
mod tests;
