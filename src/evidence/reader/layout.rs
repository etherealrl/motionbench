use super::eligibility::Stream;
use std::path::{Path, PathBuf};

/// a recording's files: the pcap with its sidecar, its box input and its target events beside it
pub struct Located {
    pub pcap: PathBuf,
    pub input: PathBuf,
    pub sidecar: PathBuf,
    /// none while the events are stale, so they never enter a score
    pub event_dataset: Option<PathBuf>,
    /// why the last build of the target events failed, when it did. any events from before it
    /// stay on disk but are left out
    pub stale_events: Option<String>,
}
impl Located {
    /// the recording's own id: the stem the recorder gave all its files, which has the start's
    /// microsecond clock in it. it stays the same through a restart, a move or an import, a file
    /// renamed by hand is another recording
    pub fn id(&self) -> String {
        recording_id(&self.pcap)
    }

    /// the evidence saved beside the capture
    pub fn streams(&self) -> Vec<Stream> {
        self.input
            .is_file()
            .then_some(Stream::PhysicalInput)
            .into_iter()
            .collect()
    }
}

/// says the last build of a recording's target events failed and why. any old ones stay on disk
/// to look at, they are only left out of every score until a build succeeds
pub fn stale_marker(pcap: &Path) -> PathBuf {
    pcap.with_extension("events.stale.txt")
}

pub fn recording_id(pcap: &Path) -> String {
    pcap.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// the files of a recording this app made. a capture made anywhere else has neither the metadata
/// nor a marker beside it, and is refused
pub fn locate(path: &Path) -> Result<Located, String> {
    if path.is_dir() {
        return Err("choose the recording's .pcap file, not a folder".into());
    }
    let stale_events = stale_marker(path).exists().then(|| {
        std::fs::read_to_string(stale_marker(path))
            .ok()
            .filter(|why| !why.trim().is_empty())
            .unwrap_or_else(|| "and the reason it saved can't be read".into())
    });
    let located = Located {
        pcap: path.into(),
        input: path.with_extension("input.csv"),
        sidecar: path.with_extension("json"),
        event_dataset: (stale_events.is_none() && path.with_extension("events.npz").is_file())
            .then(|| path.with_extension("events.npz")),
        stale_events,
    };
    if ![
        located.sidecar.clone(),
        path.with_extension("incomplete.json"),
        path.with_extension("active.json"),
    ]
    .iter()
    .any(|beside| beside.exists())
    {
        return Err(
            "this capture was not recorded by Motion Bench, nothing beside it says how it was made"
                .into(),
        );
    }
    Ok(located)
}

#[cfg(test)]
mod tests;
