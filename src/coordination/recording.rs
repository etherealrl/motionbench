//! one recording's lifecycle as a state machine: which participant begins next, what a stop
//! drains and in what order, when the commit may run and what is left after it. it does no
//! work itself; the desktop carries out its effects and reports back with events
//!
//! other operations that must not run beside a start or a stop (starting capture, target
//! capture, arming and disarming, learning or clearing a binding) hold it with `Claim` and
//! `Release`, so one `busy` covers them all. the teardown after `Closed` (stopping the aim
//! controller and target capture, cancelling an analysis job) belongs to the desktop, nothing
//! of a recording is left by then

use crate::evidence::reason::{Failure, Scope, StopReason};

/// what takes part in a recording, in the order it begins
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Part {
    /// the usb recorder, without which there is no recording; it closes in the commit
    Usb,
    MediusInput,
    Observers,
    /// target capture
    Vision,
    /// the aim controller's injection, the condition under test
    Injection,
    /// the control journal; it is saved in the commit and only degrades the recording
    Control,
}

/// one start, so a late report about an earlier recording cannot end the next one; a later
/// start is a greater attempt
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Attempt(u64);

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// start a recording with these participants beside usb
    Start(Vec<Part>),
    Begun(Attempt, Part),
    /// the failure itself goes into the recording through the desktop, it is here for the record
    BeginFailed(Attempt, Part, Failure),
    /// the Record button, the recording button or stopping capture
    Stop(StopReason),
    /// the recording failed, or with `None` capture failed while no recording was seen; a
    /// `None` only ever stops capture, it never ends a recording
    Failed(Option<Attempt>, Failure),
    Finished(Attempt, Part),
    /// a participant that did not finish in time; the commit goes on without it
    Lost(Attempt, Part),
    CaptureStopped,
    /// the commit ran; its outcome stays with the desktop, a failed one is handed off too
    Committed(Attempt),
    HandedOff(Attempt),
    Claim,
    Release,
    Shutdown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    Begin(Attempt, Part),
    /// disarm an aim controller that took no part in the recording
    Disarm,
    Finish(Attempt, Part),
    /// stops usb capture and its observers; nothing happens when capture is not running
    StopCapture,
    Commit(Attempt, StopReason),
    /// list the recording in Runs and build its target events
    HandOff(Attempt),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    AlreadyRecording,
    NotRecording,
    Busy,
    Closing,
}

impl Refusal {
    /// the words a refused command returns, the snapshot's own for busy and closing
    pub fn message(self) -> &'static str {
        match self {
            Refusal::AlreadyRecording => "already recording",
            Refusal::NotRecording => "not recording",
            Refusal::Busy => crate::coordination::readiness::BUSY,
            Refusal::Closing => crate::coordination::readiness::CLOSING,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaptureStop {
    /// asked for, it runs once the recording no longer needs capture
    Wanted,
    /// asked of the desktop, not stopped yet
    Pending,
}

#[derive(Clone, Debug, Default, PartialEq)]
enum Phase {
    #[default]
    Idle,
    Starting {
        attempt: Attempt,
        /// the first is the one beginning
        todo: Vec<Part>,
        begun: Vec<Part>,
        stop: Option<StopReason>,
    },
    Recording {
        attempt: Attempt,
        begun: Vec<Part>,
    },
    Stopping {
        attempt: Attempt,
        stop: StopReason,
        waiting: Vec<Part>,
    },
    Committing(Attempt),
    HandingOff(Attempt),
    Closed,
}

#[derive(Clone, Debug, Default)]
pub struct Coordinator {
    attempts: u64,
    phase: Phase,
    capture: Option<CaptureStop>,
    closing: bool,
    held: bool,
    /// the recording failed while another operation held it, it ends on `Release`
    failed: bool,
}

impl Coordinator {
    /// a start, a stop or another operation is on its way, or the app is closing
    pub fn busy(&self) -> bool {
        self.held
            || self.closing
            || self.capture.is_some()
            || !matches!(self.phase, Phase::Idle | Phase::Recording { .. })
    }

    /// a recording has begun and no stop is on its way
    pub fn recording(&self) -> bool {
        matches!(self.phase, Phase::Recording { .. })
    }

    /// the app may close: nothing of a recording or of capture is left
    pub fn closed(&self) -> bool {
        self.phase == Phase::Closed
    }

    /// the app is closing or has closed, nothing new may start
    pub fn closing(&self) -> bool {
        self.closing || self.closed()
    }

    pub fn on(&mut self, event: Event) -> Result<Vec<Effect>, Refusal> {
        match event {
            Event::Start(parts) => self.start(parts),
            Event::Stop(reason) => self.stop(reason),
            Event::Claim => {
                self.available()?;
                self.held = true;
                Ok(Vec::new())
            }
            event => Ok(self.report(event)),
        }
    }

    /// what happened, never refused; a report about another attempt is ignored
    fn report(&mut self, event: Event) -> Vec<Effect> {
        if let Some(attempt) = attempt_of(&event)
            && Some(attempt) != self.attempt()
        {
            return Vec::new();
        }
        match event {
            // only the part beginning can be begun or fail to
            Event::Begun(_, part) | Event::BeginFailed(_, part, _)
                if self.beginning() != Some(part) =>
            {
                Vec::new()
            }
            Event::Begun(_, part) => self.begun(part, true),
            Event::BeginFailed(_, part, _) => self.begin_failed(part),
            Event::Failed(attempt, failure) => self.failed(attempt.is_some(), failure),
            Event::Finished(_, part) | Event::Lost(_, part) => {
                if let Phase::Stopping { waiting, .. } = &mut self.phase {
                    waiting.retain(|p| *p != part);
                }
                self.commit()
            }
            Event::CaptureStopped => {
                if self.capture == Some(CaptureStop::Pending) {
                    self.capture = None;
                }
                match self.phase {
                    Phase::Stopping { .. } => self.commit(),
                    _ => self.settle(),
                }
            }
            Event::Committed(attempt) if self.phase == Phase::Committing(attempt) => {
                // nothing is listed while the app closes, it is listed when it opens again
                if self.closing {
                    self.phase = Phase::Idle;
                    self.settle()
                } else {
                    self.phase = Phase::HandingOff(attempt);
                    vec![Effect::HandOff(attempt)]
                }
            }
            Event::HandedOff(attempt) if self.phase == Phase::HandingOff(attempt) => {
                self.phase = Phase::Idle;
                self.settle()
            }
            Event::Release if self.held => {
                self.held = false;
                self.resume()
            }
            Event::Shutdown if !self.closing && self.phase != Phase::Closed => {
                self.closing = true;
                self.want_capture_stopped();
                if self.held {
                    return Vec::new();
                }
                match &mut self.phase {
                    Phase::Starting { stop, .. } => {
                        *stop = Some(merge(*stop, StopReason::AppExit));
                        Vec::new()
                    }
                    _ => self.resume(),
                }
            }
            _ => Vec::new(),
        }
    }

    /// the recording on its way, from its start to its hand-off; a failure seen under the lock
    /// that shows the recording failed is reported with this, so it cannot end a later one
    pub fn attempt(&self) -> Option<Attempt> {
        match &self.phase {
            Phase::Starting { attempt, .. }
            | Phase::Recording { attempt, .. }
            | Phase::Stopping { attempt, .. }
            | Phase::Committing(attempt)
            | Phase::HandingOff(attempt) => Some(*attempt),
            Phase::Idle | Phase::Closed => None,
        }
    }

    fn available(&self) -> Result<(), Refusal> {
        if self.closing || self.phase == Phase::Closed {
            Err(Refusal::Closing)
        } else if self.busy() {
            Err(Refusal::Busy)
        } else {
            Ok(())
        }
    }

    fn start(&mut self, mut todo: Vec<Part>) -> Result<Vec<Effect>, Refusal> {
        if matches!(self.phase, Phase::Recording { .. }) && !self.closing {
            return Err(Refusal::AlreadyRecording);
        }
        self.available()?;
        self.attempts += 1;
        let attempt = Attempt(self.attempts);
        todo.push(Part::Usb);
        todo.sort();
        todo.dedup();
        self.phase = Phase::Starting {
            attempt,
            todo,
            begun: Vec::new(),
            stop: None,
        };
        Ok(vec![Effect::Begin(attempt, Part::Usb)])
    }

    fn stop(&mut self, reason: StopReason) -> Result<Vec<Effect>, Refusal> {
        if self.closing || self.phase == Phase::Closed {
            return Err(Refusal::Closing);
        }
        if self.held {
            return Err(Refusal::Busy);
        }
        if reason == StopReason::CaptureStop {
            if self.phase == Phase::Idle {
                if self.capture.is_some() {
                    return Ok(Vec::new());
                }
                self.capture = Some(CaptureStop::Pending);
                return Ok(vec![Effect::Disarm, Effect::StopCapture]);
            }
            self.want_capture_stopped();
        }
        match &mut self.phase {
            Phase::Idle => Err(Refusal::NotRecording),
            Phase::Starting { stop, .. } => {
                *stop = Some(merge(*stop, reason));
                Ok(Vec::new())
            }
            Phase::Recording { .. } => Ok(self.drain(reason)),
            // a stop on its way keeps its reason; stopping capture runs after it
            _ => Ok(Vec::new()),
        }
    }

    fn beginning(&self) -> Option<Part> {
        match &self.phase {
            Phase::Starting { todo, .. } => todo.first().copied(),
            _ => None,
        }
    }

    fn begun(&mut self, part: Part, took_part: bool) -> Vec<Effect> {
        let Phase::Starting {
            attempt,
            todo,
            begun,
            ..
        } = &mut self.phase
        else {
            return Vec::new();
        };
        todo.remove(0);
        if took_part {
            begun.push(part);
        }
        match todo.first() {
            Some(next) => vec![Effect::Begin(*attempt, *next)],
            None => self.started(),
        }
    }

    /// the start is over, a stop asked for meanwhile runs now
    fn started(&mut self) -> Vec<Effect> {
        let Phase::Starting {
            attempt,
            begun,
            stop,
            ..
        } = std::mem::take(&mut self.phase)
        else {
            return Vec::new();
        };
        self.phase = Phase::Recording { attempt, begun };
        stop.map_or_else(Vec::new, |reason| self.drain(reason))
    }

    fn begin_failed(&mut self, part: Part) -> Vec<Effect> {
        let Phase::Starting { todo, stop, .. } = &mut self.phase else {
            return Vec::new();
        };
        match part {
            // the recorder removes what it made, there is no recording to end; a capture stop
            // asked for meanwhile disarms like one asked for with no recording
            Part::Usb => {
                self.phase = Phase::Idle;
                let disarm = (self.capture == Some(CaptureStop::Wanted) && !self.closing)
                    .then_some(Effect::Disarm);
                disarm.into_iter().chain(self.settle()).collect()
            }
            // a journal that cannot start only degrades the recording
            Part::Control => self.begun(part, false),
            _ => {
                // what has begun is finished as for any stop
                todo.clear();
                *stop = Some(merge(*stop, StopReason::StartFailed));
                self.started()
            }
        }
    }

    /// a failure seen with no recording running only stops capture, when it is capture's own
    fn failed(&mut self, recording: bool, failure: Failure) -> Vec<Effect> {
        if failure.reason.scope() == Scope::Pipeline && self.phase != Phase::Closed {
            self.want_capture_stopped();
        }
        if self.held {
            self.failed |= recording;
            return Vec::new();
        }
        match &mut self.phase {
            _ if !recording => self.settle(),
            Phase::Starting { stop, .. } => {
                *stop = Some(merge(*stop, StopReason::RecordingFailed));
                Vec::new()
            }
            Phase::Recording { .. } => self.drain(StopReason::RecordingFailed),
            _ => Vec::new(),
        }
    }

    /// what was put off while another operation held it
    fn resume(&mut self) -> Vec<Effect> {
        let failed = std::mem::take(&mut self.failed);
        match self.phase {
            Phase::Recording { .. } if failed => self.drain(StopReason::RecordingFailed),
            Phase::Recording { .. } if self.closing => self.drain(StopReason::AppExit),
            Phase::Idle => self.settle(),
            _ => Vec::new(),
        }
    }

    /// the order every stop drains in: the injection's journal first, the observers, then
    /// capture when it stops, then what is still owed by the box and target capture. a stop the
    /// user asked for leaves the aim armed for the next recording, any other disarms it
    fn drain(&mut self, stop: StopReason) -> Vec<Effect> {
        let Phase::Recording { attempt, begun } = &self.phase else {
            return Vec::new();
        };
        let attempt = *attempt;
        let mut effects = Vec::new();
        let mut waiting = Vec::new();
        let mut finish = |part, effects: &mut Vec<Effect>| {
            if begun.contains(&part) {
                effects.push(Effect::Finish(attempt, part));
                waiting.push(part);
            }
        };
        finish(Part::Injection, &mut effects);
        if !matches!(stop, StopReason::RecordStop | StopReason::HotkeyStop) {
            effects.push(Effect::Disarm);
        }
        finish(Part::Observers, &mut effects);
        let stop_capture = self.capture == Some(CaptureStop::Wanted);
        if stop_capture {
            effects.push(Effect::StopCapture);
        }
        finish(Part::MediusInput, &mut effects);
        finish(Part::Vision, &mut effects);
        if stop_capture {
            self.capture = Some(CaptureStop::Pending);
        }
        self.phase = Phase::Stopping {
            attempt,
            stop,
            waiting,
        };
        effects.extend(self.commit());
        effects
    }

    /// once every drained participant is done and capture has stopped, if it was stopping
    fn commit(&mut self) -> Vec<Effect> {
        match &self.phase {
            Phase::Stopping {
                attempt,
                stop,
                waiting,
            } if waiting.is_empty() && self.capture != Some(CaptureStop::Pending) => {
                let effect = Effect::Commit(*attempt, *stop);
                self.phase = Phase::Committing(*attempt);
                vec![effect]
            }
            _ => Vec::new(),
        }
    }

    /// with no recording left: stop capture if that was asked, close if the app is closing
    fn settle(&mut self) -> Vec<Effect> {
        if self.phase != Phase::Idle || self.held {
            return Vec::new();
        }
        match self.capture {
            Some(CaptureStop::Wanted) => {
                self.capture = Some(CaptureStop::Pending);
                vec![Effect::StopCapture]
            }
            Some(CaptureStop::Pending) => Vec::new(),
            None => {
                if self.closing {
                    self.phase = Phase::Closed;
                }
                Vec::new()
            }
        }
    }

    fn want_capture_stopped(&mut self) {
        self.capture.get_or_insert(CaptureStop::Wanted);
    }
}

fn attempt_of(event: &Event) -> Option<Attempt> {
    match event {
        Event::Begun(attempt, _)
        | Event::BeginFailed(attempt, _, _)
        | Event::Finished(attempt, _)
        | Event::Lost(attempt, _)
        | Event::Committed(attempt)
        | Event::HandedOff(attempt)
        | Event::Failed(Some(attempt), _) => Some(*attempt),
        _ => None,
    }
}

/// a failure says more than a stop that was asked for; otherwise the first one stays
fn merge(queued: Option<StopReason>, next: StopReason) -> StopReason {
    let failure = |reason| {
        matches!(
            reason,
            StopReason::StartFailed | StopReason::RecordingFailed
        )
    };
    match queued {
        Some(queued) if failure(queued) || !failure(next) => queued,
        _ => next,
    }
}

#[cfg(test)]
mod tests;
