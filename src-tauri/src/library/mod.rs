//! the recordings listed in Runs and the catalog that remembers them across restarts: their
//! names, crops, picked devices, the last result each one was read into, and the ones taken out.
//! a recording is known by its id, the stem the recorder gave its files, so moving the folder or
//! importing it from somewhere else keeps all of it. the catalog only changes when it was written,
//! a change that couldn't be saved isn't kept in memory either

pub mod jobs;

use crate::storage;
use jobs::Jobs;
use motion_bench::analysis::compare::{ResultRow, Selection};
use motion_bench::{
    analysis::{compare, dataset::Dataset},
    capture::{hid::Decoder, session::Device},
    evidence::reader::{
        eligibility::{COMPARISON, PAIRED, eligibility},
        layout::{locate, recording_id},
        recording::open_recording,
        status::{Metadata, Summary, classify, summary},
    },
    sync::LockExt,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Mutex,
};

#[derive(Clone, Serialize)]
pub(crate) struct Run {
    pub(crate) id: String,
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) devices: Vec<Device>,
    pub(crate) selected: Option<String>,
    pub(crate) decoder: Decoder,
    pub(crate) paired: bool,
    pub(crate) panels: Vec<String>,
    pub(crate) missing: BTreeMap<String, String>,
    pub(crate) metadata: Value,
    /// when the recording began, from its metadata or, for one that never finished, its marker
    pub(crate) started_us: Option<u64>,
    /// how the recording ended, the same words for the table and for a refused comparison
    pub(crate) status: Summary,
    /// why each kind of comparison won't read this recording, nothing when it will
    pub(crate) refusals: Refusals,
    pub(crate) result: Option<ResultRow>,
    /// the span every analysis of this recording reads, kept in the library and never in the
    /// recording's own files
    pub(crate) crop: Option<Selection>,
}
#[derive(Clone, Default, Serialize)]
pub(crate) struct Refusals {
    pub(crate) comparison: Option<String>,
    pub(crate) paired: Option<String>,
}

impl Run {
    /// a recording the comparisons refuse offers no panel, wherever one would be added
    pub(crate) fn offer(&mut self, panel: &str) {
        match &self.refusals.comparison {
            Some(why) => {
                self.missing.insert(panel.into(), why.clone());
            }
            None => {
                if !self.panels.iter().any(|p| p == panel) {
                    self.panels.push(panel.into());
                }
            }
        }
    }

    /// a device picked by hand replaces the note asking for one, a refusal still stands
    pub(crate) fn pick(&mut self, device: String, decoder: Decoder) {
        self.selected = Some(device);
        self.decoder = decoder;
        if !matches!(self.decoder, Decoder::Unavailable) {
            self.missing.remove("texture19");
            self.offer("texture19");
        }
    }
}

/// a recording this app made, listed whatever happened to it
/// when a recording that never finished began, from the marker it left
fn marker_started(pcap: &Path) -> Option<u64> {
    let bytes = std::fs::read(pcap.with_extension("active.json")).ok()?;
    serde_json::from_slice::<Value>(&bytes).ok()?["started_us"].as_u64()
}

pub(crate) fn import(path: &Path) -> Result<Run, String> {
    let path = std::path::absolute(path).map_err(|e| e.to_string())?;
    let located = locate(&path)?;
    let path_text = path.display().to_string();
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    // a failed recording is listed with its status rather than refused, its evidence stays
    // viewable and the comparison says why it will not use it. a capture with nothing readable in
    // it is what a crash can leave, it is listed too, with the status the files beside it give it
    let (recording, metadata, status) = match open_recording(&path, None) {
        Ok(recording) => {
            let (metadata, status) = (recording.metadata.clone(), recording.status.clone());
            (Some(recording), metadata, status)
        }
        Err(error) => {
            let why = format!("the capture file cannot be read ({error})");
            let metadata = Metadata::read(&located.sidecar);
            let status = classify(&path, &metadata, Some((None, why)));
            (None, metadata, status)
        }
    };
    let mut run = Run {
        id: located.id(),
        path: path_text,
        name,
        devices: Vec::new(),
        selected: None,
        decoder: Decoder::Unavailable,
        paired: false,
        panels: Vec::new(),
        missing: BTreeMap::new(),
        metadata: json!({}),
        started_us: metadata
            .valid()
            .and_then(|m| m["clock"]["system_us"].as_u64())
            .or_else(|| marker_started(&path)),
        status: summary(&status),
        refusals: {
            let refused = |needs| {
                eligibility(&metadata, &status, &located.streams(), needs)
                    .err()
                    .map(|refusal| refusal.to_string())
            };
            Refusals {
                comparison: refused(COMPARISON),
                paired: refused(PAIRED),
            }
        },
        result: None,
        crop: None,
    };
    let dataset_path = {
        let Some(recording) = recording else {
            run.offer("texture19");
            return Ok(run);
        };
        let located = &recording.located;
        let data = recording.data();
        run.devices = data.devices();
        run.selected = data
            .selected
            .map(|k| format!("{}:{}:{}", k.bus, k.device, k.endpoint));
        if let Some(key) = data.selected {
            run.decoder = data.decoders.get(&key).cloned().unwrap_or_default();
        }
        run.paired = located.input.is_file();
        run.metadata = recording.metadata.valid().cloned().unwrap_or(json!({}));
        if data.selected.is_none()
            && let Some(error) = &recording.selection_error
        {
            run.missing.insert(
                "texture19".into(),
                format!("the saved device is not in this recording ({error}), pick one to compare"),
            );
        } else if !matches!(run.decoder, Decoder::Unavailable) || run.refusals.comparison.is_some()
        {
            run.offer("texture19");
        }
        located.event_dataset.clone()
    };
    if let Some(dataset_path) = dataset_path {
        let dataset = match Dataset::load(&dataset_path) {
            Ok(dataset) => dataset,
            Err(error) => {
                for panel in ["trajectory14", "full49"] {
                    run.missing.insert(
                        panel.into(),
                        format!("target event dataset could not be loaded: {error}"),
                    );
                }
                return Ok(run);
            }
        };
        for panel in ["texture19", "trajectory14", "full49"] {
            match dataset.rows(panel) {
                Ok(_) => run.offer(panel),
                Err(error) => {
                    run.missing.insert(panel.into(), error);
                }
            }
        }
    } else if let Some(why) = &located.stale_events {
        for panel in ["trajectory14", "full49"] {
            run.missing.insert(
                panel.into(),
                format!(
                    "the last build of its target events failed so none of them are scored, {why}"
                ),
            );
        }
    } else if let Some(why) = no_trials(&located.pcap) {
        for panel in ["trajectory14", "full49"] {
            run.missing.insert(panel.into(), why.clone());
        }
    } else {
        run.missing.insert(
            "trajectory14".into(),
            "movement boundaries and count-space target geometry are missing".into(),
        );
        run.missing.insert(
            "full49".into(),
            "target geometry and A to B prefix are missing".into(),
        );
    }
    Ok(run)
}

pub(crate) const CATALOG_SCHEMA: &str = "motion-bench.library.v2";

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    path: String,
    /// the pcap's sha256 when it was listed. a recording never changes once it's saved, so a
    /// file with its name but other bytes is another recording
    sha256: String,
    name: String,
    selected: Option<String>,
    decoder: Decoder,
    /// taken out of Runs, so the folder scan doesn't add it straight back
    removed: bool,
    crop: Option<Selection>,
    /// the id of the last result that read it
    result: Option<String>,
}
impl Entry {
    fn new(path: &Path, sha256: String) -> Self {
        Self {
            path: path.display().to_string(),
            sha256,
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            selected: None,
            decoder: Decoder::Unavailable,
            removed: false,
            crop: None,
            result: None,
        }
    }

    /// what Runs shows of `run` kept, a result put away while the crops disagree keeps its place
    /// so a restart shows it again once they agree
    fn remember(&mut self, run: &Run) {
        self.path.clone_from(&run.path);
        self.name.clone_from(&run.name);
        self.selected.clone_from(&run.selected);
        self.decoder = run.decoder.clone();
        self.removed = false;
        self.crop = run.crop;
        if let Some(result) = &run.result {
            self.result = Some(result.id.clone());
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Catalog {
    schema: String,
    recordings: BTreeMap<String, Entry>,
}

#[derive(Default)]
struct Inner {
    runs: Vec<Run>,
    entries: BTreeMap<String, Entry>,
    /// the catalog on disk was read into `entries`, a change before that would write over it
    read: bool,
    /// what reading the catalog had to say, shown when the runs are restored
    notices: Vec<String>,
    /// why the catalog can't be changed: it couldn't be read and no copy of it could be kept, so
    /// it's left as it is
    frozen: Option<String>,
    /// the runs were read once, a second restore would list removed ones again
    restored: bool,
    /// the catalog on disk couldn't be read and a copy of it is kept, so the next write may
    /// replace it
    replaceable: bool,
    /// counts every change to the runs, so the window reads them again only when they moved
    revision: u64,
}

/// the runs as the window lists them, with the revision they are at. a result comes as its
/// summary, its feature rows are read on their own when it is opened
#[derive(Serialize)]
pub(crate) struct Listed {
    revision: u64,
    runs: Vec<Run>,
}

pub(crate) struct Library {
    folder: PathBuf,
    inner: Mutex<Inner>,
}

impl Library {
    pub(crate) fn new(folder: PathBuf) -> Self {
        Self {
            folder,
            inner: Mutex::default(),
        }
    }

    fn catalog(&self) -> PathBuf {
        self.folder.join("library.json")
    }

    fn results(&self) -> PathBuf {
        self.folder.join("results")
    }

    pub(crate) fn restored(&self) -> bool {
        self.inner.locked().restored
    }

    #[cfg(test)]
    pub(crate) fn runs(&self) -> Vec<Run> {
        self.inner.locked().runs.clone()
    }

    pub(crate) fn run(&self, id: &str) -> Result<Run, String> {
        self.inner
            .locked()
            .runs
            .iter()
            .find(|run| run.id == id)
            .cloned()
            .ok_or("run unavailable".into())
    }

    pub(crate) fn revision(&self) -> u64 {
        self.inner.locked().revision
    }

    pub(crate) fn listed(&self) -> Listed {
        let inner = self.inner.locked();
        let runs = inner
            .runs
            .iter()
            .map(|run| Run {
                id: run.id.clone(),
                path: run.path.clone(),
                name: run.name.clone(),
                devices: run.devices.clone(),
                selected: run.selected.clone(),
                decoder: run.decoder.clone(),
                paired: run.paired,
                panels: run.panels.clone(),
                missing: run.missing.clone(),
                metadata: run.metadata.clone(),
                started_us: run.started_us,
                status: run.status.clone(),
                refusals: run.refusals.clone(),
                result: run.result.as_ref().map(ResultRow::summary),
                crop: run.crop,
            })
            .collect();
        Listed {
            revision: inner.revision,
            runs,
        }
    }

    /// `work` sees the runs while nothing can change them, so a job it starts reads runs that
    /// are still listed
    pub(crate) fn with_runs<T>(&self, work: impl FnOnce(&[Run]) -> T) -> T {
        work(&self.inner.locked().runs)
    }

    /// the catalog on disk read into `inner` before its first change, entries already there win.
    /// a catalog that doesn't read is copied aside first, and when that fails nothing may change
    fn read(&self, inner: &mut Inner) {
        if inner.read {
            return;
        }
        inner.read = true;
        let path = self.catalog();
        let frozen = |why: String| {
            format!(
                "the recording library couldn't be read ({why}), so it's left as it is and changes to Runs aren't saved"
            )
        };
        match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Catalog>(&bytes) {
                Ok(catalog) if catalog.schema == CATALOG_SCHEMA => {
                    for (id, entry) in catalog.recordings {
                        inner.entries.entry(id).or_insert(entry);
                    }
                }
                read => {
                    let why = match read {
                        Ok(catalog) => format!("it's {}, not {CATALOG_SCHEMA}", catalog.schema),
                        Err(error) => error.to_string(),
                    };
                    match storage::backup::preserve(&path, &bytes) {
                        Ok(copy) => {
                            inner.replaceable = true;
                            inner.notices.push(format!(
                            "the recording library couldn't be read ({why}), so recordings in this folder show under their file names, removed ones are back and ones imported from elsewhere are gone until you import them again. the old library is kept as {}",
                            copy.display()
                        ))
                        }
                        Err(error) => {
                            let why = frozen(format!(
                                "{why}, and a copy of it couldn't be kept: {error}"
                            ));
                            inner.notices.push(why.clone());
                            inner.frozen = Some(why);
                        }
                    }
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                let why = frozen(error.to_string());
                inner.notices.push(why.clone());
                inner.frozen = Some(why);
            }
        }
    }

    /// `change` made to copies of the runs and entries, kept only once the catalog with it is
    /// written
    fn change<T>(
        &self,
        change: impl FnOnce(&mut Vec<Run>, &mut BTreeMap<String, Entry>) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut inner = self.inner.locked();
        self.read(&mut inner);
        if let Some(why) = &inner.frozen {
            return Err(why.clone());
        }
        let mut runs = inner.runs.clone();
        let mut entries = inner.entries.clone();
        let out = change(&mut runs, &mut entries)?;
        settle_results(&mut runs);
        for run in &runs {
            if let Some(entry) = entries.get_mut(&run.id) {
                entry.remember(run);
            }
        }
        self.write(&entries)?;
        inner.runs = runs;
        inner.entries = entries;
        inner.revision += 1;
        Ok(out)
    }

    fn write(&self, entries: &BTreeMap<String, Entry>) -> Result<(), String> {
        std::fs::create_dir_all(&self.folder)
            .map_err(|e| format!("couldn't save the recording library: {e}"))?;
        let catalog = Catalog {
            schema: CATALOG_SCHEMA.into(),
            recordings: entries.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&catalog).map_err(|e| e.to_string())?;
        let path = self.catalog();
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, bytes)
            .map_err(|e| format!("couldn't save the recording library: {e}"))?;
        std::fs::rename(temporary, path)
            .map_err(|e| format!("couldn't finish saving the recording library: {e}"))
    }

    fn load_result(&self, id: &str) -> Option<ResultRow> {
        // an id is 16 hex digits, anything else can't name a file of ours
        if id.len() != 16 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let bytes = std::fs::read(self.results().join(format!("{id}.json"))).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// every recording the catalog lists and every one in the folder, read again. a recording
    /// found under another path than the catalog's was moved and keeps everything it had there
    pub(crate) fn restore(&self) -> Vec<String> {
        let (mut entries, mut warnings) = {
            let mut inner = self.inner.locked();
            self.read(&mut inner);
            (inner.entries.clone(), std::mem::take(&mut inner.notices))
        };
        let mut moved = false;
        if let Ok(files) = std::fs::read_dir(&self.folder) {
            for file in files.flatten() {
                let path = file.path();
                // a capture with nothing beside it was not recorded here, and is left out
                if !path.extension().is_some_and(|e| e == "pcap") || locate(&path).is_err() {
                    continue;
                }
                let path = std::path::absolute(&path).unwrap_or(path);
                let text = path.display().to_string();
                match entries.get_mut(&recording_id(&path)) {
                    Some(entry) if entry.path == text => {}
                    Some(entry) => match sha256(&path) {
                        Ok(digest) if digest != entry.sha256 => {
                            warnings.push(another_recording(&path, &entry.path));
                        }
                        // a copy beside the listed one is the same recording, it stays where
                        // it was
                        Ok(_) if Path::new(&entry.path).is_file() => {}
                        Ok(_) => {
                            entry.path = text;
                            moved = true;
                        }
                        Err(error) => warnings.push(error),
                    },
                    None => match sha256(&path) {
                        Ok(digest) => {
                            entries.insert(recording_id(&path), Entry::new(&path, digest));
                        }
                        Err(error) => warnings.push(error),
                    },
                }
            }
        }
        let mut runs = Vec::new();
        for entry in entries.values().filter(|entry| !entry.removed) {
            if let Some(Err(error)) = owed_events(Path::new(&entry.path)) {
                warnings.push(format!("{}: target event export: {error}", entry.name));
            }
            match import(Path::new(&entry.path)) {
                Ok(mut run) => {
                    run.name = entry.name.clone();
                    run.crop = entry.crop;
                    if entry
                        .selected
                        .as_ref()
                        .is_some_and(|id| run.devices.iter().any(|d| &d.id == id))
                    {
                        run.selected = entry.selected.clone();
                    }
                    if !matches!(entry.decoder, Decoder::Unavailable) {
                        run.decoder = entry.decoder.clone();
                        run.offer("texture19");
                    }
                    run.result = entry.result.as_deref().and_then(|id| self.load_result(id));
                    runs.push(run);
                }
                Err(error) => warnings.push(format!("{}: {error}", entry.name)),
            }
        }
        let mut inner = self.inner.locked();
        // a recording the hotkey listed while this read is the newer one
        let meanwhile: Vec<String> = inner.runs.iter().map(|run| run.id.clone()).collect();
        for run in runs {
            if !meanwhile.contains(&run.id) {
                inner.runs.push(run);
            }
        }
        for (id, entry) in entries {
            if !meanwhile.contains(&id) {
                inner.entries.insert(id, entry);
            }
        }
        settle_results(&mut inner.runs);
        inner.restored = true;
        inner.revision += 1;
        // an unreadable catalog is written over at once, it's kept aside, so its notice shows once
        if (moved || inner.replaceable) && inner.frozen.is_none() {
            let entries = inner.entries.clone();
            if let Err(error) = self.write(&entries) {
                warnings.push(format!("the moved recordings show, though {error}"));
            }
        }
        warnings
    }

    /// `run` listed, read again from its files. one already listed or taken out before keeps
    /// the name and crop it had, and a file with its name must hold the same bytes
    pub(crate) fn add(&self, run: Run) -> Result<Run, String> {
        self.put(run, false)
    }

    /// `run` read again after its recording ended, a run that isn't listed stays out
    pub(crate) fn relist(&self, run: Run) -> Result<(), String> {
        self.put(run, true).map(drop)
    }

    fn put(&self, mut run: Run, only_listed: bool) -> Result<Run, String> {
        // the hash reads the whole pcap, so it's taken before the lock
        let digest = sha256(Path::new(&run.path))?;
        self.change(|runs, entries| {
            let listed = runs.iter().position(|listed| listed.id == run.id);
            if only_listed && listed.is_none() {
                return Ok(run.clone());
            }
            match entries.get(&run.id) {
                Some(entry) if entry.sha256 != digest => {
                    return Err(another_recording(Path::new(&run.path), &entry.path));
                }
                Some(entry) => {
                    // a copy of a recording that is still where it was doesn't move it
                    if Path::new(&entry.path).is_file() {
                        run.path.clone_from(&entry.path);
                    }
                    run.name.clone_from(&entry.name);
                    run.crop = entry.crop;
                    if listed.is_none() {
                        run.result = entry.result.as_deref().and_then(|id| self.load_result(id));
                    }
                }
                None => {
                    entries.insert(run.id.clone(), Entry::new(Path::new(&run.path), digest));
                }
            }
            if let Some(index) = listed {
                let previous = runs.remove(index);
                run.name = previous.name;
                run.crop = previous.crop;
                run.result = previous.result;
            }
            runs.push(run.clone());
            Ok(run)
        })
    }

    pub(crate) fn remove(&self, id: &str, jobs: &Jobs) -> Result<(), String> {
        self.change(|runs, entries| {
            if jobs.running() {
                return Err("finish or cancel analysis before deleting a run".into());
            }
            let index = runs
                .iter()
                .position(|run| run.id == id)
                .ok_or("run unavailable")?;
            let run = runs.remove(index);
            if let Some(entry) = entries.get_mut(&run.id) {
                entry.remember(&run);
                entry.removed = true;
            }
            Ok(())
        })
    }

    pub(crate) fn rename(&self, id: &str, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() || name.len() > 160 {
            return Err("use a recording name between 1 and 160 characters".into());
        }
        self.change(|runs, _| {
            find(runs, id)?.name = name.into();
            Ok(())
        })
    }

    /// a running analysis read the crop it started with, its result would describe neither
    pub(crate) fn set_crop(
        &self,
        id: &str,
        crop: Option<Selection>,
        jobs: &Jobs,
    ) -> Result<(), String> {
        self.change(|runs, _| {
            if jobs.running() {
                return Err("finish or cancel analysis before cropping a recording".into());
            }
            find(runs, id)?.crop = crop;
            Ok(())
        })
    }

    pub(crate) fn pick(&self, id: &str, device: String, decoder: Decoder) -> Result<(), String> {
        self.change(|runs, _| {
            find(runs, id)?.pick(device, decoder);
            Ok(())
        })
    }

    /// `result` saved once in the library's results folder, and shown on every candidate it
    /// read. it already has its id
    pub(crate) fn attach(&self, result: &ResultRow, candidates: &[String]) -> Result<(), String> {
        let folder = self.results();
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        compare::write_result(&folder.join(format!("{}.json", result.id)), result)?;
        self.change(|runs, _| {
            for run in runs.iter_mut().filter(|run| candidates.contains(&run.id)) {
                run.result = Some(result.clone());
            }
            Ok(())
        })
    }

    /// the result with this id, whichever run shows it or from its file
    pub(crate) fn result(&self, id: &str) -> Result<ResultRow, String> {
        let shown = self
            .inner
            .locked()
            .runs
            .iter()
            .filter_map(|run| run.result.as_ref())
            .find(|result| result.id == id)
            .cloned();
        shown
            .or_else(|| self.load_result(id))
            .ok_or("this analysis isn't saved anymore, run it again".into())
    }
}

fn find<'a>(runs: &'a mut [Run], id: &str) -> Result<&'a mut Run, String> {
    runs.iter_mut()
        .find(|run| run.id == id)
        .ok_or("run unavailable".into())
}

fn sha256(path: &Path) -> Result<String, String> {
    use sha2::Digest;
    let read = |e: std::io::Error| format!("{} could not be read: {e}", path.display());
    let mut file = std::fs::File::open(path).map_err(read)?;
    let mut hasher = sha2::Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(read)?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn another_recording(other: &Path, listed: &str) -> String {
    format!(
        "{} has the name of {listed} but not the same contents, so it isn't listed. a recording is known by its file name, rename the files of one of them to keep both",
        other.display()
    )
}

/// why target events were built for `pcap` but none of its clicks made a trial, with the reason
/// most of them were left out for
pub(crate) fn no_trials(pcap: &Path) -> Option<String> {
    let bytes = std::fs::read(pcap.with_extension("events.json")).ok()?;
    let events: Value = serde_json::from_slice(&bytes).ok()?;
    let reasons: Vec<String> = events["events"]
        .as_array()?
        .iter()
        .filter_map(|event| event["reason"].as_str())
        // the residual's size is in its reason, so they'd all count apart
        .map(|reason| {
            if reason.starts_with("target/projection residual") {
                "the target on screen didn't follow the mouse's movement, check the sensitivity, FOV and display delay in Target capture".into()
            } else {
                reason.to_string()
            }
        })
        .collect();
    let mut counts = BTreeMap::<&str, usize>::new();
    for reason in &reasons {
        *counts.entry(reason).or_default() += 1;
    }
    let most = counts.iter().max_by_key(|(_, n)| **n);
    Some(match most {
        None => "target capture was on but there's no click in the recording to make a trial from"
            .into(),
        Some((reason, n)) => format!(
            "target capture was on but none of the {} clicks made a trial, {n} of them because {reason}",
            reasons.len()
        ),
    })
}

/// the target events of a recording whose hand-off was skipped because the app closed while it
/// was saved, or whose last build failed: its target capture finished, it was finished whole and
/// it has no events yet or only stale ones. built when the library is read again
fn owed_events(pcap: &Path) -> Option<Result<usize, String>> {
    let read = |extension| {
        std::fs::read(pcap.with_extension(extension))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    };
    let owed = (!pcap.with_extension("events.json").exists()
        || motion_bench::evidence::reader::layout::stale_marker(pcap).exists())
        && read("vision.json").is_some_and(|vision| vision["complete"] == true)
        && read("json").is_some_and(|metadata| {
            metadata["outcome"] == "complete" || metadata["outcome"] == "degraded"
        });
    owed.then(|| motion_bench::targets::vision::build_events(pcap))
}

/// a result read its recordings with the crops they had then, so once one of them is cropped
/// differently the result no longer describes it and is put away. its file stays, it is only
/// shown again while the crops agree
fn settle_results(runs: &mut [Run]) {
    let crops: BTreeMap<String, Option<Selection>> =
        runs.iter().map(|run| (run.id.clone(), run.crop)).collect();
    for run in runs {
        if run.result.as_ref().is_some_and(|result| {
            result
                .crops
                .iter()
                .any(|(id, crop)| crops.get(id).is_some_and(|now| now != crop))
        }) {
            run.result = None;
        }
    }
}

#[cfg(test)]
mod tests;
