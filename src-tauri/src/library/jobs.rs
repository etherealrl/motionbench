//! analysis jobs. one runs at a time, and everything about it, cancelling it, reading its result,
//! is asked for by its id so a newer job is never answered for an older one

use motion_bench::analysis::{compare::ResultRow, error::Error};
use motion_bench::sync::LockExt;
use serde::Serialize;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Running,
    Complete,
    Failed,
    Cancelled,
}

#[derive(Clone, Serialize)]
pub struct Job {
    pub id: u64,
    pub status: Status,
    pub progress: f64,
    pub error: Option<String>,
    pub result: Option<ResultRow>,
}

#[derive(Default)]
struct Table {
    current: Option<Job>,
    cancel: Arc<AtomicBool>,
    last_id: u64,
}

/// the job table has its own lock rather than living in the app state, so a panic that poisons
/// the app state cannot leave a job running forever
#[derive(Clone, Default)]
pub struct Jobs(Arc<Mutex<Table>>);

impl Jobs {
    // the table only holds plain values that each write leaves whole, so a panic elsewhere while
    // it was locked cannot leave it half changed
    fn table(&self) -> MutexGuard<'_, Table> {
        self.0.locked()
    }

    /// the current job without its result, which can be large and is fetched by id instead
    pub fn current(&self) -> Option<Job> {
        self.table().current.as_ref().map(|job| Job {
            id: job.id,
            status: job.status,
            progress: job.progress,
            error: job.error.clone(),
            result: None,
        })
    }

    pub fn running(&self) -> bool {
        self.table()
            .current
            .as_ref()
            .is_some_and(|job| job.status == Status::Running)
    }

    pub fn start(&self) -> Result<(u64, Arc<AtomicBool>), String> {
        let mut table = self.table();
        if table
            .current
            .as_ref()
            .is_some_and(|job| job.status == Status::Running)
        {
            return Err("an analysis is already running".into());
        }
        table.last_id += 1;
        table.cancel = Arc::new(AtomicBool::new(false));
        table.current = Some(Job {
            id: table.last_id,
            status: Status::Running,
            progress: 0.0,
            error: None,
            result: None,
        });
        Ok((table.last_id, table.cancel.clone()))
    }

    pub fn progress(&self, id: u64, progress: f64) {
        if let Some(job) = self.table().current.as_mut().filter(|job| job.id == id) {
            job.progress = progress;
        }
    }

    pub fn cancel(&self, id: u64) -> Result<(), String> {
        let table = self.table();
        match &table.current {
            Some(job) if job.id == id && job.status == Status::Running => {
                table.cancel.store(true, Ordering::Relaxed);
                Ok(())
            }
            // a click that lands just after the job ended has nothing left to stop
            Some(job) if job.id == id => Ok(()),
            _ => Err("this analysis is no longer the current one".into()),
        }
    }

    pub fn result(&self, id: u64) -> Result<ResultRow, String> {
        let table = self.table();
        let job = table
            .current
            .as_ref()
            .filter(|job| job.id == id)
            .ok_or("this analysis is no longer available, a newer one replaced it")?;
        job.result.clone().ok_or_else(|| match job.status {
            Status::Running => "this analysis is still running".into(),
            _ => "this analysis has no result".into(),
        })
    }

    fn finish(&self, id: u64, status: Status, error: Option<String>, result: Option<ResultRow>) {
        if let Some(job) = self.table().current.as_mut().filter(|job| job.id == id) {
            job.status = status;
            job.error = error;
            if status == Status::Complete {
                job.progress = 1.0;
                job.result = result;
            }
        }
    }
}

/// runs `work` on its own thread and records how it ended, a panic included. `publish` sees a
/// complete result before the job reports it and returns a problem to show beside it, if any,
/// and the cancel token only asks the work to stop, whether it did is what the work returns
pub fn spawn(
    jobs: Jobs,
    id: u64,
    work: impl FnOnce(&dyn Fn(f64)) -> Result<ResultRow, Error> + Send + 'static,
    publish: impl FnOnce(&ResultRow) -> Option<String> + Send + 'static,
) -> Option<JoinHandle<()>> {
    let table = jobs.clone();
    let started = std::thread::Builder::new().spawn(move || {
        let progress = |p: f64| jobs.progress(id, p);
        match catch_unwind(AssertUnwindSafe(|| work(&progress))) {
            Ok(Ok(result)) => {
                let problem =
                    catch_unwind(AssertUnwindSafe(|| publish(&result))).unwrap_or_else(|panic| {
                        Some(format!(
                            "the result could not be saved: {}",
                            panic_text(&panic)
                        ))
                    });
                jobs.finish(id, Status::Complete, problem, Some(result));
            }
            Ok(Err(Error::Cancelled)) => jobs.finish(
                id,
                Status::Cancelled,
                Some(Error::Cancelled.to_string()),
                None,
            ),
            Ok(Err(Error::Failed(message))) => jobs.finish(id, Status::Failed, Some(message), None),
            // a panic fails the job even when a cancel was pending, it did not stop as asked
            Err(panic) => jobs.finish(
                id,
                Status::Failed,
                Some(format!(
                    "the analysis stopped unexpectedly: {}",
                    panic_text(&panic)
                )),
                None,
            ),
        }
    });
    match started {
        Ok(handle) => Some(handle),
        Err(error) => {
            let message = format!("the analysis could not start: {error}");
            table.finish(id, Status::Failed, Some(message), None);
            None
        }
    }
}

fn panic_text(panic: &Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no reason given".into())
}

#[cfg(test)]
pub(crate) mod tests;
