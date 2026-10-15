use crate::analysis::{
    classify::{self, Config, Report},
    dataset::Dataset,
    error::{Error, check, check_io},
    texture,
};
use crate::{
    capture::hid::Decoder,
    evidence::{
        reader::{
            eligibility::{COMPARISON, PAIRED, eligibility},
            layout::{Located, locate, recording_id},
            recording::{Cancellable, Recording, open_recording_until},
            status::{Summary, assess, summary},
        },
        recorder,
    },
    signal::{stats::View, ticks::tick},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

/// a span of a recording on the pcap clock: absolute USBPcap microseconds, the `t_us` of reports
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Selection {
    pub from_us: u64,
    pub to_us: u64,
}
impl Selection {
    pub fn contains(&self, t_us: u64) -> bool {
        (self.from_us..=self.to_us).contains(&t_us)
    }

    /// `seconds` after `first`, the selected device's first report inside the crop, on the pcap
    /// clock and never past the crop's end, so a range typed or brushed past it reads nothing
    /// beyond it. every range a person picks goes through here
    pub fn from_seconds(
        first: u64,
        seconds: [f64; 2],
        crop: Option<Selection>,
    ) -> Result<Self, Error> {
        let [from, to] = seconds;
        if !from.is_finite() || !to.is_finite() || from < 0.0 || to <= from {
            return Err("that time range isn't valid".into());
        }
        let end = crop.map_or(u64::MAX, |c| c.to_us);
        let at = |seconds: f64| first.saturating_add((seconds * 1e6) as u64).min(end);
        Ok(Selection {
            from_us: at(from),
            to_us: at(to),
        })
    }
}

/// how wide a texture window can be, in ticks
pub const WIDTH_MS: std::ops::RangeInclusive<usize> = 8..=1000;
/// paired windows go to the judge this many to a group, so a fold holds whole stretches of
/// movement rather than windows torn from their neighbours
const PAIRED_GROUP_WINDOWS: usize = 8;
pub struct Window {
    pub index: usize,
    pub row: Vec<f64>,
}
/// the full windows of `count` ticks from `origin`, a tail shorter than a window dropped
pub fn windows(
    view: &View<'_>,
    origin: u64,
    count: usize,
    width: usize,
    cancel: &AtomicBool,
) -> Result<(Vec<Window>, usize), Error> {
    if width == 0 {
        return Err("window width must be positive".into());
    }
    let mut collector = WindowCollector::counted(origin, count, width, cancel);
    for (time, x, y) in view.motion() {
        collector.push(time, x, y)?;
    }
    Ok(collector.finish())
}

struct WindowCollector<'a> {
    origin: u64,
    count: usize,
    width: usize,
    current: Option<usize>,
    ticks: Vec<[i32; 2]>,
    rows: Vec<Window>,
    cancel: &'a AtomicBool,
}
impl<'a> WindowCollector<'a> {
    /// the ticks from `origin` up to `to`, the tick `to` itself falls in left out
    fn new(origin: u64, to: u64, width: usize, cancel: &'a AtomicBool) -> Self {
        Self::counted(origin, tick(origin, to).unwrap_or(0), width, cancel)
    }
    fn counted(origin: u64, count: usize, width: usize, cancel: &'a AtomicBool) -> Self {
        Self {
            origin,
            count,
            width,
            current: None,
            ticks: vec![[0; 2]; width],
            rows: Vec::new(),
            cancel,
        }
    }
    fn flush(&mut self) {
        if let Some(index) = self.current
            && index < self.count / self.width
            && !texture::is_idle(&self.ticks)
        {
            self.rows.push(Window {
                index,
                row: texture::features(&self.ticks).to_vec(),
            });
        }
        self.ticks.fill([0; 2]);
    }
    fn push(&mut self, t: u64, x: i32, y: i32) -> Result<(), Error> {
        check(self.cancel)?;
        let Some(tick) = tick(self.origin, t).filter(|tick| *tick < self.count) else {
            return Ok(());
        };
        let index = tick / self.width;
        if self.current != Some(index) {
            self.flush();
            self.current = Some(index);
        }
        let slot = &mut self.ticks[tick % self.width];
        slot[0] = slot[0].checked_add(x).ok_or("count stream overflow")?;
        slot[1] = slot[1].checked_add(y).ok_or("count stream overflow")?;
        Ok(())
    }
    fn finish(mut self) -> (Vec<Window>, usize) {
        self.flush();
        let idle = (self.count / self.width).saturating_sub(self.rows.len());
        (self.rows, idle)
    }
}
struct Wire {
    pcap: PathBuf,
    key: crate::signal::stats::Key,
    decoder: Decoder,
    /// the selected device's first and last report inside the crop
    first: u64,
    last: u64,
    reports: u64,
    /// when the selected device first went away, if it did
    disconnected_at: Option<u64>,
}
impl Wire {
    /// a range that ends before the selected device went away is whole, one that reaches the
    /// unplug or starts after it is not
    fn covers(&self, from: u64, to: u64) -> Result<(), Error> {
        match self.disconnected_at {
            Some(t) if t < from => {
                Err("the selected device disconnected before the selected range".into())
            }
            Some(t) if t <= to => Err(format!(
                "the selected device disconnected {} ms into the selected range",
                (t - from) / 1000
            )
            .into()),
            _ => Ok(()),
        }
    }

    /// `range` in seconds from the first report, or every report, inside the crop
    fn span(&self, range: Option<[f64; 2]>, crop: Option<Selection>) -> Result<(u64, u64), Error> {
        let Some(range) = range else {
            return Ok((self.first, self.last));
        };
        let span = Selection::from_seconds(self.first, range, crop)?;
        Ok((span.from_us, span.to_us))
    }
}

/// a source's recording read the way Runs reads it, its device picked the same way
fn open_wire(source: &Source, pcap: &Path, cancel: &AtomicBool) -> Result<Wire, Error> {
    let recording = open_recording_until(pcap, source.decoder.clone(), cancel);
    check(cancel)?;
    let mut recording = recording?;
    if let Some(endpoint) = &source.endpoint {
        recording.select(endpoint)?;
    }
    wire(&recording, source.crop)
}

/// the selected device of a recording, only its reports inside `crop` when there is one
fn wire(recording: &Recording, crop: Option<Selection>) -> Result<Wire, Error> {
    if let Some(why) = &recording.cut_short {
        return Err(why.clone().into());
    }
    let data = recording.data();
    // capture's own failure, or the selected device's clock going back, outside the crop too,
    // since the crop is placed on that clock
    if let Some((_, error)) = data.read_error() {
        return Err(error.into());
    }
    let key = data.selected.ok_or_else(|| {
        recording.selection_error.clone().unwrap_or_else(|| {
            "recording contains multiple endpoints, select one before judging".into()
        })
    })?;
    // another device took the address, so the reports under this key are not one device's
    if recording.reused.contains(&(key.bus, key.device)) {
        return Err(
            "another device took the selected device's address during the recording".into(),
        );
    }
    let decoder = data.decoders.get(&key).cloned().unwrap_or_default();
    if matches!(decoder, Decoder::Unavailable) {
        return Err(
            "this recording has report timing but no movement layout for its mouse, so its movement cannot be decoded"
                .into(),
        );
    }
    let (from, to) = crop.map_or((0, u64::MAX), |c| (c.from_us, c.to_us));
    let stream = &data.bus.streams[&key];
    let inside = stream.copy(from, to);
    let (Some(first), Some(last)) = (inside.first_us(), inside.last_us()) else {
        return Err("the crop holds no reports of the recording's mouse".into());
    };
    Ok(Wire {
        pcap: recording.located.pcap.clone(),
        key,
        decoder,
        first,
        last,
        reports: stream.view(from, to).reports() as u64,
        disconnected_at: recording.disconnected.get(&(key.bus, key.device)).copied(),
    })
}

/// the selected device's windows from `from` to `to`, decoded from the capture's own bytes with
/// the one decoder the recording settled on
fn wire_windows(
    wire: &Wire,
    from: u64,
    to: u64,
    width: usize,
    cancel: &AtomicBool,
) -> Result<(Vec<Window>, usize), Error> {
    if to <= from {
        return Err("analysis range is empty".into());
    }
    let mut collector = WindowCollector::new(from, to, width, cancel);
    let mut error = None;
    crate::capture::pcap::read_pcap(
        std::io::BufReader::new(Cancellable {
            reader: std::fs::File::open(&wire.pcap).map_err(|e| e.to_string())?,
            cancel,
        }),
        |record| {
            if error.is_some() {
                return;
            }
            if let crate::capture::pcap::Record::Packet {
                event: Some(crate::capture::pcap::Event::Report(report)),
                ..
            } = record
                && (report.bus, report.device, report.endpoint)
                    == (wire.key.bus, wire.key.device, wire.key.endpoint)
                && let Some((x, y)) = wire.decoder.decode(report.bytes())
                && let Err(e) = collector.push(report.t_us, x, y)
            {
                error = Some(e);
            }
        },
    )
    .map_err(|e| {
        if check(cancel).is_err() {
            Error::Cancelled
        } else {
            Error::from(e)
        }
    })?;
    if let Some(error) = error {
        return Err(error);
    }
    Ok(collector.finish())
}
/// a recording a result read that didn't end complete. it stays with the result, since how the
/// recording ended bounds what the number can say
#[derive(Clone, Serialize, Deserialize)]
pub struct NotComplete {
    pub id: String,
    pub path: String,
    pub kind: String,
    pub message: Option<String>,
}
impl NotComplete {
    fn of(located: &Located, status: &Summary) -> Option<Self> {
        (status.kind != "complete").then(|| Self {
            id: located.id(),
            path: located.pcap.display().to_string(),
            kind: status.kind.into(),
            message: status.message.clone(),
        })
    }
}

/// a recording a result read, and the sha256 of every file of it that was read, by extension.
/// a digest says the bytes are the same, not that two recordings were separate experiments
#[derive(Clone, Serialize, Deserialize)]
pub struct SourceRecord {
    pub id: String,
    pub path: String,
    pub digests: BTreeMap<String, String>,
}
impl SourceRecord {
    fn read(located: &Located, files: &[&Path], cancel: &AtomicBool) -> Result<Self, Error> {
        let pcap = located.pcap.display().to_string();
        let mut digests = BTreeMap::new();
        for file in files {
            let name = file.display().to_string();
            let extension = name
                .strip_prefix(pcap.trim_end_matches("pcap"))
                .unwrap_or(&name);
            digests.insert(extension.to_string(), sha256(file, cancel)?);
        }
        Ok(Self {
            id: located.id(),
            path: pcap,
            digests,
        })
    }
}

fn sha256(path: &Path, cancel: &AtomicBool) -> Result<String, Error> {
    use sha2::Digest;
    use std::io::Read;
    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("{} could not be read: {e}", path.display()))?;
    let mut hasher = sha2::Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        check(cancel)?;
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("{} could not be read: {e}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// which code made a result: the app's version and commit, the upstream reference is
/// `judge_revision`
#[derive(Clone, Serialize, Deserialize)]
pub struct Implementation {
    pub app_version: String,
    pub revision: String,
}
impl Default for Implementation {
    fn default() -> Self {
        Self {
            app_version: env!("CARGO_PKG_VERSION").into(),
            revision: env!("MOTION_BENCH_REVISION").into(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ResultRow {
    /// the start of the sha256 of the result saved without it, empty until it's saved
    pub id: String,
    pub schema: String,
    pub judge_revision: String,
    pub implementation: Implementation,
    /// the method's sampling grid, not the device's report period
    pub tick_us: u64,
    pub panel: String,
    pub comparison: String,
    pub config: Config,
    /// recording ids, each one is in `sources` with its path and digests
    pub baseline: Vec<String>,
    pub candidate: Vec<String>,
    pub sources: Vec<SourceRecord>,
    pub feature_names: Vec<String>,
    pub w1: Vec<f64>,
    pub real_features: Vec<Vec<f64>>,
    pub fake_features: Vec<Vec<f64>>,
    pub judge: Option<Report>,
    pub judge_unavailable: Option<String>,
    pub idle_dropped: [usize; 2],
    pub alignment_shift_us: Option<i64>,
    /// the crop each recording it read had then, by its id; a result stops describing a
    /// recording whose crop changed since
    pub crops: BTreeMap<String, Option<Selection>>,
    pub not_complete: Vec<NotComplete>,
    pub metadata: serde_json::Value,
}
impl ResultRow {
    /// gives the result its id, from everything else in it, so the same result always gets the
    /// same one
    pub fn identify(&mut self) {
        use sha2::Digest;
        self.id.clear();
        let bytes = serde_json::to_vec(self).expect("a result always serializes");
        self.id = format!("{:x}", sha2::Sha256::digest(bytes))[..16].into();
    }

    pub fn summary(&self) -> Self {
        Self {
            id: self.id.clone(),
            schema: self.schema.clone(),
            judge_revision: self.judge_revision.clone(),
            implementation: self.implementation.clone(),
            tick_us: self.tick_us,
            panel: self.panel.clone(),
            comparison: self.comparison.clone(),
            config: self.config.clone(),
            baseline: self.baseline.clone(),
            candidate: self.candidate.clone(),
            sources: self.sources.clone(),
            feature_names: self.feature_names.clone(),
            w1: self.w1.clone(),
            real_features: Vec::new(),
            fake_features: Vec::new(),
            judge: self.judge.as_ref().map(|j| Report {
                permutation_error: j.permutation_error.clone(),
                permutation_method: j.permutation_method.clone(),
                auc: j.auc,
                auc_ci: j.auc_ci,
                repeat_auc: j.repeat_auc.clone(),
                fold_auc: j.fold_auc.clone(),
                permutation_auc: Vec::new(),
                p_value: j.p_value,
                smallest_p: j.smallest_p,
                null_p95: j.null_p95,
                n_real: j.n_real,
                n_fake: j.n_fake,
                n_groups: j.n_groups,
            }),
            judge_unavailable: self.judge_unavailable.clone(),
            idle_dropped: self.idle_dropped,
            alignment_shift_us: self.alignment_shift_us,
            crops: self.crops.clone(),
            not_complete: self.not_complete.clone(),
            metadata: serde_json::Value::Object(
                self.metadata
                    .as_object()
                    .map(|map| {
                        map.iter()
                            .filter(|(key, _)| key.as_str() != "source_groups")
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect()
                    })
                    .unwrap_or_default(),
            ),
        }
    }
}
pub struct Population {
    pub rows: Vec<Vec<f64>>,
    pub groups: Vec<String>,
    pub idle: usize,
}
pub fn finish(
    real: Population,
    fake: Population,
    names: Vec<String>,
    config: Config,
    cancel: &AtomicBool,
    progress: impl Fn(f64),
) -> Result<ResultRow, Error> {
    if real.rows.is_empty() || fake.rows.is_empty() {
        return Err("no active analysis windows in one population".into());
    }
    let w1 = texture::w1(&real.rows, &fake.rows);
    let (judge, unavailable) = match classify::run(
        &real.rows,
        &fake.rows,
        &real.groups,
        &fake.groups,
        &config,
        cancel,
        progress,
    ) {
        Ok(r) => (Some(r), None),
        Err(Error::Cancelled) => return Err(Error::Cancelled),
        Err(e) => (None, Some(e.to_string())),
    };
    Ok(ResultRow {
        id: String::new(),
        schema: "motion-bench.comparison.v4".into(),
        judge_revision: classify::REVISION.into(),
        implementation: Implementation::default(),
        tick_us: crate::signal::ticks::TICK_US,
        panel: "texture19".into(),
        comparison: "independent populations".into(),
        config,
        baseline: Vec::new(),
        candidate: Vec::new(),
        sources: Vec::new(),
        feature_names: names,
        w1,
        real_features: real.rows,
        fake_features: fake.rows,
        judge,
        judge_unavailable: unavailable,
        idle_dropped: [real.idle, fake.idle],
        alignment_shift_us: None,
        crops: BTreeMap::new(),
        not_complete: Vec::new(),
        metadata: serde_json::json!({"source_groups":{"baseline":real.groups,"candidate":fake.groups}}),
    })
}

pub fn paired(
    path: &Path,
    decoder: Option<Decoder>,
    config: Config,
    range: Option<[f64; 2]>,
    cancel: &AtomicBool,
    progress: impl Fn(f64),
) -> Result<ResultRow, Error> {
    let source = Source {
        path: path.into(),
        decoder,
        endpoint: None,
        crop: None,
    };
    paired_selected(&source, config, range, cancel, progress)
}
/// `range` is in seconds from the selected device's first report inside the crop
pub fn paired_selected(
    source: &Source,
    config: Config,
    range: Option<[f64; 2]>,
    cancel: &AtomicBool,
    progress: impl Fn(f64),
) -> Result<ResultRow, Error> {
    if !WIDTH_MS.contains(&config.width_ms) {
        return Err("texture width must be between 8 and 1000 ms".into());
    }
    let crop = source.crop;
    let located = locate(&source.path)?;
    let (metadata, status) = assess(&located);
    eligibility(&metadata, &status, &located.streams(), PAIRED)
        .map_err(|refusal| format!("this recording cannot be compared, {refusal}"))?;
    let wire = open_wire(source, &located.pcap, cancel)?;
    let (from, to) = wire.span(range, crop)?;
    if to <= from {
        return Err("analysis range is empty".into());
    }
    wire.covers(from, to)?;
    let mut input_rows = 0usize;
    let (shift, input_first, input_last) = recorder::visit_input(&located.input, |_, _, _, _| {
        input_rows += 1;
        check_io(cancel)
    })
    .map_err(Error::from)?;
    let input_from = u64::try_from(from as i128 + shift as i128)
        .map_err(|_| "input alignment exceeds timestamp range")?;
    let input_to = u64::try_from(to as i128 + shift as i128)
        .map_err(|_| "input alignment exceeds timestamp range")?;
    if input_last.is_none_or(|last| last < input_from)
        || input_first.is_none_or(|first| first > input_to)
    {
        return Err("input and wire clocks do not overlap in the selected range".into());
    }
    let mut collector = WindowCollector::new(input_from, input_to, config.width_ms, cancel);
    recorder::visit_input(&located.input, |t, x, y, _| {
        collector
            .push(t, x as i32, y as i32)
            .map_err(std::io::Error::other)
    })
    .map_err(Error::from)?;
    let (real, ri) = collector.finish();
    let (fake, fi) = wire_windows(&wire, from, to, config.width_ms, cancel)?;
    let key = wire.key;
    let mut pairs = BTreeMap::<usize, (Option<Vec<f64>>, Option<Vec<f64>>)>::new();
    for row in real {
        pairs.entry(row.index).or_default().0 = Some(row.row);
    }
    for row in fake {
        pairs.entry(row.index).or_default().1 = Some(row.row);
    }
    let empty = texture::features(&vec![[0; 2]; config.width_ms]).to_vec();
    let mut real = Vec::new();
    let mut fake = Vec::new();
    let mut groups = Vec::new();
    for (index, (r, f)) in pairs {
        real.push(r.unwrap_or_else(|| empty.clone()));
        fake.push(f.unwrap_or_else(|| empty.clone()));
        groups.push(format!("int:{}", index / PAIRED_GROUP_WINDOWS));
    }
    let mut result = finish(
        Population {
            rows: real,
            groups: groups.clone(),
            idle: ri,
        },
        Population {
            rows: fake,
            groups,
            idle: fi,
        },
        texture::NAMES.iter().map(|s| s.to_string()).collect(),
        config,
        cancel,
        progress,
    )?;
    result.comparison = "paired physical input / wire output".into();
    // one recording is both sides, its box input against its wire
    result.baseline = vec![located.id()];
    result.candidate = vec![located.id()];
    result.sources = vec![SourceRecord::read(
        &located,
        &[&located.pcap, &located.sidecar, &located.input],
        cancel,
    )?];
    result.alignment_shift_us = Some(shift);
    result.crops = BTreeMap::from([(located.id(), crop)]);
    result.not_complete = NotComplete::of(&located, &summary(&status))
        .into_iter()
        .collect();

    let source_groups = result.metadata["source_groups"].clone();
    // eligibility took only valid metadata, which always has its selection, pairing and counts
    result.metadata = metadata.valid().cloned().unwrap_or_default();
    result.metadata["recording_status"] = serde_json::json!(summary(&status));
    result.metadata["source_groups"] = source_groups;
    result.metadata["paired"]["input_rows"] = serde_json::json!(input_rows);
    result.metadata["paired"]["shift_us"] = serde_json::json!(shift);
    result.metadata["counts"]["reports"] = serde_json::json!(wire.reports);
    result.metadata["selected_range_us"] = serde_json::json!([from, to]);
    result.metadata["crop_us"] = serde_json::json!(crop);
    result.metadata["selected_endpoint"] =
        serde_json::json!(format!("{}:{}:{}", key.bus, key.device, key.endpoint));
    result.metadata["exclusions"] = serde_json::json!(
        "paired windows with no movement on either side; unilateral idle windows retain zero features"
    );
    Ok(result)
}

#[derive(Clone)]
pub struct Source {
    pub path: PathBuf,
    pub decoder: Option<Decoder>,
    pub endpoint: Option<String>,
    /// only this span of the recording is read
    pub crop: Option<Selection>,
}
/// a recording's target events, only those inside its crop when it has one
fn events(source: &Source) -> Result<(PathBuf, Dataset), String> {
    let located = locate(&source.path)?;
    let path = located
        .event_dataset
        .ok_or_else(|| match located.stale_events {
            Some(why) => format!(
            "the last build of this recording's target events failed so none of them are scored, {why}"
        ),
            None => "this recording has no events.npz with target details".into(),
        })?;
    let mut dataset = Dataset::load(&path)?;
    if let Some(crop) = &source.crop {
        dataset.within(crop)?;
    }
    Ok((path, dataset))
}
/// the sources again once both sides hold the same source identities, which their event
/// datasets name
pub fn matching_sources(
    baselines: &[Source],
    candidates: &[Source],
) -> Result<(Vec<Source>, Vec<Source>), String> {
    let groups = |sources: &[Source]| -> Result<std::collections::BTreeSet<String>, String> {
        let mut groups = std::collections::BTreeSet::new();
        for source in sources {
            groups.extend(events(source)?.1.groups.ok_or(
                "matching source events require source_trial_id, source_group, or user_id",
            )?);
        }
        Ok(groups)
    };
    let (rg, fg) = (groups(baselines)?, groups(candidates)?);
    if rg.is_empty() || rg != fg {
        return Err(
            "matching source events require the same explicit source identities on both sides"
                .into(),
        );
    }
    Ok((baselines.to_vec(), candidates.to_vec()))
}
pub fn independent(
    baselines: &[PathBuf],
    candidates: &[PathBuf],
    panel: &str,
    decoder: Option<Decoder>,
    config: Config,
    cancel: &AtomicBool,
    progress: impl Fn(f64),
) -> Result<ResultRow, Error> {
    let sources = |paths: &[PathBuf]| {
        paths
            .iter()
            .map(|p| Source {
                path: p.clone(),
                decoder: decoder.clone(),
                endpoint: None,
                crop: None,
            })
            .collect::<Vec<_>>()
    };
    independent_selected(
        &sources(baselines),
        &sources(candidates),
        panel,
        None,
        config,
        cancel,
        progress,
    )
}
pub fn independent_selected(
    baselines: &[Source],
    candidates: &[Source],
    panel: &str,
    range: Option<[f64; 2]>,
    config: Config,
    cancel: &AtomicBool,
    progress: impl Fn(f64),
) -> Result<ResultRow, Error> {
    if let Some(range) = range {
        Selection::from_seconds(0, range, None)?;
    }
    if !WIDTH_MS.contains(&config.width_ms) {
        return Err("texture width must be between 8 and 1000 ms".into());
    }
    let mut names = Vec::new();
    let mut provenance = Vec::new();
    let mut crops = BTreeMap::new();
    let mut not_complete = Vec::new();
    let mut records = Vec::new();
    let mut collect = |paths: &[Source]| -> Result<Population, Error> {
        let mut rows = Vec::new();
        let mut groups = Vec::new();
        let mut idle = 0;
        for source in paths {
            check(cancel)?;
            let path = &source.path;
            let located = locate(path)?;
            let (metadata, status) = assess(&located);
            eligibility(&metadata, &status, &located.streams(), COMPARISON)
                .map_err(|refusal| format!("this recording cannot be compared, {refusal}"))?;
            let recording_status = summary(&status);
            not_complete.extend(NotComplete::of(&located, &recording_status));
            let recording = metadata.valid().cloned().unwrap_or_default();
            let crop = source.crop;
            crops.insert(located.id(), crop);
            if panel != "texture19" {
                if range.is_some() {
                    return Err("time brushing applies to wire recordings, event datasets use their explicit movement masks".into());
                }
                let (dataset_path, dataset) = events(source)?;
                records.push(SourceRecord::read(
                    &located,
                    &[&dataset_path, &located.sidecar],
                    cancel,
                )?);
                let (n, r) = dataset.rows(panel)?;
                let event_metadata = std::fs::read(dataset_path.with_extension("json"))
                    .ok()
                    .and_then(|v| serde_json::from_slice::<serde_json::Value>(&v).ok());
                provenance.push(serde_json::json!({"path":path,"event_dataset":dataset_path,"events":event_metadata,"recording":recording,"recording_status":recording_status,"crop_us":crop}));
                names = n;
                let g = dataset
                    .groups
                    .unwrap_or_else(|| vec![format!("str:{}", path.display()); r.len()]);
                rows.extend(r);
                groups.extend(g);
            } else {
                let wire = open_wire(source, &located.pcap, cancel)?;
                records.push(SourceRecord::read(
                    &located,
                    &[&located.pcap, &located.sidecar],
                    cancel,
                )?);
                provenance.push(serde_json::json!({"path":path,"endpoint":format!("{}:{}:{}",wire.key.bus,wire.key.device,wire.key.endpoint),"decoder":wire.decoder,"recording":recording,"recording_status":recording_status,"crop_us":crop}));
                let (from, to) = wire.span(range, crop)?;
                wire.covers(from, to)?;
                let (r, dropped) = wire_windows(&wire, from, to, config.width_ms, cancel)?;
                idle += dropped;
                groups.extend(vec![format!("str:{}", path.display()); r.len()]);
                rows.extend(r.into_iter().map(|r| r.row));
                names = texture::NAMES.iter().map(|s| s.to_string()).collect();
            }
        }
        Ok(Population { rows, groups, idle })
    };
    let real = collect(baselines)?;
    let fake = collect(candidates)?;
    let mut result = finish(real, fake, names, config, cancel, progress)?;
    result.panel = panel.into();
    result.crops = crops;
    result.not_complete = not_complete;
    result.sources = records;
    let ids = |sources: &[Source]| {
        sources
            .iter()
            .map(|source| recording_id(&source.path))
            .collect()
    };
    result.baseline = ids(baselines);
    result.candidate = ids(candidates);

    result.metadata = serde_json::json!({"source_groups":result.metadata["source_groups"],"sources":provenance,"relative_range_seconds":range,"grouping":"recording/session or supplied source identity","exclusions":"idle texture windows"});
    Ok(result)
}

pub fn write_result(path: &Path, result: &ResultRow) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(result).map_err(|e| e.to_string())?;
    let partial = path.with_extension("json.partial");
    std::fs::write(&partial, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(partial, path).map_err(|e| e.to_string())
}
pub fn cli(args: Vec<String>) -> i32 {
    if args.first().is_none_or(|a| a != "judge") || args.len() < 2 {
        eprintln!(
            "usage: motion-bench judge <recording.pcap>...\nreads each whole recording. a crop set in the app lives in the app's library, so it does not apply here"
        );
        return 2;
    }
    let paths = args.iter().skip(1).collect::<Vec<_>>();
    let mut failed = false;
    for path in paths {
        let path = Path::new(path);
        match paired(
            path,
            None,
            Config::default(),
            None,
            &AtomicBool::new(false),
            |_| {},
        ) {
            Ok(mut result) => {
                result.identify();
                println!("{}", serde_json::to_string_pretty(&result).unwrap());
            }
            Err(error) => {
                eprintln!("{}: {error}", path.display());
                failed = true;
            }
        }
    }
    i32::from(failed)
}

#[cfg(test)]
mod tests;
