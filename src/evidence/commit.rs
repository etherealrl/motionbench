//! the recording's terminal metadata has one writer. an active marker goes down before any
//! evidence and comes off only after the metadata is published, so a recording cut off in
//! between still says it never finished

use super::recorder::Sink;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[cfg(test)]
use crate::testkit::faults::{remove_file, rename};
#[cfg(not(test))]
use std::fs::{remove_file, rename};

/// the shape of the metadata this build publishes. a reader refuses a newer one
pub const RECORDING_SCHEMA: &str = "motion-bench.recording.v2";
pub const RECORDING_VERSION: u32 = 2;

/// what can take part in a recording beside the usb capture. the ids are written to disk and
/// read back, so they are spelled out once, here
pub mod participant {
    /// the input observers, which watch the other devices
    pub const OBSERVERS: &str = "observers";
    /// the journal of when the recording was asked to start and stop
    pub const RECORDING_CONTROL: &str = "recording_control";
    /// the box's own view of the movement, on its clock
    pub const MEDIUS_INPUT: &str = "medius_input";
    /// the screen recording for target analyses
    pub const VISION: &str = "vision";
    /// the aim controller's injection, the condition being recorded when it is armed
    pub const INJECTION: &str = "injection";
}

/// something that ran beside the capture, handed to the commit: its id, the error that ended it
/// if one did, and the section the metadata keeps for it under that id
pub struct Participant {
    pub id: &'static str,
    pub error: Option<String>,
    pub section: serde_json::Value,
}

/// the file that says a recording is under way. it names the process and the start, so a
/// reader can tell a recording still running from one a crash left behind
pub struct Marker(PathBuf);

impl Marker {
    /// created and synced before the recording takes any evidence. a marker that could not be
    /// written whole is removed, it belongs to this attempt only
    pub fn create(path: PathBuf, stem: &str, started_us: u64) -> io::Result<Self> {
        let content = serde_json::json!({
            "schema": "motion-bench.active.v1",
            "stem": stem,
            "pid": std::process::id(),
            "started_us": started_us,
        });
        let written = (|| {
            let mut file = Sink::create(&path)?;
            file.write_all(&serde_json::to_vec_pretty(&content)?)?;
            file.flush()?;
            file.sync_all()
        })();
        if let Err(error) = written {
            let _ = remove_file(&path);
            return Err(error);
        }
        Ok(Marker(path))
    }

    /// only once the metadata is published. a marker left beside published metadata loses to it
    pub fn remove(self) -> io::Result<()> {
        remove_file(&self.0)
    }
}

/// writes `value` at `path` whole or not at all: a temporary file beside it, synced, then
/// renamed. the name must not exist yet, published metadata is never written twice, and a
/// rename on windows would replace it
pub fn publish(path: &Path, value: &serde_json::Value) -> io::Result<()> {
    // ponytail: checked then renamed, not atomic against another writer; one owner writes it
    if path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "the recording's metadata is already published",
        ));
    }
    let temporary = path.with_extension("json.tmp");
    let result = (|| {
        let mut file = Sink::create(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(value)?)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = remove_file(&temporary);
    }
    result
}

/// startup files of an attempt that never became a recording. the stem is unique to the
/// attempt, so nothing older can share these names
pub fn discard(paths: &[PathBuf]) {
    for path in paths {
        if path.exists() {
            let _ = remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests;
