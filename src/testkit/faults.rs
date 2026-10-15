//! recorder files that fail on cue. nothing makes a real file fail halfway through a recording
//! on windows, so the recorder writes through this in tests

use crate::sync::LockExt;
use std::path::Path;

#[derive(Clone, Copy, Debug)]
pub enum Fault {
    Create,
    /// the file takes this many bytes, then every further write fails
    WriteAfter(u64),
    /// the write that would cross this byte fails once, later writes go through
    FailOnceAt(u64),
    Flush,
    Sync,
    /// publishing over this name fails
    Rename,
    /// removing this file fails
    Remove,
    /// `sync_all` stops at the gate before it syncs, so a test can act while the writer is
    /// stopped in the middle
    Pause(&'static Gate),
    /// every write stops at the gate before it writes, as on a disk that stalls
    HoldWrite(&'static Gate),
}

thread_local! {
    static FAULT: std::cell::Cell<Option<(&'static str, Fault)>> = const { std::cell::Cell::new(None) };
    static LOG: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// the creates, syncs, renames and removes this thread made since the last call, as
/// `"create pcap"`, `"rename json.tmp json"`, so a test can check their order
pub fn take_log() -> Vec<String> {
    LOG.take()
}

fn log(entry: String) {
    LOG.with_borrow_mut(|log| log.push(entry));
}

/// the part of a file name after the stem, `pcap` or `active.json`
fn kind(path: &Path) -> Option<&str> {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.split_once('.'))
        .map(|(_, kind)| kind)
}

fn planned(path: &Path) -> Option<Fault> {
    FAULT
        .get()
        .filter(|(planned, _)| Some(*planned) == kind(path))
        .map(|(_, fault)| fault)
}

pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> std::io::Result<()> {
    let (from, to) = (from.as_ref(), to.as_ref());
    log(format!(
        "rename {} {}",
        kind(from).unwrap_or(""),
        kind(to).unwrap_or("")
    ));
    if matches!(planned(to), Some(Fault::Rename)) {
        return Err(injected_error());
    }
    std::fs::rename(from, to)
}

pub fn remove_file(path: impl AsRef<Path>) -> std::io::Result<()> {
    let path = path.as_ref();
    log(format!("remove {}", kind(path).unwrap_or("")));
    if matches!(planned(path), Some(Fault::Remove)) {
        return Err(injected_error());
    }
    std::fs::remove_file(path)
}

/// makes the recorder file whose name ends in `.{kind}` after the stem fail, for example
/// `"csv"` or `"motion.csv"`, until the returned guard is dropped. stems have no dots, so
/// `"csv"` does not match the motion file. only the calling thread is affected
pub fn inject(kind: &'static str, fault: Fault) -> FaultGuard {
    FAULT.set(Some((kind, fault)));
    FaultGuard
}

pub struct FaultGuard;

impl Drop for FaultGuard {
    fn drop(&mut self) {
        FAULT.set(None);
    }
}

/// where a writer waits for a test. a writer that never gets there makes `reached` time out
/// instead of hanging the test, and a test that never releases lets the writer go after a while
#[derive(Debug)]
pub struct Gate {
    state: std::sync::Mutex<u8>,
    changed: std::sync::Condvar,
}

impl Gate {
    pub const fn new() -> Self {
        Gate {
            state: std::sync::Mutex::new(0),
            changed: std::sync::Condvar::new(),
        }
    }

    /// whether the writer got to the gate within `timeout`
    pub fn reached(&self, timeout: std::time::Duration) -> bool {
        let state = self.state.locked();
        *self
            .changed
            .wait_timeout_while(state, timeout, |s| *s == 0)
            .unwrap()
            .0
            > 0
    }

    pub fn release(&self) {
        *self.state.locked() = 2;
        self.changed.notify_all();
    }

    fn wait(&self) {
        let mut state = self.state.locked();
        *state = (*state).max(1);
        self.changed.notify_all();
        let _ = self
            .changed
            .wait_timeout_while(state, std::time::Duration::from_secs(30), |s| *s < 2)
            .unwrap();
    }
}

impl Default for Gate {
    fn default() -> Self {
        Self::new()
    }
}

fn injected_error() -> std::io::Error {
    std::io::Error::other("injected fault")
}

/// what the recorder writes through in tests: a real file plus the fault, if any, planned for
/// it when it was created
pub struct FaultyFile {
    file: std::fs::File,
    kind: String,
    fault: Option<Fault>,
    written: u64,
    failed_once: bool,
}

impl FaultyFile {
    pub fn create(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref();
        let kind = kind(path).unwrap_or("").to_string();
        log(format!("create {kind}"));
        let fault = planned(path);
        if matches!(fault, Some(Fault::Create)) {
            return Err(injected_error());
        }
        Ok(FaultyFile {
            file: std::fs::File::create(path)?,
            kind,
            fault,
            written: 0,
            failed_once: false,
        })
    }

    pub fn sync_all(&self) -> std::io::Result<()> {
        log(format!("sync {}", self.kind));
        match self.fault {
            Some(Fault::Sync) => return Err(injected_error()),
            Some(Fault::Pause(gate)) => gate.wait(),
            _ => {}
        }
        self.file.sync_all()
    }
}

impl std::io::Write for FaultyFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Some(Fault::HoldWrite(gate)) = self.fault {
            gate.wait();
        }
        if let Some(Fault::FailOnceAt(at)) = self.fault
            && !self.failed_once
            && self.written + buf.len() as u64 > at
        {
            self.failed_once = true;
            return Err(injected_error());
        }
        let allowed = match self.fault {
            Some(Fault::WriteAfter(limit)) => limit.saturating_sub(self.written),
            _ => u64::MAX,
        };
        if allowed == 0 && !buf.is_empty() {
            return Err(injected_error());
        }
        let take = buf.len().min(allowed.try_into().unwrap_or(usize::MAX));
        let count = self.file.write(&buf[..take])?;
        self.written += count as u64;
        Ok(count)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if matches!(self.fault, Some(Fault::Flush)) {
            return Err(injected_error());
        }
        self.file.flush()
    }
}
