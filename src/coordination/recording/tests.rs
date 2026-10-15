use super::*;
use crate::evidence::reason::Reason;
use Effect::*;
use Part::*;

const ALL: [Part; 5] = [MediusInput, Observers, Vision, Injection, Control];

fn ok(c: &mut Coordinator, event: Event) -> Vec<Effect> {
    c.on(event.clone())
        .unwrap_or_else(|refusal| panic!("{event:?} refused: {refusal:?}"))
}

fn writer() -> Failure {
    Failure::new(Reason::WriterFailed, "disk full")
}

fn transport() -> Failure {
    Failure::new(Reason::TransportFailed, "usbpcap ended")
}

/// begins every part of a fresh start and returns its attempt
fn recording(c: &mut Coordinator, parts: &[Part]) -> Attempt {
    let [Begin(attempt, Usb)] = ok(c, Event::Start(parts.to_vec()))[..] else {
        panic!("a start begins usb first")
    };
    let mut next = Usb;
    loop {
        match ok(c, Event::Begun(attempt, next))[..] {
            [Begin(_, part)] => next = part,
            [] => return attempt,
            ref other => panic!("{other:?}"),
        }
    }
}

fn finish_all(c: &mut Coordinator, attempt: Attempt, parts: &[Part]) -> Vec<Effect> {
    parts
        .iter()
        .flat_map(|part| ok(c, Event::Finished(attempt, *part)))
        .collect()
}

#[test]
fn a_start_begins_usb_then_each_part_in_order_and_a_stop_drains_like_before() {
    // the Record button: a start, then a stop and the hand-off
    let mut c = Coordinator::default();
    let [Begin(a, Usb)] = ok(
        &mut c,
        Event::Start(vec![Control, Injection, Observers, Vision, MediusInput]),
    )[..] else {
        panic!()
    };
    assert!(c.busy());
    let mut order = vec![Usb];
    for part in [Usb, MediusInput, Observers, Vision, Injection] {
        let [Begin(_, next)] = ok(&mut c, Event::Begun(a, part))[..] else {
            panic!()
        };
        order.push(next);
    }
    assert_eq!(
        order,
        [Usb, MediusInput, Observers, Vision, Injection, Control]
    );
    assert_eq!(ok(&mut c, Event::Begun(a, Control)), []);
    assert!(!c.busy());
    assert_eq!(c.on(Event::Start(vec![])), Err(Refusal::AlreadyRecording));

    assert_eq!(
        ok(&mut c, Event::Stop(StopReason::RecordStop)),
        [
            Finish(a, Injection),
            Finish(a, Observers),
            Finish(a, MediusInput),
            Finish(a, Vision)
        ]
    );
    assert!(c.busy());
    assert_eq!(
        finish_all(&mut c, a, &[Injection, Observers, MediusInput]),
        []
    );
    assert_eq!(
        ok(&mut c, Event::Finished(a, Vision)),
        [Commit(a, StopReason::RecordStop)]
    );
    assert_eq!(ok(&mut c, Event::Committed(a)), [HandOff(a)]);
    assert!(c.busy(), "the hand-off is part of the stop");
    assert_eq!(ok(&mut c, Event::HandedOff(a)), []);
    assert!(!c.busy());
    assert_eq!(c.phase, Phase::Idle);
}

#[test]
fn a_stop_with_no_participants_commits_at_once_and_leaves_the_aim_armed() {
    let mut c = Coordinator::default();
    let a = recording(&mut c, &[]);
    assert_eq!(
        ok(&mut c, Event::Stop(StopReason::HotkeyStop)),
        [Commit(a, StopReason::HotkeyStop)]
    );
}

#[test]
fn every_order_of_finishing_commits_once_after_the_last() {
    let drained = [Injection, Observers, MediusInput, Vision];
    let mut orders = vec![vec![]];
    for _ in 0..drained.len() {
        orders = orders
            .into_iter()
            .flat_map(|order: Vec<Part>| {
                drained
                    .iter()
                    .filter(|p| !order.contains(p))
                    .map(|p| [order.clone(), vec![*p]].concat())
                    .collect::<Vec<_>>()
            })
            .collect();
    }
    assert_eq!(orders.len(), 24);
    for order in orders {
        let mut c = Coordinator::default();
        let a = recording(&mut c, &ALL);
        ok(&mut c, Event::Stop(StopReason::RecordStop));
        let (last, first) = order.split_last().unwrap();
        assert_eq!(finish_all(&mut c, a, first), [], "{order:?}");
        // a second report of the same part does not commit early
        assert_eq!(ok(&mut c, Event::Finished(a, first[0])), [], "{order:?}");
        assert_eq!(
            ok(&mut c, Event::Finished(a, *last)),
            [Commit(a, StopReason::RecordStop)]
        );
    }
}

#[test]
fn stopping_capture_while_recording_stops_it_after_the_observers_and_commits_once_it_stopped() {
    // stop_capture during a recording
    let mut c = Coordinator::default();
    let a = recording(&mut c, &ALL);
    assert_eq!(
        ok(&mut c, Event::Stop(StopReason::CaptureStop)),
        [
            Finish(a, Injection),
            Disarm,
            Finish(a, Observers),
            StopCapture,
            Finish(a, MediusInput),
            Finish(a, Vision)
        ]
    );
    assert_eq!(
        finish_all(&mut c, a, &[Injection, Observers, MediusInput, Vision]),
        []
    );
    assert_eq!(
        ok(&mut c, Event::CaptureStopped),
        [Commit(a, StopReason::CaptureStop)]
    );
    // stopping capture lists the recording too
    assert_eq!(ok(&mut c, Event::Committed(a)), [HandOff(a)]);
    assert_eq!(ok(&mut c, Event::HandedOff(a)), []);
    assert!(!c.busy());
}

#[test]
fn stopping_capture_with_no_recording_disarms_and_stays_busy_until_it_stopped() {
    // stop_capture while idle
    let mut c = Coordinator::default();
    assert_eq!(
        ok(&mut c, Event::Stop(StopReason::CaptureStop)),
        [Disarm, StopCapture]
    );
    assert!(c.busy());
    assert_eq!(c.on(Event::Start(vec![])), Err(Refusal::Busy));
    assert_eq!(c.on(Event::Claim), Err(Refusal::Busy));
    assert_eq!(ok(&mut c, Event::Stop(StopReason::CaptureStop)), []);
    assert_eq!(ok(&mut c, Event::CaptureStopped), []);
    assert!(!c.busy());
    assert_eq!(
        c.on(Event::Stop(StopReason::RecordStop)),
        Err(Refusal::NotRecording)
    );
}

#[test]
fn stopping_capture_during_a_record_stop_keeps_its_reason_and_runs_after_the_hand_off() {
    let mut c = Coordinator::default();
    let a = recording(&mut c, &[Observers]);
    ok(&mut c, Event::Stop(StopReason::RecordStop));
    assert_eq!(ok(&mut c, Event::Stop(StopReason::CaptureStop)), []);
    assert_eq!(ok(&mut c, Event::Stop(StopReason::HotkeyStop)), []);
    assert_eq!(
        ok(&mut c, Event::Finished(a, Observers)),
        [Commit(a, StopReason::RecordStop)]
    );
    assert_eq!(ok(&mut c, Event::Committed(a)), [HandOff(a)]);
    assert_eq!(ok(&mut c, Event::HandedOff(a)), [StopCapture]);
    assert!(c.busy());
    assert_eq!(ok(&mut c, Event::CaptureStopped), []);
    assert!(!c.busy());
}

#[test]
fn a_failed_recording_ends_at_once_and_a_failed_pipeline_also_stops_capture() {
    // a failure the pump saw
    let mut c = Coordinator::default();
    let a = recording(&mut c, &[Observers, Vision]);
    assert_eq!(
        ok(&mut c, Event::Failed(Some(a), writer())),
        [Disarm, Finish(a, Observers), Finish(a, Vision)]
    );
    // a second report of the failure changes nothing
    assert_eq!(ok(&mut c, Event::Failed(Some(a), writer())), []);
    assert_eq!(finish_all(&mut c, a, &[Observers]), []);
    assert_eq!(
        ok(&mut c, Event::Finished(a, Vision)),
        [Commit(a, StopReason::RecordingFailed)]
    );

    let mut c = Coordinator::default();
    let a = recording(&mut c, &[Observers, Vision]);
    assert_eq!(
        ok(&mut c, Event::Failed(Some(a), transport())),
        [Disarm, Finish(a, Observers), StopCapture, Finish(a, Vision)]
    );
    finish_all(&mut c, a, &[Observers, Vision]);
    assert_eq!(
        ok(&mut c, Event::CaptureStopped),
        [Commit(a, StopReason::RecordingFailed)]
    );
}

#[test]
fn a_pipeline_failure_during_a_stop_stops_capture_after_it() {
    let mut c = Coordinator::default();
    let a = recording(&mut c, &[Vision]);
    ok(&mut c, Event::Stop(StopReason::RecordStop));
    assert_eq!(ok(&mut c, Event::Failed(Some(a), transport())), []);
    assert_eq!(
        ok(&mut c, Event::Finished(a, Vision)),
        [Commit(a, StopReason::RecordStop)]
    );
    assert_eq!(ok(&mut c, Event::Committed(a)), [HandOff(a)]);
    assert_eq!(ok(&mut c, Event::HandedOff(a)), [StopCapture]);
}

#[test]
fn capture_failing_with_no_recording_stops_it_and_other_failures_are_left_to_capture() {
    let mut c = Coordinator::default();
    assert_eq!(ok(&mut c, Event::Failed(None, writer())), []);
    assert!(!c.busy());
    assert_eq!(ok(&mut c, Event::Failed(None, transport())), [StopCapture]);
    assert!(c.busy());
}

#[test]
fn a_failed_start_finishes_only_what_began_and_disarms() {
    // a failed begin ends the recording with StartFailed
    let mut c = Coordinator::default();
    let [Begin(a, Usb)] = ok(&mut c, Event::Start(ALL.to_vec()))[..] else {
        panic!()
    };
    ok(&mut c, Event::Begun(a, Usb));
    ok(&mut c, Event::Begun(a, MediusInput));
    assert_eq!(
        ok(&mut c, Event::BeginFailed(a, Observers, writer())),
        [Disarm, Finish(a, MediusInput)]
    );
    assert_eq!(
        ok(&mut c, Event::Finished(a, MediusInput)),
        [Commit(a, StopReason::StartFailed)]
    );

    // an injection that could not begin is disarmed, the rest is finished
    let mut c = Coordinator::default();
    let [Begin(a, Usb)] = ok(&mut c, Event::Start(ALL.to_vec()))[..] else {
        panic!()
    };
    for part in [Usb, MediusInput, Observers, Vision] {
        ok(&mut c, Event::Begun(a, part));
    }
    assert_eq!(
        ok(&mut c, Event::BeginFailed(a, Injection, writer())),
        [
            Disarm,
            Finish(a, Observers),
            Finish(a, MediusInput),
            Finish(a, Vision)
        ]
    );
}

#[test]
fn usb_failing_to_begin_leaves_nothing_to_end() {
    let mut c = Coordinator::default();
    let [Begin(a, Usb)] = ok(&mut c, Event::Start(ALL.to_vec()))[..] else {
        panic!()
    };
    assert_eq!(ok(&mut c, Event::BeginFailed(a, Usb, writer())), []);
    assert!(!c.busy());
    assert_eq!(c.phase, Phase::Idle);
}

#[test]
fn a_journal_that_cannot_begin_does_not_stop_the_recording() {
    let mut c = Coordinator::default();
    let [Begin(a, Usb)] = ok(&mut c, Event::Start(vec![Control]))[..] else {
        panic!()
    };
    assert_eq!(ok(&mut c, Event::Begun(a, Usb)), [Begin(a, Control)]);
    assert_eq!(ok(&mut c, Event::BeginFailed(a, Control, writer())), []);
    assert!(matches!(c.phase, Phase::Recording { .. }));
    // the journal is saved in the commit, a stop does not wait on it
    assert_eq!(
        ok(&mut c, Event::Stop(StopReason::RecordStop)),
        [Commit(a, StopReason::RecordStop)]
    );
}

#[test]
fn a_stop_during_the_start_waits_for_it_and_commits_once() {
    let mut c = Coordinator::default();
    let [Begin(a, Usb)] = ok(&mut c, Event::Start(vec![Vision]))[..] else {
        panic!()
    };
    assert_eq!(ok(&mut c, Event::Stop(StopReason::RecordStop)), []);
    assert_eq!(ok(&mut c, Event::Stop(StopReason::HotkeyStop)), []);
    ok(&mut c, Event::Begun(a, Usb));
    assert_eq!(ok(&mut c, Event::Begun(a, Vision)), [Finish(a, Vision)]);
    assert_eq!(
        ok(&mut c, Event::Finished(a, Vision)),
        [Commit(a, StopReason::RecordStop)]
    );
}

#[test]
fn a_stop_then_a_failed_begin_commits_once_as_a_failed_start() {
    let mut c = Coordinator::default();
    let [Begin(a, Usb)] = ok(&mut c, Event::Start(vec![Vision, Injection]))[..] else {
        panic!()
    };
    ok(&mut c, Event::Stop(StopReason::RecordStop));
    ok(&mut c, Event::Begun(a, Usb));
    assert_eq!(
        ok(&mut c, Event::BeginFailed(a, Vision, writer())),
        [Disarm, Commit(a, StopReason::StartFailed)]
    );
    assert_eq!(c.phase, Phase::Committing(a));
    let commits = [
        ok(&mut c, Event::Committed(a)),
        ok(&mut c, Event::HandedOff(a)),
    ]
    .concat();
    assert_eq!(commits, [HandOff(a)]);
}

#[test]
fn a_capture_stop_queued_during_the_start_survives_a_usb_failure() {
    let mut c = Coordinator::default();
    let [Begin(a, Usb)] = ok(&mut c, Event::Start(vec![]))[..] else {
        panic!()
    };
    ok(&mut c, Event::Stop(StopReason::CaptureStop));
    assert_eq!(
        ok(&mut c, Event::BeginFailed(a, Usb, writer())),
        [Disarm, StopCapture]
    );
    assert!(c.busy());
}

#[test]
fn closing_during_a_stop_still_waiting_keeps_the_stop_reason() {
    let mut c = Coordinator::default();
    let a = recording(&mut c, &[Observers]);
    ok(&mut c, Event::Stop(StopReason::RecordStop));
    assert_eq!(ok(&mut c, Event::Shutdown), []);
    assert_eq!(
        ok(&mut c, Event::Finished(a, Observers)),
        [Commit(a, StopReason::RecordStop)]
    );
    assert_eq!(ok(&mut c, Event::Committed(a)), [StopCapture]);
}

#[test]
fn reports_out_of_order_or_out_of_phase_change_nothing() {
    let mut c = Coordinator::default();
    let [Begin(a, Usb)] = ok(&mut c, Event::Start(vec![Observers, Vision]))[..] else {
        panic!()
    };
    // only the part beginning can be begun or fail to
    for event in [
        Event::Begun(a, Vision),
        Event::BeginFailed(a, Observers, writer()),
        Event::Committed(a),
        Event::HandedOff(a),
        Event::Finished(a, Usb),
        Event::Release,
    ] {
        assert_eq!(ok(&mut c, event), []);
    }
    assert_eq!(ok(&mut c, Event::Begun(a, Usb)), [Begin(a, Observers)]);
    ok(&mut c, Event::Begun(a, Observers));
    ok(&mut c, Event::Begun(a, Vision));
    ok(&mut c, Event::Stop(StopReason::RecordStop));
    // nothing is committed or handed off before it was asked for
    assert_eq!(ok(&mut c, Event::Committed(a)), []);
    assert_eq!(ok(&mut c, Event::HandedOff(a)), []);
    ok(&mut c, Event::Finished(a, Observers));
    assert_eq!(
        ok(&mut c, Event::Finished(a, Vision)),
        [Commit(a, StopReason::RecordStop)]
    );
    assert_eq!(ok(&mut c, Event::HandedOff(a)), []);
    assert_eq!(ok(&mut c, Event::Committed(a)), [HandOff(a)]);
}

#[test]
fn a_failure_during_the_start_ends_it_once_it_has_begun() {
    let mut c = Coordinator::default();
    let [Begin(a, Usb)] = ok(&mut c, Event::Start(vec![Vision]))[..] else {
        panic!()
    };
    ok(&mut c, Event::Stop(StopReason::RecordStop));
    ok(&mut c, Event::Begun(a, Usb));
    assert_eq!(ok(&mut c, Event::Failed(Some(a), transport())), []);
    assert_eq!(
        ok(&mut c, Event::Begun(a, Vision)),
        [Disarm, StopCapture, Finish(a, Vision)]
    );
    ok(&mut c, Event::Finished(a, Vision));
    assert_eq!(
        ok(&mut c, Event::CaptureStopped),
        [Commit(a, StopReason::RecordingFailed)]
    );
}

#[test]
fn a_lost_participant_lets_the_commit_go_and_its_late_finish_is_ignored() {
    let mut c = Coordinator::default();
    let a = recording(&mut c, &[Observers, Vision]);
    ok(&mut c, Event::Stop(StopReason::RecordStop));
    ok(&mut c, Event::Finished(a, Observers));
    assert_eq!(
        ok(&mut c, Event::Lost(a, Vision)),
        [Commit(a, StopReason::RecordStop)]
    );
    assert_eq!(ok(&mut c, Event::Finished(a, Vision)), []);
    assert_eq!(c.phase, Phase::Committing(a));
}

#[test]
fn reports_about_an_earlier_recording_are_ignored() {
    let mut c = Coordinator::default();
    let old = recording(&mut c, &[Vision]);
    ok(&mut c, Event::Stop(StopReason::RecordStop));
    ok(&mut c, Event::Finished(old, Vision));
    ok(&mut c, Event::Committed(old));
    ok(&mut c, Event::HandedOff(old));
    let new = recording(&mut c, &[Vision]);
    assert_ne!(old, new);
    // a failure seen during the old one, reported after the new start
    for event in [
        Event::Failed(Some(old), writer()),
        Event::Finished(old, Vision),
        Event::Committed(old),
        Event::HandedOff(old),
        Event::BeginFailed(old, Vision, writer()),
    ] {
        assert_eq!(ok(&mut c, event), []);
    }
    assert!(matches!(c.phase, Phase::Recording { attempt, .. } if attempt == new));
}

#[test]
fn another_operation_holds_it_and_what_happens_meanwhile_waits_for_its_release() {
    let mut c = Coordinator::default();
    let a = recording(&mut c, &[Vision]);
    ok(&mut c, Event::Claim);
    assert!(c.busy());
    assert_eq!(c.on(Event::Claim), Err(Refusal::Busy));
    assert_eq!(
        c.on(Event::Stop(StopReason::RecordStop)),
        Err(Refusal::Busy)
    );
    assert_eq!(c.on(Event::Start(vec![])), Err(Refusal::AlreadyRecording));
    assert_eq!(ok(&mut c, Event::Failed(Some(a), transport())), []);
    assert_eq!(
        ok(&mut c, Event::Release),
        [Disarm, StopCapture, Finish(a, Vision)]
    );

    let mut c = Coordinator::default();
    ok(&mut c, Event::Claim);
    assert_eq!(c.on(Event::Start(vec![])), Err(Refusal::Busy));
    assert_eq!(ok(&mut c, Event::Failed(None, transport())), []);
    assert_eq!(ok(&mut c, Event::Release), [StopCapture]);
    assert_eq!(ok(&mut c, Event::Release), []);
}

#[test]
fn closing_while_recording_stops_capture_in_the_drain_and_lists_nothing() {
    // the exit handler: the injection journal closes and the aim is disarmed before the
    // observers stop, as on every stop the user didn't ask for, and stopping the aim controller and target capture comes after Closed
    let mut c = Coordinator::default();
    let a = recording(&mut c, &ALL);
    assert_eq!(
        ok(&mut c, Event::Shutdown),
        [
            Finish(a, Injection),
            Disarm,
            Finish(a, Observers),
            StopCapture,
            Finish(a, MediusInput),
            Finish(a, Vision)
        ]
    );
    assert_eq!(ok(&mut c, Event::Shutdown), []);
    assert_eq!(
        c.on(Event::Stop(StopReason::RecordStop)),
        Err(Refusal::Closing)
    );
    finish_all(&mut c, a, &[Injection, Observers, MediusInput, Vision]);
    assert_eq!(
        ok(&mut c, Event::CaptureStopped),
        [Commit(a, StopReason::AppExit)]
    );
    assert_eq!(ok(&mut c, Event::Committed(a)), []);
    assert_eq!(c.phase, Phase::Closed);
    assert_eq!(c.on(Event::Start(vec![])), Err(Refusal::Closing));
    assert_eq!(c.on(Event::Claim), Err(Refusal::Closing));
}

#[test]
fn closing_in_every_other_phase_ends_closed() {
    // idle
    let mut c = Coordinator::default();
    assert_eq!(ok(&mut c, Event::Shutdown), [StopCapture]);
    assert_eq!(ok(&mut c, Event::CaptureStopped), []);
    assert_eq!(c.phase, Phase::Closed);

    // starting: the queued exit outlives a usb failure
    let mut c = Coordinator::default();
    let [Begin(a, Usb)] = ok(&mut c, Event::Start(vec![]))[..] else {
        panic!()
    };
    assert_eq!(ok(&mut c, Event::Shutdown), []);
    assert_eq!(
        ok(&mut c, Event::BeginFailed(a, Usb, writer())),
        [StopCapture]
    );
    ok(&mut c, Event::CaptureStopped);
    assert_eq!(c.phase, Phase::Closed);

    // starting, then it begins: committed as an exit
    let mut c = Coordinator::default();
    let [Begin(a, Usb)] = ok(&mut c, Event::Start(vec![]))[..] else {
        panic!()
    };
    ok(&mut c, Event::Shutdown);
    assert_eq!(ok(&mut c, Event::Begun(a, Usb)), [Disarm, StopCapture]);
    assert_eq!(
        ok(&mut c, Event::CaptureStopped),
        [Commit(a, StopReason::AppExit)]
    );

    // stopping: the stop keeps its reason, no hand-off
    let mut c = Coordinator::default();
    let a = recording(&mut c, &[]);
    ok(&mut c, Event::Stop(StopReason::RecordStop));
    ok(&mut c, Event::Shutdown);
    assert_eq!(ok(&mut c, Event::Committed(a)), [StopCapture]);
    ok(&mut c, Event::CaptureStopped);
    assert_eq!(c.phase, Phase::Closed);

    // handing off: the hand-off finishes first
    let mut c = Coordinator::default();
    let a = recording(&mut c, &[]);
    ok(&mut c, Event::Stop(StopReason::RecordStop));
    ok(&mut c, Event::Committed(a));
    assert_eq!(ok(&mut c, Event::Shutdown), []);
    assert_eq!(ok(&mut c, Event::HandedOff(a)), [StopCapture]);

    // held by another operation: it closes once that lets go
    let mut c = Coordinator::default();
    let a = recording(&mut c, &[]);
    ok(&mut c, Event::Claim);
    assert_eq!(ok(&mut c, Event::Shutdown), []);
    assert_eq!(ok(&mut c, Event::Release), [Disarm, StopCapture]);
    assert_eq!(
        ok(&mut c, Event::CaptureStopped),
        [Commit(a, StopReason::AppExit)]
    );
}

/// every event the outside can send, given the effects still owed
fn moves(owed: &[Effect], attempt: Option<Attempt>) -> Vec<Event> {
    let mut events = vec![
        Event::Start(ALL.to_vec()),
        Event::Stop(StopReason::RecordStop),
        Event::Stop(StopReason::CaptureStop),
        Event::Failed(attempt, writer()),
        Event::Failed(attempt, transport()),
        Event::Shutdown,
        Event::Claim,
        Event::Release,
    ];
    for effect in owed {
        events.extend(match effect {
            Begin(a, part) => vec![
                Event::Begun(*a, *part),
                Event::BeginFailed(*a, *part, writer()),
            ],
            Finish(a, part) => vec![Event::Finished(*a, *part), Event::Lost(*a, *part)],
            StopCapture => vec![Event::CaptureStopped],
            Commit(a, _) => vec![Event::Committed(*a)],
            HandOff(a) => vec![Event::HandedOff(*a)],
            Disarm => vec![],
        });
    }
    events
}

fn answer(effect: &Effect) -> Option<Event> {
    match effect {
        Begin(a, part) => Some(Event::Begun(*a, *part)),
        Finish(a, part) => Some(Event::Finished(*a, *part)),
        StopCapture => Some(Event::CaptureStopped),
        Commit(a, _) => Some(Event::Committed(*a)),
        HandOff(a) => Some(Event::HandedOff(*a)),
        Disarm => None,
    }
}

#[derive(Clone)]
struct World {
    c: Coordinator,
    owed: Vec<Effect>,
    commits: Vec<Attempt>,
    begun: Vec<(Attempt, Part)>,
    finishes: Vec<(Attempt, Part)>,
}

impl World {
    fn new(c: Coordinator, begun: Vec<(Attempt, Part)>) -> Self {
        World {
            c,
            owed: Vec::new(),
            commits: Vec::new(),
            begun,
            finishes: Vec::new(),
        }
    }

    fn send(&mut self, event: &Event) {
        let closed = self.c.phase == Phase::Closed;
        if let Event::Begun(a, part) = event
            && self.owed.contains(&Begin(*a, *part))
        {
            self.begun.push((*a, *part));
        }
        // an answered effect is no longer owed, a failed begin or a loss answers it too
        self.owed.retain(|effect| {
            answer(effect).as_ref() != Some(event)
                && !matches!((effect, event),
                    (Begin(a, p), Event::BeginFailed(b, q, _)) | (Finish(a, p), Event::Lost(b, q))
                        if a == b && p == q)
        });
        if let Ok(effects) = self.c.on(event.clone()) {
            assert!(!closed || effects.is_empty(), "{effects:?} after closing");
            for effect in effects {
                match effect {
                    Commit(a, _) => {
                        assert!(!self.commits.contains(&a), "a second commit for {a:?}");
                        assert!(
                            !self
                                .owed
                                .iter()
                                .any(|e| matches!(e, Finish(..) | StopCapture)),
                            "a commit while {:?} is owed",
                            self.owed
                        );
                        self.commits.push(a);
                    }
                    Finish(a, part) => {
                        assert!(self.begun.contains(&(a, part)), "{part:?} never began");
                        assert!(
                            !self.finishes.contains(&(a, part)),
                            "{part:?} finished twice"
                        );
                        self.finishes.push((a, part));
                    }
                    HandOff(_) => assert!(!self.c.closing, "a hand-off while closing"),
                    _ => {}
                }
                self.owed.push(effect);
            }
        }
        self.owed.retain(|effect| *effect != Disarm);
        assert!(
            self.owed.is_empty() || self.c.busy(),
            "idle while {:?} is owed",
            self.owed
        );
    }

    /// answers what is owed, lets go and stops, until nothing is left on its way
    fn quiesce(mut self) -> Phase {
        for _ in 0..64 {
            if let Some(event) = self.owed.first().and_then(answer) {
                self.send(&event);
            } else if self.c.held {
                self.send(&Event::Release);
            } else if matches!(self.c.phase, Phase::Recording { .. }) {
                self.send(&Event::Stop(StopReason::RecordStop));
            } else {
                assert!(
                    !self.c.busy() || self.c.phase == Phase::Closed,
                    "stuck in {:?}",
                    self.c
                );
                // every recording usb began is committed
                for (a, part) in &self.begun {
                    assert!(
                        *part != Usb || self.commits.contains(a),
                        "{a:?} never committed"
                    );
                }
                return self.c.phase;
            }
        }
        panic!("no end in sight: {:?} owing {:?}", self.c, self.owed)
    }
}

fn explore(world: &World, depth: usize, seen: &mut usize) {
    let end = world.clone().quiesce();
    assert!(matches!(end, Phase::Idle | Phase::Closed), "{end:?}");
    *seen += 1;
    if depth == 0 {
        return;
    }
    for event in moves(&world.owed, world.c.attempt()) {
        let mut next = world.clone();
        next.send(&event);
        explore(&next, depth - 1, seen);
    }
}

#[test]
fn any_six_events_from_idle_or_five_from_a_recording_keep_every_rule_and_can_always_end() {
    let mut seen = 0;
    explore(
        &World::new(Coordinator::default(), Vec::new()),
        6,
        &mut seen,
    );
    assert!(seen > 100_000, "{seen}");
    for parts in [&[][..], &[Observers], &[Vision, Injection], &ALL] {
        let mut c = Coordinator::default();
        let a = recording(&mut c, parts);
        let begun = [&[Usb][..], parts]
            .concat()
            .into_iter()
            .map(|p| (a, p))
            .collect();
        let mut seen = 0;
        explore(&World::new(c, begun), 5, &mut seen);
        assert!(seen > 10_000, "{parts:?}: {seen}");
    }
}
