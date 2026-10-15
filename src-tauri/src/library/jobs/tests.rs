use super::*;
use motion_bench::analysis::classify::Config;
use std::sync::Barrier;

pub(crate) fn row(panel: &str) -> ResultRow {
    ResultRow {
        id: String::new(),
        schema: "test".into(),
        judge_revision: String::new(),
        implementation: Default::default(),
        tick_us: 1000,
        panel: panel.into(),
        comparison: String::new(),
        config: Config::default(),
        baseline: Vec::new(),
        candidate: Vec::new(),
        sources: Vec::new(),
        feature_names: Vec::new(),
        w1: Vec::new(),
        real_features: Vec::new(),
        fake_features: Vec::new(),
        judge: None,
        judge_unavailable: None,
        idle_dropped: [0, 0],
        alignment_shift_us: None,
        crops: Default::default(),
        not_complete: Vec::new(),
        metadata: serde_json::Value::Null,
    }
}

fn run(
    jobs: &Jobs,
    work: impl FnOnce(&dyn Fn(f64)) -> Result<ResultRow, Error> + Send + 'static,
) -> Job {
    let (id, _) = jobs.start().unwrap();
    spawn(jobs.clone(), id, work, |_| None)
        .unwrap()
        .join()
        .unwrap();
    jobs.current().unwrap()
}

#[test]
fn a_panicking_worker_fails_its_job_and_frees_the_next_one() {
    let jobs = Jobs::default();
    let job = run(&jobs, |_| panic!("boom"));
    assert_eq!(job.status, Status::Failed);
    assert!(job.error.unwrap().contains("boom"));
    assert!(!jobs.running());
    assert_eq!(run(&jobs, |_| Ok(row("next"))).status, Status::Complete);
}

#[test]
fn only_the_cancelled_variant_makes_a_job_cancelled() {
    let jobs = Jobs::default();
    // the cancellation wording as a plain failure is a failure
    let job = run(&jobs, |_| Err(Error::Failed("analysis cancelled".into())));
    assert_eq!(job.status, Status::Failed);
    let job = run(&jobs, |_| Err(Error::Cancelled));
    assert_eq!(job.status, Status::Cancelled);

    // asking to stop is only a request, whatever the work returns decides
    for (outcome, expected) in [
        (Err(Error::from("boom")), Status::Failed),
        (Ok(row("done")), Status::Complete),
    ] {
        let (id, _) = jobs.start().unwrap();
        jobs.cancel(id).unwrap();
        spawn(jobs.clone(), id, move |_| outcome, |_| None)
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(jobs.current().unwrap().status, expected);
    }
}

#[test]
fn cancel_and_result_answer_only_for_their_own_job() {
    let jobs = Jobs::default();
    let first = run(&jobs, |_| Ok(row("first"))).id;
    assert_eq!(jobs.result(first).unwrap().panel, "first");

    let gate = Arc::new(Barrier::new(2));
    let (second, cancel) = jobs.start().unwrap();
    let wait = gate.clone();
    let worker = spawn(
        jobs.clone(),
        second,
        move |progress| {
            progress(0.5);
            wait.wait();
            Ok(row("second"))
        },
        |_| None,
    );
    assert!(jobs.cancel(first).is_err());
    assert!(!cancel.load(Ordering::Relaxed));
    assert!(
        jobs.result(first)
            .err()
            .unwrap()
            .contains("no longer available")
    );
    assert!(jobs.result(second).err().unwrap().contains("still running"));
    assert!(jobs.start().is_err());
    gate.wait();
    worker.unwrap().join().unwrap();
    assert_eq!(jobs.result(second).unwrap().panel, "second");
    // a late click on a finished job has nothing to stop and is not an error
    assert!(jobs.cancel(second).is_ok());
}

#[test]
fn publishing_runs_once_for_a_complete_result_and_its_problem_is_shown() {
    let jobs = Jobs::default();
    let (id, _) = jobs.start().unwrap();
    spawn(
        jobs.clone(),
        id,
        |_| Ok(row("saved")),
        |result| Some(format!("could not save {}", result.panel)),
    )
    .unwrap()
    .join()
    .unwrap();
    let job = jobs.current().unwrap();
    assert_eq!(job.status, Status::Complete);
    assert_eq!(job.error.as_deref(), Some("could not save saved"));
    assert!(
        job.result.is_none(),
        "the snapshot copy leaves the result out"
    );

    let (id, _) = jobs.start().unwrap();
    spawn(
        jobs.clone(),
        id,
        |_| Err("failed".into()),
        |_| panic!("publish must not run for a failed job"),
    )
    .unwrap()
    .join()
    .unwrap();
    assert_eq!(jobs.current().unwrap().status, Status::Failed);
}

#[test]
fn a_panic_while_publishing_still_completes_the_job() {
    let jobs = Jobs::default();
    let (id, _) = jobs.start().unwrap();
    spawn(
        jobs.clone(),
        id,
        |_| Ok(row("kept")),
        |_| panic!("disk gone"),
    )
    .unwrap()
    .join()
    .unwrap();
    let job = jobs.current().unwrap();
    assert_eq!(job.status, Status::Complete);
    assert!(job.error.unwrap().contains("could not be saved: disk gone"));
    assert_eq!(jobs.result(id).unwrap().panel, "kept");
}

#[test]
fn a_panic_fails_the_job_even_with_a_cancel_pending() {
    let jobs = Jobs::default();
    let (id, _) = jobs.start().unwrap();
    jobs.cancel(id).unwrap();
    spawn(jobs.clone(), id, |_| panic!("boom"), |_| None)
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(jobs.current().unwrap().status, Status::Failed);
}

#[test]
fn a_stale_completion_leaves_the_newer_job_alone() {
    let jobs = Jobs::default();
    let old = run(&jobs, |_| Ok(row("old"))).id;
    let (new, _) = jobs.start().unwrap();
    jobs.finish(old, Status::Failed, Some("late".into()), None);
    jobs.progress(old, 0.9);
    let job = jobs.current().unwrap();
    assert_eq!(
        (job.id, job.status, job.progress),
        (new, Status::Running, 0.0)
    );
    assert_eq!(job.error, None);
}
