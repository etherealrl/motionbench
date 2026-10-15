//! the desktop's side of the recording coordinator: it asks the coordinator, carries out the
//! effects it returns and reports back what they did. a request that returns effects makes its
//! caller their driver until none is left; the coordinator only returns effects when it leaves
//! a phase nothing is driving, so one thread drives at a time. reports that are not a request
//! (a failure the snapshot pump saw, a claim let go) drive theirs on a thread of their own
//!
//! the coordinator and the attempt each sit behind a lock taken last: never held while taking
//! the bench, the capture's data or a worker, never held while an effect runs

use motion_bench::sync::LockExt;
mod words;

pub(crate) use words::announce;
use words::refused;

pub mod hotkey;

pub mod commit;

use crate::{
    app::state::{Bench, Shared},
    capture::observers,
    library::import,
    targets::vision,
};
use commit::commit_recording;
use motion_bench::{
    capture::{clock::Anchor, session::Data},
    coordination::readiness::BROKEN,
    coordination::recording::{Attempt, Coordinator, Effect, Event, Part, Refusal},
    evidence::{
        commit::participant::{INJECTION, MEDIUS_INPUT, OBSERVERS, RECORDING_CONTROL, VISION},
        reason::{Failure, Reason, StopReason},
        recorder::Recorder,
    },
    hardware::medius::{self, Motion},
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

#[derive(Default)]
pub(crate) struct Recording {
    gate: Mutex<Gate>,
    current: Mutex<Option<Current>>,
    /// the latest recording whose start or end was said; an older one's words come too late
    said: Mutex<Option<Attempt>>,
    /// a panic was caught somewhere a lock can't show it, see `broken`
    broken: std::sync::atomic::AtomicBool,
}

#[derive(Default)]
struct Gate {
    coordinator: Coordinator,
    /// a recording button press on its way to the worker that runs it
    pressed: bool,
}

/// what one recording keeps between its effects
struct Current {
    start: Anchor,
    /// when its stop was asked for, the first time
    stop: Option<Anchor>,
    data: Option<Arc<Mutex<Data>>>,
    pcap: Option<PathBuf>,
    control: Option<Value>,
    vision: bool,
}

/// what a drive did, for its caller to say
#[derive(Default)]
pub(crate) struct Driven {
    attempt: Option<Attempt>,
    /// when a stop's waits run out, one budget for the whole stop
    until: Option<std::time::Instant>,
    stem: Option<String>,
    /// why the first participant that could not begin did not
    begin_error: Option<String>,
    stop: Option<StopReason>,
    committed: Option<Result<PathBuf, String>>,
    listed: Option<Result<(), String>>,
    events: Option<Result<usize, String>>,
    warnings: Vec<String>,
}

/// held by a command that must not run beside a start or a stop; letting go runs what was put
/// off meanwhile
pub(crate) struct Claim(Shared);

impl Drop for Claim {
    fn drop(&mut self) {
        let effects = self.0.recording.report(Event::Release);
        spawn(&self.0, effects);
    }
}

impl Recording {
    // a panic while it was held leaves the coordinator as consistent as its last call did
    fn gate(&self) -> MutexGuard<'_, Gate> {
        self.gate.locked()
    }

    fn current(&self) -> MutexGuard<'_, Option<Current>> {
        self.current.locked()
    }

    /// marks the app broken; true the first time, so whoever sees it first disarms
    pub(crate) fn broke(&self) -> bool {
        !self.broken.swap(true, std::sync::atomic::Ordering::AcqRel)
    }

    pub(crate) fn is_broken(&self) -> bool {
        self.broken.load(std::sync::atomic::Ordering::Acquire)
    }

    pub(crate) fn busy(&self) -> bool {
        let gate = self.gate();
        gate.coordinator.busy() || gate.pressed
    }

    pub(crate) fn recording(&self) -> bool {
        self.gate().coordinator.recording()
    }

    pub(crate) fn closed(&self) -> bool {
        self.gate().coordinator.closed()
    }

    pub(crate) fn attempt(&self) -> Option<Attempt> {
        self.gate().coordinator.attempt()
    }

    /// a recording button press, refused while anything is on its way or another press is
    pub(crate) fn press(&self) -> bool {
        let mut gate = self.gate();
        let free = !gate.coordinator.busy() && !gate.pressed;
        gate.pressed |= free;
        free
    }

    /// a press that will not be run after all
    pub(crate) fn unpress(&self) {
        self.gate().pressed = false;
    }

    pub(crate) fn claim(state: &Shared) -> Result<Claim, String> {
        if broken(state) {
            return Err(BROKEN.into());
        }
        let mut gate = state.recording.gate();
        if gate.pressed {
            return Err(Refusal::Busy.message().into());
        }
        gate.coordinator
            .on(Event::Claim)
            .map_err(|refusal| refusal.message().to_string())?;
        Ok(Claim(state.clone()))
    }

    /// a start, a stop or the exit; `press` takes the recording button's press with it. only
    /// the exit is taken while something is on its way, so a refused request leaves nothing
    /// behind to act on later; it drops a press still waiting for the worker
    fn request(&self, event: Event, at: Anchor, press: bool) -> Result<Vec<Effect>, Refusal> {
        let mut gate = self.gate();
        let shutdown = event == Event::Shutdown;
        if press || shutdown {
            gate.pressed = false;
        } else if gate.pressed {
            return Err(Refusal::Busy);
        }
        if !shutdown && gate.coordinator.busy() {
            return Err(if gate.coordinator.closing() {
                Refusal::Closing
            } else {
                Refusal::Busy
            });
        }
        let start = matches!(event, Event::Start(_));
        let effects = gate.coordinator.on(event)?;
        let mut current = self.current();
        if start {
            *current = Some(Current {
                start: at,
                stop: None,
                data: None,
                pcap: None,
                control: None,
                vision: false,
            });
        } else if let Some(current) = current.as_mut() {
            current.stop.get_or_insert(at);
        }
        Ok(effects)
    }

    /// what happened, never refused
    fn report(&self, event: Event) -> Vec<Effect> {
        let mut gate = self.gate();
        // a failure of the running recording asks for its stop when it is seen, even while its
        // start is still on its way; a release asks for it when it ends the recording
        let failure = matches!(event, Event::Failed(Some(attempt), _) if Some(attempt) == gate.coordinator.attempt());
        let release = event == Event::Release;
        let effects = gate.coordinator.on(event).unwrap_or_default();
        let mut current = self.current();
        if gate.coordinator.attempt().is_none() {
            *current = None;
        } else if (failure || release && !effects.is_empty())
            && let Some(current) = current.as_mut()
        {
            current.stop.get_or_insert_with(Anchor::take);
        }
        effects
    }
}

/// a failed recording or a failed capture, told to the coordinator by the snapshot pump under
/// the capture's data lock it read it with: the recording's attempt cannot change while its data
/// shows it failed, and a capture can only start again with a claim, refused while this one's
/// stop is on its way, so a late report cannot stop a newer capture
/// what it starts is driven on a thread of its own, which waits for the locks the pump holds
pub(crate) fn watch(state: &Shared, data: &Data) {
    let effects = if let Some(trouble) = data.trouble() {
        let attempt = state.recording.attempt();
        state
            .recording
            .report(Event::Failed(attempt, trouble.clone()))
    } else if let Some(failure) = data.capture_failure() {
        state.recording.report(Event::Failed(None, failure))
    } else {
        return;
    };
    spawn(state, effects);
}

/// a panic left the app's state half written: the bench or the capture's data was poisoned by
/// one, or one was caught where no lock shows it. the first one to see it disarms the aim
/// controller, nothing else that moves the mouse or writes evidence may start after it
pub(crate) fn broken(state: &Shared) -> bool {
    let poisoned = state.bench.is_poisoned()
        || state
            .bench
            .locked()
            .capture
            .as_ref()
            .is_some_and(|capture| capture.data.is_poisoned());
    if poisoned && state.recording.broke() {
        disarm_now(state);
    }
    poisoned || state.recording.is_broken()
}

/// the aim controller let go of on a thread of its own, the box may be the thing that's stuck
fn disarm_now(state: &Shared) {
    let aim = state.bench.locked().aim();
    std::thread::spawn(move || {
        let _ = aim.disarm();
    });
    // the recording on its way is saved as failed, the pump's watch ends it
    if let Ok(data) = data(state) {
        let mut data = data.locked();
        data.fail_injection("the aim controller was disarmed because the app broke");
        data.fail(Reason::WriterFailed, "the app broke during this recording");
    }
}

/// a panic caught outside a lock; the app is broken from here on
pub(crate) fn caught(state: &Shared) {
    if state.recording.broke() {
        disarm_now(state);
    }
}

/// effects a report returned, driven beside the caller
fn spawn(state: &Shared, effects: Vec<Effect>) {
    if effects.is_empty() {
        return;
    }
    let state = state.clone();
    std::thread::spawn(move || {
        let ran = catch_unwind(AssertUnwindSafe(|| {
            let driven = drive(&state, effects);
            let _ = announce(&state, driven);
        }));
        if ran.is_err() {
            caught(&state);
        }
    });
}

/// the participants a recording starting now would have
fn parts(bench: &Bench) -> Vec<Part> {
    // the box's input can join after the start, its queue is drained at every stop
    let mut parts = vec![Part::Control, Part::MediusInput];
    if bench.observers.is_some() {
        parts.push(Part::Observers);
    }
    if bench.vision.is_some() {
        parts.push(Part::Vision);
    }
    if bench.aim_armed() {
        parts.push(Part::Injection);
    }
    parts
}

/// the Record button asks for a state, the recording button (`desired` none) only toggles
pub(crate) fn toggle(
    state: &Shared,
    requested: Anchor,
    desired: Option<bool>,
) -> Result<String, String> {
    let press = desired.is_none();
    let (effects, starting) = match ask(state, requested, desired) {
        Ok(asked) => asked,
        Err(error) => {
            if press {
                state.recording.unpress();
            }
            return Err(refused(state, error));
        }
    };
    if starting {
        state.recording_hotkey.update(|s| {
            s.error = None;
            s.message = Some("Starting recording…".into());
        });
    } else {
        state.recording_hotkey.update(|s| s.error = None);
    }
    let driven = drive(state, effects);
    announce(state, driven).map(Option::unwrap_or_default)
}

/// the request a toggle makes; refused before the coordinator is asked, the press is still
/// the caller's to give back
fn ask(
    state: &Shared,
    requested: Anchor,
    desired: Option<bool>,
) -> Result<(Vec<Effect>, bool), String> {
    let press = desired.is_none();
    if broken(state) {
        return Err(BROKEN.into());
    }
    if state.recording.busy() && !press {
        return Err(Refusal::Busy.message().into());
    }
    let recording = state.recording.recording();
    if desired == Some(recording) {
        return Err(if recording {
            Refusal::AlreadyRecording
        } else {
            Refusal::NotRecording
        }
        .message()
        .into());
    }
    let event = if recording {
        // a stop needs only that the app is not closing, the coordinator knows that
        Event::Stop(if press {
            StopReason::HotkeyStop
        } else {
            StopReason::RecordStop
        })
    } else {
        let bench = state.bench.locked();
        crate::app::state::allowed(bench.readiness(false).record)?;
        Event::Start(parts(&bench))
    };
    let starting = !recording;
    let effects = state
        .recording
        .request(event, requested, press)
        .map_err(|refusal| refusal.message().to_string())?;
    Ok((effects, starting))
}

/// stopping usb capture, with the recording it ends; the path of that recording if one ended
pub(crate) fn stop_capture(state: &Shared) -> Result<Option<String>, String> {
    if broken(state) {
        return Err(BROKEN.into());
    }
    if state.bench.locked().capture.is_none() {
        return Err("capture is not running".into());
    }
    let effects = state
        .recording
        .request(Event::Stop(StopReason::CaptureStop), Anchor::take(), false)
        .map_err(|refusal| refusal.message().to_string())?;
    let driven = drive(state, effects);
    announce(state, driven)
}

/// the exit: the recording on its way is committed and capture stopped, then what is left is
/// torn down, all of it within `bound`. what is still stuck after it is left behind for the
/// process to end, and a recording that did not commit in time keeps its marker, so the next
/// start lists it as cut short
pub(crate) fn close(state: &Shared, bound: std::time::Duration) {
    let until = std::time::Instant::now() + bound;
    let closing = state.clone();
    within(until, move || exit(&closing, until));
}

fn exit(state: &Shared, until: std::time::Instant) {
    state
        .shutdown
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let effects = state
        .recording
        .request(Event::Shutdown, Anchor::take(), false)
        .unwrap_or_default();
    // what is stuck gets a third of the exit, the commit needs the rest to save the recording
    let now = std::time::Instant::now();
    let exiting = Driven {
        until: Some(now + until.saturating_duration_since(now) / 3),
        ..Driven::default()
    };
    drive_from(state, effects, exiting);
    while !state.recording.closed() && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let (aim, vision, capture, observer) = {
        let mut bench = state.bench.locked();
        if let Some(job) = bench.jobs.current() {
            let _ = bench.jobs.cancel(job.id);
        }
        (
            bench.take_aim(),
            bench.vision.take(),
            bench.capture.take(),
            bench.observers.take(),
        )
    };
    aim.stop();
    if let Some(mut vision) = vision {
        vision.stop();
    }
    drop(capture);
    if let Some(mut observer) = observer {
        observer.stop();
    }
}

/// runs `effects` and every effect their completions return, in order
pub(crate) fn drive(state: &Shared, effects: Vec<Effect>) -> Driven {
    drive_from(state, effects, Driven::default())
}

fn drive_from(state: &Shared, effects: Vec<Effect>, mut driven: Driven) -> Driven {
    let mut queue = VecDeque::from(effects);
    while let Some(effect) = queue.pop_front() {
        // an effect that panics still completes, or the recording would stay busy for good
        let event = catch_unwind(AssertUnwindSafe(|| run(state, &effect, &mut driven)))
            .unwrap_or_else(|_| {
                caught(state);
                if let Effect::Finish(_, part) = effect {
                    lost(
                        state,
                        part,
                        "it failed unexpectedly while the recording stopped",
                    );
                }
                driven
                    .warnings
                    .push(format!("{effect:?} failed unexpectedly"));
                gave_up(&effect)
            });
        if let Some(event) = event {
            queue.extend(state.recording.report(event));
        }
    }
    driven
}

fn gave_up(effect: &Effect) -> Option<Event> {
    Some(match *effect {
        Effect::Begin(attempt, part) => Event::BeginFailed(
            attempt,
            part,
            Failure::new(Reason::ParticipantFailed, "it failed unexpectedly"),
        ),
        Effect::Finish(attempt, part) => Event::Lost(attempt, part),
        Effect::StopCapture => Event::CaptureStopped,
        Effect::Commit(attempt, _) => Event::Committed(attempt),
        Effect::HandOff(attempt) => Event::HandedOff(attempt),
        Effect::Disarm => return None,
    })
}

fn run(state: &Shared, effect: &Effect, driven: &mut Driven) -> Option<Event> {
    if let Effect::Begin(attempt, _)
    | Effect::Finish(attempt, _)
    | Effect::Commit(attempt, _)
    | Effect::HandOff(attempt) = *effect
    {
        driven.attempt = Some(attempt);
    }
    match *effect {
        Effect::Begin(attempt, part) => Some(match begin(state, part, driven) {
            Ok(()) => Event::Begun(attempt, part),
            Err(error) => {
                if part == Part::Control {
                    driven.warnings.push(error.clone());
                } else {
                    driven.begin_error.get_or_insert(error.clone());
                }
                Event::BeginFailed(
                    attempt,
                    part,
                    Failure::new(Reason::ParticipantFailed, error),
                )
            }
        }),
        Effect::Disarm => {
            let aim = state.bench.locked().aim();
            unlocked(state, None);
            let stuck = aim.clone();
            // an aim controller that took no part leaves nothing in the recording
            if within(until(state, driven), move || aim.disarm()).is_none() {
                retire(state, stuck, "the aim controller didn't stop in time");
            }
            None
        }
        Effect::Finish(attempt, part) => Some(if finish(state, part, driven) {
            Event::Finished(attempt, part)
        } else {
            Event::Lost(attempt, part)
        }),
        Effect::StopCapture => {
            let (capture, observer) = {
                let mut bench = state.bench.locked();
                (bench.capture.take(), bench.observers.take())
            };
            unlocked(state, capture.as_ref().map(|c| &c.data));
            // nothing else can reach a capture taken out of the bench, one that does not stop in
            // time is left to stop on its own
            let stopped = within(until(state, driven), move || {
                let observed = observer.map(|mut observer| {
                    observer.stop();
                    observer.snapshot()
                });
                let health = capture.map(|mut capture| {
                    capture.stop();
                    capture.data.locked().health.clone()
                });
                (observed, health)
            });
            if let Some((observed, health)) = stopped {
                let mut bench = state.bench.locked();
                if let Some(observed) = observed {
                    bench.last_observers = observed;
                }
                if let Some(health) = health {
                    bench.last_health = health;
                }
            }
            Some(Event::CaptureStopped)
        }
        Effect::Commit(attempt, stop) => {
            driven.stop = Some(stop);
            driven.committed = Some(commit(state, stop, driven));
            Some(Event::Committed(attempt))
        }
        Effect::HandOff(attempt) => {
            hand_off(state, driven);
            Some(Event::HandedOff(attempt))
        }
    }
}

fn data(state: &Shared) -> Result<Arc<Mutex<Data>>, String> {
    state
        .recording
        .current()
        .as_ref()
        .and_then(|c| c.data.clone())
        .ok_or_else(|| "no recording is running".into())
}

fn pcap(state: &Shared) -> Result<PathBuf, String> {
    state
        .recording
        .current()
        .as_ref()
        .and_then(|c| c.pcap.clone())
        .ok_or_else(|| "no recording is running".into())
}

fn begin(state: &Shared, part: Part, driven: &mut Driven) -> Result<(), String> {
    match part {
        Part::Usb => {
            // what the files need is read under the locks; they are made with neither held
            let (data, root, info) = {
                let bench = state.bench.locked();
                let capture = bench.capture.as_ref().ok_or("capture is not running")?;
                let firmware = bench.link.snapshot();
                let info = firmware.connected.then(|| {
                    let mut info = firmware.sidecar();
                    info["requested"] = bench.requested.clone();
                    info["health"] = json!({"catch_dropped":firmware.catch_dropped,"input_drops":firmware.input_drops,"link_drops":firmware.link_drops,"host_drops":firmware.host_drops,"tx_drops":firmware.tx_drops,"session":firmware.session,"resets":firmware.resets});
                    info["experiment"] = bench.aim_provenance();
                    info
                });
                (capture.data.clone(), capture.root.clone(), info)
            };
            let (device, decoder) = data.locked().plan(&root, info)?;
            unlocked(state, Some(&data));
            let mut recorder = Recorder::begin(&state.folder, device).map_err(|e| e.to_string())?;
            recorder.decoder = decoder;
            let installed = data.locked().install(recorder);
            let stem = installed.map_err(motion_bench::capture::session::Refused::abandon)?;
            if let Some(current) = state.recording.current().as_mut() {
                current.pcap = Some(state.folder.join(format!("{stem}.pcap")));
                current.data = Some(data);
            }
            driven.stem = Some(stem);
            Ok(())
        }
        Part::MediusInput => {
            // it takes part from the start only while the box's watch is on
            let firmware = state.bench.locked().link.snapshot();
            if firmware.connected && firmware.catching {
                data(state)?.locked().take_part(MEDIUS_INPUT);
            }
            Ok(())
        }
        Part::Observers => {
            let journal = state
                .bench
                .locked()
                .observers
                .as_ref()
                .map(observers::Worker::journal);
            let data = data(state).ok();
            let device = data
                .as_ref()
                .and_then(|data| data.locked().recorder.as_ref().map(|r| r.vid_pid()));
            unlocked(state, data.as_ref());
            let begun = match (journal, device) {
                (None, _) => Ok(()),
                (Some(journal), Some(device)) => journal.begin(&pcap(state)?, device),
                (Some(_), None) => Err("no recording is running".into()),
            };
            failed_to_begin(state, begun, "input observer evidence could not start")
        }
        Part::Vision => {
            let journal = vision_journal(state);
            let Some(journal) = journal else {
                return Ok(());
            };
            unlocked(state, data(state).ok().as_ref());
            // target capture is optional, a model or a GPU that isn't ready never stops the usb
            // recording, which only gets no target events
            if let Err(error) = journal.begin(&pcap(state)?) {
                data(state)?
                    .locked()
                    .degrade(VISION, format!("screen recording could not start: {error}"));
                return Ok(());
            }
            data(state)?.locked().take_part(VISION);
            if let Some(current) = state.recording.current().as_mut() {
                current.vision = true;
            }
            Ok(())
        }
        Part::Injection => {
            let aim = state.bench.locked().aim();
            unlocked(state, data(state).ok().as_ref());
            // it takes part only while it is armed
            let took_part = failed_to_begin(
                state,
                aim.begin(&pcap(state)?),
                "injection evidence could not start",
            )?;
            if took_part {
                data(state)?.locked().take_part(INJECTION);
            }
            Ok(())
        }
        Part::Control => {
            let pcap = pcap(state)?;
            let start = state
                .recording
                .current()
                .as_ref()
                .map(|c| c.start.clone())
                .ok_or("no recording is running")?;
            let control = json!({
                "schema": "motion-bench.recording-control.v1",
                "path": pcap.display().to_string(),
                "complete": false,
                "start_requested": start,
                "start_completed": Anchor::take(),
                "qpc_frequency": motion_bench::capture::clock::frequency(),
                "timestamp_source": "host control boundaries; report timestamps remain USBPcap observations"
            });
            if let Some(current) = state.recording.current().as_mut() {
                current.control = Some(control.clone());
            }
            // the journal is optional evidence, the recording goes on and reads as degraded; the
            // commit writes it whole again
            unlocked(state, data(state).ok().as_ref());
            std::fs::write(
                pcap.with_extension("recording-control.json"),
                serde_json::to_vec_pretty(&control).unwrap(),
            )
            .map_err(|error| {
                let error = format!("the control journal could not be saved at the start: {error}");
                if let Ok(data) = data(state) {
                    data.locked().degrade(RECORDING_CONTROL, &error);
                }
                error
            })
        }
    }
}

/// file work must find the bench and the capture's data free; a test fails when it does not
fn unlocked(state: &Shared, data: Option<&Arc<Mutex<Data>>>) {
    // a poisoned lock is not a held one
    #[cfg(test)]
    fn free<T>(lock: &Mutex<T>) -> bool {
        !matches!(lock.try_lock(), Err(std::sync::TryLockError::WouldBlock))
    }
    #[cfg(test)]
    assert!(
        free(&state.bench) && data.is_none_or(|data| free(data)),
        "file work under a lock"
    );
    #[cfg(not(test))]
    let _ = (state, data);
    #[cfg(test)]
    panics::probe();
}

/// a test can make the nth file-work probe of its thread panic, to see what a panic in each
/// effect leaves behind
#[cfg(test)]
pub(crate) mod panics {
    use std::cell::Cell;
    thread_local! {
        pub static SEEN: Cell<usize> = const { Cell::new(0) };
        pub static AT: Cell<Option<usize>> = const { Cell::new(None) };
    }
    pub fn probe() {
        let n = SEEN.with(|seen| seen.replace(seen.get() + 1));
        if AT.with(Cell::get) == Some(n) {
            panic!("a panic on purpose at probe {n}");
        }
    }
}

/// a participant that could not begin fails the recording with it
fn failed_to_begin<T>(state: &Shared, begun: Result<T, String>, what: &str) -> Result<T, String> {
    begun.inspect_err(|error| {
        if let Ok(data) = data(state) {
            data.locked()
                .fail(Reason::ParticipantFailed, format!("{what}: {error}"));
        }
    })
}

/// what a participant still owes the recording before its input closes; false when it did not
/// finish before the stop's time ran out, it is then lost and its result, if it ever comes, is
/// dropped so it cannot land in a later recording
fn finish(state: &Shared, part: Part, driven: &mut Driven) -> bool {
    let Ok(data) = data(state) else {
        return true;
    };
    let until = until(state, driven);
    match part {
        Part::Injection => {
            let aim = state.bench.locked().aim();
            unlocked(state, Some(&data));
            let journal = aim.clone();
            match within(until, move || aim.end_recording()) {
                Some(Ok(())) => true,
                Some(Err(error)) => {
                    data.locked()
                        .fail_injection(format!("injection evidence failed: {error}"));
                    true
                }
                None => {
                    retire(state, journal, "the aim controller didn't stop in time");
                    lost(state, part, "the aim controller didn't stop in time");
                    false
                }
            }
        }
        Part::Observers => {
            let observer = state.bench.locked().observers.take();
            let Some(mut observer) = observer else {
                return true;
            };
            unlocked(state, Some(&data));
            let Some(observed) = within(until, move || {
                observer.stop();
                observer.snapshot()
            }) else {
                lost(state, part, "the input observers didn't stop in time");
                return false;
            };
            if let Some(error) = &observed.error {
                data.locked().degrade(
                    OBSERVERS,
                    format!("input observer evidence failed: {error}"),
                );
            }
            state.bench.locked().last_observers = observed;
            true
        }
        Part::MediusInput => {
            // waited for with no lock held, the window keeps drawing meanwhile
            let reader = state.bench.locked().link.input_reader();
            let caught_up = reader.caught_up(
                motion_bench::capture::clock::qpc(),
                until.saturating_duration_since(std::time::Instant::now()),
            );
            let bench = state.bench.locked();
            last_box_input(
                &bench.link,
                &mut data.locked(),
                (bench.seen_serial, bench.seen_discontinuities),
                caught_up,
            );
            true
        }
        Part::Vision => {
            let journal = vision_journal(state);
            let Some(journal) = journal else {
                return true;
            };
            unlocked(state, Some(&data));
            match within(until, move || journal.finish()) {
                Some(Ok(())) => true,
                Some(Err(error)) => {
                    data.locked().degrade(
                        VISION,
                        format!("target evidence finalization failed: {error}"),
                    );
                    true
                }
                None => {
                    lost(state, part, "target capture didn't finish in time");
                    false
                }
            }
        }
        Part::Usb | Part::Control => true,
    }
}

/// what a participant that did not finish costs the recording: the injection is the condition
/// under test, so it fails it, the others only degrade it. target capture's file may still be
/// half written, so no events are built from it
fn lost(state: &Shared, part: Part, why: &str) {
    let Ok(data) = data(state) else {
        return;
    };
    let mut data = data.locked();
    match part {
        Part::Injection => data.fail_injection(why),
        Part::Observers => data.degrade(OBSERVERS, why),
        Part::Vision => {
            data.degrade(VISION, why);
            if let Some(current) = state.recording.current().as_mut() {
                current.vision = false;
            }
        }
        Part::MediusInput => data.degrade_pairing(why),
        Part::Usb | Part::Control => {}
    }
}

/// the aim controller disarmed within a stop's time; one the box keeps stuck is retired
pub(crate) fn disarm_within(state: &Shared, aim: crate::app::state::Aim) -> Result<(), String> {
    let stuck = aim.clone();
    let until = std::time::Instant::now() + state.stop_within;
    within(until, move || aim.disarm()).unwrap_or_else(|| {
        retire(state, stuck, "the aim controller didn't stop in time");
        Err("the aim controller didn't stop in time, so it was let go of".into())
    })
}

/// `work` that may wait on hardware, given a stop's time and left behind after it
#[cfg(feature = "developer-tools")]
pub(crate) fn bounded<T: Send + 'static>(
    state: &Shared,
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    within(std::time::Instant::now() + state.stop_within, work)
}

/// an aim controller whose disarm is stuck on the box is let go of: the next arm makes a new
/// one, so the stuck disarm can only ever touch its own journal, which is closed off the driver
fn retire(state: &Shared, aim: crate::app::state::Aim, why: &'static str) {
    state.bench.locked().retire_aim(&aim);
    std::thread::spawn(move || aim.abandon(why));
}

/// when this wait runs out: the first wait of a stop starts its clock for all of them, and each
/// still gets a third of it, so one that is stuck cannot use up a healthy one's time
fn until(state: &Shared, driven: &mut Driven) -> std::time::Instant {
    let now = std::time::Instant::now();
    let stop = *driven.until.get_or_insert(now + state.stop_within);
    stop.max(now + state.stop_within / 3)
}

/// `work` on a thread of its own, its result if it is back by `until`. one that panics is
/// lost at once, one still running afterwards is left to finish and its result is dropped
fn within<T: Send + 'static>(
    until: std::time::Instant,
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (done, result) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(work());
    });
    result
        .recv_timeout(until.saturating_duration_since(std::time::Instant::now()))
        .ok()
}

/// what the box lost on its links, all counters together
pub(crate) fn box_losses(firmware: &medius::Snapshot) -> u64 {
    firmware.catch_dropped
        + firmware.input_drops
        + firmware.link_drops as u64
        + firmware.host_drops as u64
        + firmware.tx_drops as u64
}

/// the box's queued input goes into the recording, and losses or a reset since `seen` (its
/// losses and discontinuities when last looked at) break the pairing of a recording it took
/// part in
pub(crate) fn box_input(
    data: &mut Data,
    motions: impl IntoIterator<Item = Motion>,
    firmware: &medius::Snapshot,
    seen: (u64, u64),
    message: &str,
) {
    for motion in motions {
        data.input(&motion);
    }
    let lost = box_losses(firmware);
    if data.took_part(MEDIUS_INPUT) && (lost > seen.0 || firmware.discontinuities != seen.1) {
        data.health.serial_dropped += lost.saturating_sub(seen.0);
        data.degrade_pairing(message);
    }
}

/// the box's input at the end of a recording. the rows go first, so losses counted while they
/// were read are in the counters read after them
pub(crate) fn last_box_input(
    link: &medius::Link,
    data: &mut Data,
    seen: (u64, u64),
    caught_up: bool,
) {
    let rows: Vec<_> = link.motion.try_iter().collect();
    box_input(
        data,
        rows,
        &link.snapshot(),
        seen,
        "Medius lost reports or reset while capture was shutting down",
    );
    if !caught_up {
        data.degrade_pairing("the box's input was still behind when the recording ended");
    }
}

/// the recording's input closes under the lock, its files are published with no lock held
fn commit(state: &Shared, stop: StopReason, driven: &mut Driven) -> Result<PathBuf, String> {
    let data = data(state)?;
    let (anchor, control) = {
        let mut current = state.recording.current();
        let current = current.as_mut().ok_or("no recording is running")?;
        (
            current.stop.clone().unwrap_or_else(Anchor::take),
            current.control.take(),
        )
    };
    let control = control.map(|mut control| {
        control["stop_requested"] = serde_json::to_value(anchor).unwrap();
        if stop == StopReason::AppExit {
            control["stop_cause"] = "application closed without an explicit finish".into();
        }
        control
    });
    let closing = {
        let mut guard = data.locked();
        // read under the guard: nothing can poison it any more while it is held
        if data.is_poisoned() {
            guard.fail(
                Reason::WriterFailed,
                "a panic left this recording's state half written",
            );
        }
        guard.close()
    };
    let result = match closing {
        Ok(closing) => {
            unlocked(state, Some(&data));
            let (committed, control_error) = commit_recording(closing, stop, control);
            if let Some(error) = control_error {
                driven
                    .warnings
                    .push(format!("control evidence could not be saved: {error}"));
            }
            committed.map_err(|failure| {
                data.locked().fail(failure.reason, &failure.message);
                failure.message
            })
        }
        Err(error) => {
            if let Some(mut control) = control {
                let _ = hotkey::finish_control(
                    &mut control,
                    Some("no recording was running when it stopped"),
                );
            }
            Err(error)
        }
    };
    let health = data.locked().health.clone();
    let mut bench = state.bench.locked();
    bench.last_health = health;
    // the observers run as long as capture does, a recording only borrowed them
    if bench.capture.is_some() && bench.observers.is_none() {
        bench.observers = Some(observers_again());
    }
    result
}

/// the observers started again after a recording that kept capture; tests get one with no
/// threads, reading raw input there would read the test machine's mouse
fn observers_again() -> observers::Worker {
    #[cfg(test)]
    return observers::Worker::with_status(observers::Status {
        raw_ready: true,
        ..Default::default()
    });
    #[cfg(not(test))]
    observers::Worker::start()
}

/// the recording goes into Runs, failed or not, then its target events are built and it is
/// read again with them
fn hand_off(state: &Shared, driven: &mut Driven) {
    let Ok(pcap) = pcap(state) else {
        return;
    };
    let listed = add_to_runs(state, &pcap);
    let vision = state.recording.current().as_ref().is_some_and(|c| c.vision);
    let whole = matches!(driven.committed, Some(Ok(_)));
    if listed.is_ok() && whole && vision {
        state
            .recording_hotkey
            .update(|s| s.message = Some("saved to Runs, now preparing the target events…".into()));
        unlocked(state, None);
        let events = motion_bench::targets::vision::build_events(&pcap);
        let relisted = import(&pcap).and_then(|run| {
            if broken(state) {
                return Err(BROKEN.into());
            }
            state.library().relist(run)?;
            unlocked(state, None);
            Ok(())
        });
        if let Err(error) = relisted {
            driven
                .warnings
                .push(format!("the library could not be updated: {error}"));
        }
        driven.events = Some(events);
    }
    driven.listed = Some(listed);
}

/// lists a saved recording in Runs, replacing an entry for the same file; reading it and saving
/// the library take no lock every command needs
pub(crate) fn add_to_runs(state: &Shared, pcap: &Path) -> Result<(), String> {
    // a broken app's state is half written, nothing of it goes into the library
    if broken(state) {
        return Err(BROKEN.into());
    }
    unlocked(state, None);
    let run = import(pcap)?;
    state.library().add(run)?;
    unlocked(state, None);
    Ok(())
}

#[cfg(test)]
mod tests;

/// target capture's journal, when target capture runs
fn vision_journal(state: &Shared) -> Option<vision::Journal> {
    state
        .bench
        .locked()
        .vision
        .as_ref()
        .map(vision::Worker::journal)
}
