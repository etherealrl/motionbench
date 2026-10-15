#[cfg(test)]
use crate::recording::hotkey;
use crate::{hardware::buttons::Buttons, targets::vision};
use motion_bench::sync::LockExt;
use motion_bench::{
    capture::session::wall_us,
    controller::linear::{Config, Linear, REVISION},
    hardware::medius::{InjectionPort, PortState},
};
use serde::Serialize;
use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};

#[derive(Clone, Default, Serialize)]
pub struct Status {
    pub ready: bool,
    pub armed: bool,
    pub active: bool,
    pub learning: bool,
    pub hotkey_mask: u32,
    pub held_mask: u32,
    pub binding_seq: u64,
    pub phase: String,
    pub commands: u64,
    pub counts: [i64; 2],
    pub target_counts: Option<[f64; 2]>,
    pub recording: bool,
    /// armed, with the box or the mouse behind it gone for now; it aims again once they're back
    pub away: bool,
    pub error: Option<String>,
    /// the buttons' trouble: the feed is gone, or a learned button could not be saved
    pub buttons_error: Option<String>,
}
#[derive(Clone)]
pub struct Source {
    pub config: motion_bench::targets::vision::Config,
    pub status: Arc<Mutex<vision::Status>>,
}
#[derive(Serialize)]
struct Evidence {
    schema: &'static str,
    algorithm: &'static str,
    revision: &'static str,
    config: Config,
    calibration: motion_bench::targets::vision::Config,
    clock: Option<motion_bench::targets::vision::Clock>,
    started_us: u64,
    ended_us: Option<u64>,
    complete: bool,
    error: Option<String>,
    commands: u64,
    counts: [i64; 2],
}
struct Sink {
    path: PathBuf,
    writer: BufWriter<File>,
    evidence: Evidence,
}
impl Sink {
    fn save(&self) -> Result<(), String> {
        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(
            &temporary,
            serde_json::to_vec_pretty(&self.evidence).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::fs::rename(temporary, &self.path).map_err(|e| e.to_string())
    }
    fn finish(mut self, error: Option<String>) -> Result<(), String> {
        let written = self
            .writer
            .flush()
            .and_then(|()| self.writer.get_ref().sync_all());
        self.evidence.error = error.or_else(|| written.as_ref().err().map(ToString::to_string));
        self.evidence.complete = self.evidence.error.is_none();
        self.evidence.ended_us = Some(wall_us());
        self.save()?;
        written.map_err(|e| e.to_string())
    }
}
struct Control {
    arm_seq: u64,
    status: Status,
    config: Config,
    source: Option<Source>,
}

/// a write to the box: one that found it gone did nothing, and a box that drops what it held
/// when it goes leaves nothing behind
fn wrote(result: Result<(), PortState>) -> Result<(), String> {
    match result {
        Ok(()) | Err(PortState::Away(_) | PortState::Usable | PortState::Lost) => Ok(()),
        Err(PortState::Broken(error)) => Err(error),
    }
}

/// the recording's journal, closed with `error` when there is one
fn finish(journal: &Mutex<Option<Sink>>, error: Option<String>) -> Result<(), String> {
    match journal.locked().take() {
        Some(sink) => sink.finish(error),
        None => Ok(()),
    }
}

/// one move the loop decided to send, with what the journal says about it
struct Sent {
    arm_seq: u64,
    dx: i16,
    dy: i16,
    source_us: u64,
    source_qpc: i64,
    delta: u64,
    target: [f64; 2],
    held_mask: u32,
}

enum Step {
    Wait,
    /// the box drops the motion it still holds from earlier moves
    Discard,
    /// the same, for a box that may be gone: whatever it can still be told to drop
    Release,
    Move(Sent),
}

/// the next step from the held buttons, the focus and the latest frame, the status updated on
/// the way; the lock is held while this runs, and nothing here writes to the box or a file
fn decide(
    c: &mut Control,
    buttons: &Buttons,
    focus: &mut impl FnMut() -> bool,
    linear: &mut Linear,
    previous: &mut Option<i64>,
) -> Result<Step, String> {
    c.status.active = false;
    c.status.target_counts = None;
    let held_mask = buttons.held();
    let reason = if buttons.aim().learning {
        Some("Press a button on the box's mouse")
    } else if !c.status.armed {
        Some("Disarmed")
    } else if held_mask & c.config.hotkey_mask == 0 {
        Some("Hold the hotkey")
    } else if !focus() {
        Some("Waiting for the game to be in front")
    } else {
        None
    };
    let discard = |previous: &mut Option<i64>, linear: &mut Linear| {
        linear.reset();
        if previous.take().is_some() {
            Step::Discard
        } else {
            Step::Wait
        }
    };
    if let Some(reason) = reason {
        c.status.phase = reason.into();
        return Ok(discard(previous, linear));
    }
    let Some(source) = c.source.clone() else {
        return Ok(Step::Wait);
    };
    let state = source.status.locked();
    if !state.running || !state.ready || state.error.is_some() {
        return Err("target capture stopped, restart it and re-arm the experiment".into());
    }
    let frame = state.last.clone();
    drop(state);
    let Some(frame) = frame else {
        return Ok(Step::Wait);
    };
    let age = motion_bench::capture::clock::elapsed_us(
        frame.source_qpc,
        motion_bench::capture::clock::qpc(),
    );
    let fresh = age.is_some_and(|age| age <= source.config.max_frame_gap_ms as u64 * 1000);
    if !fresh || frame.detections.len() != 1 {
        c.status.phase = if fresh {
            "Waiting for exactly one target"
        } else {
            "Waiting for a fresh target frame"
        }
        .into();
        return Ok(discard(previous, linear));
    }
    let bounds = frame.detections[0].bounds;
    let target = source
        .config
        .counts((bounds[0] + bounds[2]) / 2.0, (bounds[1] + bounds[3]) / 2.0);
    c.status.target_counts = Some(target);
    let was_settled = linear.settled();
    let settled = linear.hold(target, c.config.deadzone_counts);
    c.status.phase = if settled {
        "Settled in deadzone"
    } else {
        "Tracking"
    }
    .into();
    c.status.active = !settled;
    let stamp = frame.source_qpc;
    let old = previous.replace(stamp);
    if settled {
        return Ok(if was_settled {
            Step::Wait
        } else {
            Step::Discard
        });
    }
    let Some(old) = old.filter(|old| *old != stamp) else {
        return Ok(Step::Wait);
    };
    let Some(delta) = motion_bench::capture::clock::elapsed_us(old, stamp) else {
        linear.reset();
        *previous = None;
        return Ok(Step::Wait);
    };
    let [dx, dy] = linear.step(target, &c.config, delta as f64 / 1_000_000.0);
    if dx == 0 && dy == 0 {
        return Ok(Step::Wait);
    }
    Ok(Step::Move(Sent {
        arm_seq: c.arm_seq,
        dx,
        dy,
        source_us: frame.source_us,
        source_qpc: frame.source_qpc,
        delta,
        target,
        held_mask,
    }))
}
pub struct Worker {
    wakeup: std::sync::mpsc::SyncSender<()>,
    port: InjectionPort,
    buttons: Arc<Buttons>,
    /// the recording's journal, apart from the status so writing it never blocks a snapshot
    journal: Arc<Mutex<Option<Sink>>>,
    /// held around every write to the box, so a disarm's discard comes after any move in flight
    io: Arc<Mutex<()>>,
    /// let go of because the box got stuck; the buttons and wakeups belong to the replacement
    retired: AtomicBool,
    control: Arc<Mutex<Control>>,
    stop: Arc<AtomicBool>,
    /// shared, so a stop can run without the lock the worker is kept under
    thread: Mutex<Option<JoinHandle<()>>>,
}
impl Worker {
    pub fn start(port: InjectionPort, buttons: Arc<Buttons>) -> Self {
        Self::start_with_focus(port, foreground(), buttons)
    }
    fn start_with_focus(
        port: InjectionPort,
        mut focus: impl FnMut() -> bool + Send + 'static,
        buttons: Arc<Buttons>,
    ) -> Self {
        let control = Arc::new(Mutex::new(Control {
            arm_seq: 0,
            status: Status {
                phase: "Disarmed".into(),
                ..Status::default()
            },
            config: Config::default(),
            source: None,
        }));
        let journal: Arc<Mutex<Option<Sink>>> = Arc::default();
        let io = Arc::new(Mutex::new(()));
        let stop = Arc::new(AtomicBool::new(false));
        let (shared, stopping) = (control.clone(), stop.clone());
        let driver = port.clone();
        let (wakeup, messages) = std::sync::mpsc::sync_channel(1);
        buttons.wake_with(Some(wakeup.clone()));
        let (held, writes, serial) = (buttons.clone(), journal.clone(), io.clone());
        let thread = std::thread::spawn(move || {
            let (port, buttons, journal, io) = (driver, held, writes, serial);
            let result = (|| -> Result<(), String> {
                let mut linear = Linear::default();
                let mut previous = None;
                let mut arm_seq = 0;
                // after a comeback the held buttons are a guess until the box reports some, so
                // the aim waits for a report with the aim button down
                let mut fresh_from: Option<u64> = None;
                while !stopping.load(Ordering::Relaxed) {
                    let _ = messages.recv_timeout(std::time::Duration::from_millis(50));
                    // what to do is decided under the lock; the serial writes and the journal
                    // happen after it is let go, so a blocked box never blocks a snapshot
                    let step = {
                        let mut c = shared.locked();
                        if arm_seq != c.arm_seq {
                            arm_seq = c.arm_seq;
                            linear.reset();
                            previous = None;
                        }
                        if let Some(error) = &c.status.error {
                            return Err(error.clone());
                        }
                        if stopping.load(Ordering::Relaxed) {
                            break;
                        }
                        if c.status.away && buttons.ready() && port.back() {
                            port.rebase();
                            c.status.away = false;
                            fresh_from = Some(buttons.reports());
                        }
                        // a lost feed's held buttons are unknown, so an armed hold can't be
                        // trusted until it's back
                        let box_state = match port.state() {
                            PortState::Usable if !buttons.ready() => {
                                PortState::Away("the box's buttons aren't there")
                            }
                            other => other,
                        };
                        let recording = journal.locked().is_some();
                        match box_state {
                            PortState::Broken(error) => return Err(error),
                            PortState::Lost if recording => {
                                return Err(
                                    "the box lost some of the aim's moves during the recording"
                                        .into(),
                                );
                            }
                            PortState::Away(why) if c.status.armed => {
                                // a recording's journal says what the aim sent, and a gap in
                                // the box is the recording's failure
                                if recording {
                                    return Err(format!("{why} during the recording, arm again"));
                                }
                                c.status.active = false;
                                c.status.target_counts = None;
                                c.status.phase = "Waiting for the box to come back".into();
                                linear.reset();
                                previous = None;
                                // a box that only lost its buttons still holds what it was sent
                                if std::mem::replace(&mut c.status.away, true) {
                                    Step::Wait
                                } else {
                                    Step::Release
                                }
                            }
                            PortState::Away(_) => Step::Wait,
                            PortState::Usable | PortState::Lost => {
                                // the next frame aims from where the cursor is, so a lost move
                                // costs nothing outside a recording
                                if box_state == PortState::Lost {
                                    port.forgive_losses();
                                }
                                let pressed_since = |mark| {
                                    buttons.reports() > mark
                                        && buttons.held() & c.config.hotkey_mask != 0
                                };
                                match fresh_from {
                                    Some(mark) if c.status.armed && !pressed_since(mark) => {
                                        c.status.phase = "Press the aim button again".into();
                                        Step::Wait
                                    }
                                    _ => {
                                        fresh_from = None;
                                        decide(
                                            &mut c,
                                            &buttons,
                                            &mut focus,
                                            &mut linear,
                                            &mut previous,
                                        )?
                                    }
                                }
                            }
                        }
                    };
                    match step {
                        Step::Wait => {}
                        Step::Discard => {
                            let _io = io.locked();
                            wrote(port.discard_pending())?;
                        }
                        Step::Release => {
                            let _io = io.locked();
                            wrote(port.discard_unchecked())?;
                        }
                        Step::Move(sent) => {
                            let _io = io.locked();
                            // a disarm since the decision ends it, and waits for this io lock
                            // before it discards, so no move goes out after it
                            let c = shared.locked();
                            if !c.status.armed || c.arm_seq != sent.arm_seq {
                                continue;
                            }
                            drop(c);
                            let sent_us = wall_us();
                            let sent_qpc = qpc();
                            match port.move_relative(sent.dx, sent.dy) {
                                Ok(()) => {}
                                // the next turn sees the box gone and waits for it
                                Err(PortState::Away(_) | PortState::Usable | PortState::Lost) => {
                                    continue;
                                }
                                Err(PortState::Broken(error)) => return Err(error),
                            }
                            let completed_qpc = qpc();
                            let completed_us = wall_us();
                            {
                                let mut c = shared.locked();
                                c.status.commands += 1;
                                c.status.counts[0] += sent.dx as i64;
                                c.status.counts[1] += sent.dy as i64;
                            }
                            if let Some(sink) = &mut *journal.locked() {
                                serde_json::to_writer(&mut sink.writer, &serde_json::json!({
                                    "source_us": sent.source_us, "sent_us": sent_us, "completed_us": completed_us,
                                    "source_qpc": sent.source_qpc,
                                    "sent_qpc": sent_qpc, "completed_qpc": completed_qpc,
                                    "delta_us": sent.delta, "target_counts": sent.target,
                                    "dx": sent.dx, "dy": sent.dy, "held_mask": sent.held_mask,
                                })).map_err(|e| e.to_string())?;
                                sink.writer.write_all(b"\n").map_err(|e| e.to_string())?;
                                sink.evidence.commands += 1;
                                sink.evidence.counts[0] += sent.dx as i64;
                                sink.evidence.counts[1] += sent.dy as i64;
                            }
                        }
                    }
                }
                Ok(())
            })();
            stopping.store(true, Ordering::Relaxed);
            buttons.arm(false);
            if result.is_err() {
                let _io = io.locked();
                let _ = port.discard_unchecked();
            }
            let error = result
                .err()
                .or_else(|| shared.locked().status.error.clone());
            let finished = finish(&journal, error.clone());
            let mut c = shared.locked();
            c.status.error = finished.err().or(error);
            c.status.armed = false;
            c.status.active = false;
            c.status.away = false;
            c.status.recording = false;
        });
        Self {
            wakeup,
            port,
            buttons,
            journal,
            io,
            control,
            stop,
            retired: AtomicBool::new(false),
            thread: Mutex::new(Some(thread)),
        }
    }
    /// the controller's status, its binding and held buttons as the buttons have them
    pub fn snapshot(&self) -> Status {
        let mut status = self.control.locked().status.clone();
        self.buttons.status(&mut status);
        status
    }
    pub fn arm(&self, config: Config, source: Source) -> Result<(), String> {
        self.buttons.require()?;
        source.config.validate()?;
        if source.config.scenario.trim().is_empty() {
            return Err("name the scenario in Capture before arming".into());
        }
        let state = source.status.locked();
        if !state.ready || !state.running || state.error.is_some() {
            return Err("start target capture and wait for the model".into());
        }
        drop(state);
        let mut c = self.control.locked();
        if c.status.armed {
            return Err("disarm before changing experiment settings".into());
        }
        if c.status.error.is_some() || c.status.recording {
            return Err("restart the experiment after finishing the current recording".into());
        }
        c.status.armed = true;
        c.status.away = false;
        c.arm_seq += 1;
        // the binding is the buttons', whatever the settings say
        c.config = Config {
            hotkey_mask: self.buttons.aim().mask,
            ..config
        };
        self.buttons.arm(true);
        source.status.locked().experiment_wakeup = Some(self.wakeup.clone());
        c.source = Some(source);
        let _ = self.wakeup.try_send(());
        Ok(())
    }
    /// let go of for good, nothing it shares is touched by it any more
    pub fn retire(&self) {
        self.retired.store(true, Ordering::Release);
    }
    pub fn disarm(&self) -> Result<(), String> {
        // the buttons stop driving it first, nothing below may hold that up
        if !self.retired.load(Ordering::Acquire) {
            self.buttons.arm(false);
        }
        let armed = {
            let mut c = self.control.locked();
            let armed = c.status.armed;
            c.status.armed = false;
            c.status.active = false;
            c.status.away = false;
            c.status.phase = "Disarmed".into();
            armed
        };
        let _ = self.wakeup.try_send(());
        let discarded = if armed {
            let _io = self.io.locked();
            wrote(self.port.discard_pending())
        } else {
            Ok(())
        };
        self.close_journal(discarded)
    }
    /// the recording's journal closed and the aim left armed, the maintainer disarms by hand
    pub fn end_recording(&self) -> Result<(), String> {
        // a move on its way to the box is journaled before the journal closes
        let _io = self.io.locked();
        self.close_journal(Ok(()))
    }
    fn close_journal(&self, discarded: Result<(), String>) -> Result<(), String> {
        // a box that went during the recording leaves a gap in what the journal says went out,
        // whichever of the disarm and the worker sees it first
        let recording = self.journal.locked().is_some();
        let discarded = discarded.and_then(|()| match self.port.state() {
            PortState::Away(why) if recording => Err(format!("{why} during the recording")),
            PortState::Lost if recording => {
                Err("the box lost some of the aim's moves during the recording".into())
            }
            PortState::Broken(error) if recording => Err(error),
            _ => Ok(()),
        });
        let error = discarded
            .err()
            .or_else(|| self.control.locked().status.error.clone());
        let finished = finish(&self.journal, error.clone());
        let mut c = self.control.locked();
        c.status.recording = false;
        c.status.error = finished.err().or(error);
        match &c.status.error {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
    pub fn begin(&self, pcap: &Path) -> Result<(), String> {
        // what the journal needs is read under the lock; its files are made after
        let (config, source) = {
            let c = self.control.locked();
            if !c.status.armed {
                return Ok(());
            }
            if !self.buttons.ready() || c.status.error.is_some() {
                return Err("the Medius buttons are not ready".into());
            }
            (
                c.config.clone(),
                c.source.clone().ok_or("missing target calibration")?,
            )
        };
        let calibration = source.config.clone();
        let clock = source.status.locked().clock.clone();
        let sink = Sink {
            path: pcap.with_extension("injection.json"),
            writer: BufWriter::new(
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(pcap.with_extension("injections.jsonl"))
                    .map_err(|e| e.to_string())?,
            ),
            evidence: Evidence {
                schema: "motion-bench.injection.v1",
                algorithm: "linear_constant_speed",
                revision: REVISION,
                config,
                calibration,
                clock,
                started_us: wall_us(),
                ended_us: None,
                complete: false,
                error: None,
                commands: 0,
                counts: [0; 2],
            },
        };
        sink.save()?;
        *self.journal.locked() = Some(sink);
        self.control.locked().status.recording = true;
        Ok(())
    }
    /// the injection journal closed with `why` without waiting on the box, for a disarm stuck
    /// on it; the disarm then finds nothing left to close
    pub fn abandon(&self, why: &str) {
        let _ = finish(&self.journal, Some(why.into()));
    }
    pub fn provenance(&self) -> serde_json::Value {
        let c = self.control.locked();
        serde_json::json!({"enabled":c.status.armed, "algorithm":"linear_constant_speed", "revision":REVISION, "config":c.config})
    }
    /// the lock every write to the box takes; held, the box looks stuck
    #[cfg(test)]
    pub(crate) fn hold_box(&self) -> std::sync::MutexGuard<'_, ()> {
        self.io.locked()
    }
    /// stopped once: the wakeups and the buttons are shared with the worker that replaces this
    /// one, so a later stop, or the drop of a last handle, leaves them alone
    pub fn stop(&self) {
        let Some(thread) = self.thread.locked().take() else {
            return;
        };
        let _ = self.disarm();
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.wakeup.try_send(());
        let _ = thread.join();
        if self.retired.load(Ordering::Acquire) {
            return;
        }
        self.buttons.wake_with(None);
        if let Some(source) = &self.control.locked().source {
            source.status.locked().experiment_wakeup = None;
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(windows)]
fn qpc() -> i64 {
    let mut value = 0;
    unsafe { windows::Win32::System::Performance::QueryPerformanceCounter(&mut value) }
        .expect("QPC is available on supported Windows versions");
    value
}
#[cfg(not(windows))]
fn qpc() -> i64 {
    0
}

/// a game or trainer is in front, so moves never land in Motion Bench itself or on the desktop
#[cfg(windows)]
fn foreground() -> impl FnMut() -> bool + Send {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    let own = std::process::id();
    move || unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
        pid != 0 && pid != own
    }
}
#[cfg(not(windows))]
fn foreground() -> impl FnMut() -> bool + Send {
    || false
}

#[cfg(test)]
pub(crate) mod tests;
