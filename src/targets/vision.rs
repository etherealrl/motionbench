use crate::evidence::reader::{
    eligibility::{TARGETS, eligibility},
    layout::{locate, stale_marker},
    status::assess,
};
use crate::signal::ticks::{TICK_US, tick};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
    path::Path,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub provider: String,
    pub cuda_device: u32,
    pub cuda_graph: bool,
    pub tensorrt_fp16: bool,
    pub model: String,
    pub scenario: String,
    pub output: u32,
    pub adapter: u32,
    pub width: u32,
    pub height: u32,
    pub horizontal_fov: f64,
    pub sensitivity: f64,
    pub yaw: f64,
    pub pitch: f64,
    pub confidence: f64,
    pub target_class: i32,
    pub fps: u32,
    pub display_delay_ms: u32,
    pub max_frame_gap_ms: u32,
    pub max_residual_radius: f64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            provider: "CUDA".into(),
            cuda_device: 0,
            cuda_graph: true,
            tensorrt_fp16: true,
            model: String::new(),
            scenario: String::new(),
            output: 0,
            adapter: 0,
            width: 2560,
            height: 1440,
            horizontal_fov: 103.0,
            sensitivity: 1.6,
            yaw: 0.07,
            pitch: 0.07,
            confidence: 0.5,
            target_class: 0,
            fps: 0,
            display_delay_ms: 0,
            max_frame_gap_ms: 50,
            max_residual_radius: 0.5,
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if !["CUDA", "TensorRT", "DirectML", "CPU"].contains(&self.provider.as_str())
            || self.cuda_device > 16
            || self.adapter > 16
            || self.output > 16
            || self.target_class < -1
            || self.target_class > 999
            || self.width < 320
            || self.width > 7680
            || self.height < 240
            || self.height > 4320
            || !(30.0..150.0).contains(&self.horizontal_fov)
            || !self.horizontal_fov.is_finite()
            || !(0.01..20.0).contains(&self.sensitivity)
            || !self.sensitivity.is_finite()
            || !(0.001..1.0).contains(&self.yaw)
            || !(0.001..1.0).contains(&self.pitch)
            || !self.yaw.is_finite()
            || !self.pitch.is_finite()
            || !(0.05..=1.0).contains(&self.confidence)
            || self.display_delay_ms > 250
            || !(10..=250).contains(&self.max_frame_gap_ms)
            || !(0.05..=2.0).contains(&self.max_residual_radius)
            || self.scenario.len() > 160
        {
            return Err("the screen capture or calibration settings aren't valid".into());
        }
        Ok(())
    }
    pub fn frame_interval(&self) -> Option<std::time::Duration> {
        (self.fps != 0).then(|| std::time::Duration::from_secs_f64(1.0 / self.fps as f64))
    }
    pub fn counts(&self, x: f64, y: f64) -> [f64; 2] {
        let focal = self.width as f64 / (2.0 * (self.horizontal_fov.to_radians() / 2.0).tan());
        [
            (x - self.width as f64 / 2.0).atan2(focal).to_degrees() / (self.sensitivity * self.yaw),
            (y - self.height as f64 / 2.0).atan2(focal).to_degrees()
                / (self.sensitivity * self.pitch),
        ]
    }
    /// the box's center in the counts `counts` gives, and the angular radius of the ball inscribed in
    /// it, in yaw counts. the angle between two points on screen does not change as the camera
    /// turns, so neither the pitch nor the mouse counts change it
    pub fn geometry(&self, b: &Detection) -> ([f64; 2], f64) {
        let [left, top, right, bottom] = b.bounds;
        let (x, y) = ((left + right) / 2.0, (top + bottom) / 2.0);
        let camera = Camera::new(self);
        let toward = |x, y| camera.direction(self.counts(x, y), [0.0; 2], 0.0);
        let center = toward(x, y);
        // each edge of the box is a plane through the eye that a ball's outline touches, and the
        // box's middle sits off the ball's own center away from the screen's middle, so the radius
        // is half the angle between opposite edges, the narrower pair
        let corners =
            [(left, top), (right, top), (right, bottom), (left, bottom)].map(|(x, y)| toward(x, y));
        let edge = |i: usize| {
            let normal = unit(cross(corners[i], corners[(i + 1) % 4]));
            (90.0 / camera.degrees[0] - camera.apart(center, normal)).abs()
        };
        let radius = ((edge(0) + edge(2)) / 2.0).min((edge(1) + edge(3)) / 2.0);
        (self.counts(x, y), radius)
    }
}
/// the game's camera: a target's place on screen and the mouse's counts so far give the target's
/// direction in the world. the two axes are not apart: away from level, a yaw moves a target on
/// screen less than its counts say, so a target that sits above or below the camera needs the
/// rotation whole
/// a task is taken to start with the camera level, so a fitted starting pitch only stands when it
/// fits clearly better than level; a pitch the data barely tells apart stays level
pub(crate) fn starting_pitch(fitted: f64, misfit: impl Fn(f64) -> f64) -> f64 {
    if misfit(0.0) <= misfit(fitted) * 1.05 {
        0.0
    } else {
        fitted
    }
}

pub(crate) struct Camera {
    focal: f64,
    /// degrees per count, yaw then pitch
    degrees: [f64; 2],
}
impl Camera {
    pub(crate) fn new(profile: &Config) -> Self {
        Self {
            focal: profile.width as f64 / (2.0 * (profile.horizontal_fov.to_radians() / 2.0).tan()),
            degrees: [
                profile.sensitivity * profile.yaw,
                profile.sensitivity * profile.pitch,
            ],
        }
    }
    /// `offset` is where the target is on screen in the counts `Config::counts` gives, `mouse`
    /// the counts moved so far, `pitch` the camera's in degrees when they were none
    pub(crate) fn direction(&self, offset: [f64; 2], mouse: [f64; 2], pitch: f64) -> [f64; 3] {
        let x = self.focal * (offset[0] * self.degrees[0]).to_radians().tan();
        let y = self.focal * (offset[1] * self.degrees[1]).to_radians().tan();
        let yaw = (mouse[0] * self.degrees[0]).to_radians();
        // the mouse moved down turns the camera down
        let pitch = (pitch - mouse[1] * self.degrees[1]).to_radians();
        let forward = [
            pitch.cos() * yaw.sin(),
            pitch.sin(),
            pitch.cos() * yaw.cos(),
        ];
        let right = [yaw.cos(), 0.0, -yaw.sin()];
        let up = [
            -pitch.sin() * yaw.sin(),
            pitch.cos(),
            -pitch.sin() * yaw.cos(),
        ];
        // screen y grows downwards
        unit([0, 1, 2].map(|i| x * right[i] - y * up[i] + self.focal * forward[i]))
    }
    /// the angle between two directions, as the yaw counts that turn it
    pub(crate) fn apart(&self, a: [f64; 3], b: [f64; 3]) -> f64 {
        dot(a, b).clamp(-1.0, 1.0).acos().to_degrees() / self.degrees[0]
    }
    /// the mouse counts that turn the camera from where `mouse` and `pitch` leave it to face
    /// `direction`, the shorter way round
    pub(crate) fn aim(&self, direction: [f64; 3], mouse: [f64; 2], pitch: f64) -> [f64; 2] {
        let yaw = direction[0].atan2(direction[2]).to_degrees() - mouse[0] * self.degrees[0];
        let rise = direction[1].clamp(-1.0, 1.0).asin().to_degrees()
            - (pitch - mouse[1] * self.degrees[1]);
        // the mouse moved down turns the camera down
        [
            ((yaw + 180.0).rem_euclid(360.0) - 180.0) / self.degrees[0],
            -rise / self.degrees[1],
        ]
    }
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(crate) fn unit(v: [f64; 3]) -> [f64; 3] {
    let length = dot(v, v).sqrt();
    v.map(|value| value / length)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Detection {
    pub bounds: [f64; 4],
    pub confidence: f64,
    pub class: usize,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Frame {
    pub clock_anchor: crate::capture::clock::Anchor,
    pub source_qpc: i64,
    pub received_qpc: i64,
    pub completed_qpc: i64,
    pub source_us: u64,
    pub received_us: u64,
    pub completed_us: u64,
    pub preprocess_ms: f64,
    pub pipeline_ms: f64,
    pub inference_ms: f64,
    pub skipped: u32,
    pub detections: Vec<Detection>,
}
impl Frame {
    /// a frame that reads whole and whose time follows from the screen's clock, which is what
    /// lines it up with the mouse's reports
    pub fn validate(&self, evidence: &Evidence) -> Result<(), String> {
        let config = &evidence.config;
        if self.source_us == 0
            || self.detections.len() > 32
            || self.detections.iter().any(|d| {
                d.bounds.iter().any(|v| !v.is_finite())
                    || !d.confidence.is_finite()
                    || !(0.0..=1.0).contains(&d.confidence)
                    || d.bounds[0] < 0.0
                    || d.bounds[1] < 0.0
                    || d.bounds[2] > config.width as f64
                    || d.bounds[3] > config.height as f64
                    || d.bounds[2] <= d.bounds[0]
                    || d.bounds[3] <= d.bounds[1]
            })
        {
            return Err("invalid target frame geometry or timestamp".into());
        }
        if self.source_qpc <= 0
            || self.received_qpc < self.source_qpc
            || self.completed_qpc < self.received_qpc
        {
            return Err("invalid native screen clock bounds".into());
        }
        if self.clock_anchor.qpc_before > self.clock_anchor.qpc_after {
            return Err("invalid screen clock anchor".into());
        }
        let clock = evidence.clock.as_ref().ok_or(NO_CLOCK)?;
        if self
            .clock_anchor
            .epoch_us(self.source_qpc, clock.qpc_frequency)
            != Some(self.source_us)
        {
            return Err(
                "a screen frame's time doesn't follow from its clock, so it can't be lined up with the mouse"
                    .into(),
            );
        }
        Ok(())
    }
}

pub fn yolo8(
    values: &[f32],
    channels: usize,
    anchors: usize,
    config: &Config,
) -> Result<Vec<Detection>, String> {
    if !(5..=1024).contains(&channels) || anchors == 0 || values.len() != channels * anchors {
        return Err("model output must be YOLOv8 [1,4+classes,anchors]".into());
    }
    let mut found = Vec::new();
    for i in 0..anchors {
        let (class, confidence) = (4..channels)
            .map(|c| (c - 4, values[c * anchors + i] as f64))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        if !confidence.is_finite()
            || confidence < config.confidence
            || (config.target_class >= 0 && class != config.target_class as usize)
        {
            continue;
        }
        let [x, y, w, h] = [
            values[i],
            values[anchors + i],
            values[anchors * 2 + i],
            values[anchors * 3 + i],
        ]
        .map(f64::from);
        if [x, y, w, h].iter().any(|v| !v.is_finite()) || w <= 0.0 || h <= 0.0 {
            continue;
        }
        found.push(Detection {
            bounds: [x - w / 2.0, y - h / 2.0, x + w / 2.0, y + h / 2.0],
            confidence,
            class,
        });
    }
    found.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    let mut selected: Vec<Detection> = Vec::new();
    for d in found {
        if selected.iter().any(|s| {
            let area = |v: &Detection| (v.bounds[2] - v.bounds[0]) * (v.bounds[3] - v.bounds[1]);
            let intersection = (s.bounds[2].min(d.bounds[2]) - s.bounds[0].max(d.bounds[0]))
                .max(0.0)
                * (s.bounds[3].min(d.bounds[3]) - s.bounds[1].max(d.bounds[1])).max(0.0);
            s.class == d.class && intersection / (area(s) + area(&d) - intersection) > 0.45
        }) {
            continue;
        }
        selected.push(d);
        if selected.len() == 32 {
            break;
        }
    }
    Ok(selected)
}

#[derive(Serialize, Deserialize)]
pub struct Evidence {
    pub schema: String,
    pub config: Config,
    pub model_sha256: String,
    pub provider: String,
    pub pipeline: String,
    pub inference: Option<serde_json::Value>,
    pub complete: bool,
    pub error: Option<String>,
    pub clock: Option<Clock>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Clock {
    pub initial_bracket_us: f64,
    pub qpc_origin: i64,
    pub qpc_frequency: i64,
    pub epoch_origin_us: u64,
}
#[derive(Serialize)]
pub struct EventSummary {
    pub start_us: u64,
    pub b_us: Option<u64>,
    pub click_us: u64,
    pub accepted: bool,
    pub reason: Option<String>,
    pub radius_counts: Option<f64>,
    pub residual_counts: Option<f64>,
}

fn npy(
    zip: &mut zip::ZipWriter<File>,
    name: &str,
    dtype: &str,
    shape: &[usize],
    bytes: &[u8],
) -> Result<(), String> {
    let shape = shape.iter().map(|v| format!("{v},")).collect::<String>();
    let mut header =
        format!("{{'descr': '{dtype}', 'fortran_order': False, 'shape': ({shape}), }}");
    header.extend(std::iter::repeat_n(
        ' ',
        (64 - (10 + header.len() + 1) % 64) % 64,
    ));
    header.push('\n');
    zip.start_file(
        format!("{name}.npy"),
        zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated),
    )
    .map_err(|e| e.to_string())?;
    zip.write_all(b"\x93NUMPY\x01\x00")
        .map_err(|e| e.to_string())?;
    zip.write_all(&(header.len() as u16).to_le_bytes())
        .map_err(|e| e.to_string())?;
    zip.write_all(header.as_bytes())
        .map_err(|e| e.to_string())?;
    zip.write_all(bytes).map_err(|e| e.to_string())
}

/// a target seen on screen: the tick, its center in the counts `Config::counts` gives, its radius
type Sighting = (usize, [f64; 2], f64);

const NO_CLOCK: &str =
    "the screen recording has no clock, so its frames can't be lined up with the mouse";

/// the target capture beside `pcap`, refused with `incomplete` when it did not finish whole
pub(crate) fn whole_evidence(pcap: &Path, incomplete: &str) -> Result<Evidence, String> {
    let evidence: Evidence = serde_json::from_slice(
        &std::fs::read(pcap.with_extension("vision.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if !evidence.complete || evidence.error.is_some() {
        return Err(incomplete.into());
    }
    Ok(evidence)
}

/// the longest recording events are built from, twenty minutes
const MAX_EVENT_RECORDING_US: u64 = 1_200_000_000;
/// how long an aim's movement can take, first movement to click, in ticks
const MOVEMENT_MS: std::ops::RangeInclusive<usize> = 20..=3000;
/// the ticks of movement before B80 an event keeps, what the judges read as its lead-in
const PREFIX_TICKS: usize = 256;
/// how long after a click the screen may show the clicked target leaving and the next arriving,
/// none or two at once, in ticks. it's under a reaction time, so no aim at the next one is in it
const SWAP_MS: usize = 100;

const PARTIALS: [&str; 2] = ["events.json.partial", "events.npz.partial"];

/// the spans, on the system clock, where target capture beside `pcap` saw more than one target
/// for at least 50 ms, frames less than 50 ms apart in one span. a recording without target
/// capture has none
pub fn crowded_spans(pcap: &Path) -> Result<Vec<[u64; 2]>, String> {
    let Ok(file) = File::open(pcap.with_extension("targets.jsonl")) else {
        return Ok(Vec::new());
    };
    let gap = 50_000;
    let mut spans: Vec<[u64; 2]> = Vec::new();
    for line in BufReader::new(file).lines() {
        let frame: Frame =
            serde_json::from_str(&line.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        if frame.detections.len() < 2 {
            continue;
        }
        let t = frame.source_us;
        match spans.last_mut() {
            Some(span) if t <= span[1] + gap => span[1] = t,
            _ => spans.push([t, t]),
        }
    }
    // AimLab draws the shot target and the next one together for a frame after every click
    spans.retain(|[from, to]| to - from >= gap);
    Ok(spans)
}

/// the target events beside `pcap`, how many were accepted. a build that fails leaves the
/// previous events on disk and marks them stale, so they stay readable but never enter a score
pub fn build_events(pcap: &Path) -> Result<usize, String> {
    let built = build(pcap);
    if let Err(error) = &built {
        for partial in PARTIALS {
            let _ = std::fs::remove_file(pcap.with_extension(partial));
        }
        // a full disk is the likeliest reason for both failing, and old events left unmarked
        // would be scored, so they go instead
        if std::fs::write(stale_marker(pcap), error).is_err() {
            match std::fs::remove_file(pcap.with_extension("events.npz")) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    return Err(format!(
                        "{error}, and the old events could be neither marked stale nor removed ({e})"
                    ));
                }
                _ => (),
            }
        }
    }
    built
}

/// the new events replace the old ones only once both files are written whole. the marker stays
/// up through the swap, so the app stopping between the renames leaves them stale and not mixed
fn commit(pcap: &Path, dataset: bool) -> Result<(), String> {
    let io = |e: std::io::Error| e.to_string();
    std::fs::write(
        stale_marker(pcap),
        "the app stopped while they were being replaced",
    )
    .map_err(io)?;
    let destination = pcap.with_extension("events.npz");
    if dataset {
        std::fs::rename(pcap.with_extension("events.npz.partial"), destination).map_err(io)?;
    } else {
        match std::fs::remove_file(destination) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
            _ => (),
        }
    }
    std::fs::rename(
        pcap.with_extension("events.json.partial"),
        pcap.with_extension("events.json"),
    )
    .map_err(io)?;
    std::fs::remove_file(stale_marker(pcap)).map_err(io)
}

/// the selected mouse's counts summed into 1 ms ticks from its first report, and the ticks its
/// left button went down in
pub(crate) struct MouseTicks {
    /// the first report, on the system clock
    pub origin: u64,
    pub ticks: Vec<[i32; 2]>,
    pub clicks: Vec<usize>,
}

/// `MouseTicks` of the finished recording `pcap`, whose sidecar is `metadata`
pub(crate) fn mouse_ticks(pcap: &Path, metadata: &serde_json::Value) -> Result<MouseTicks, String> {
    let decoder: crate::capture::hid::Decoder =
        serde_json::from_value(metadata["decoder"].clone()).map_err(|e| e.to_string())?;
    let selection = &metadata["selection"];
    let selected = (
        selection["bus"].as_u64(),
        selection["address"].as_u64(),
        selection["endpoint"].as_u64(),
    );
    let mut reports = Vec::new();
    let mut error = None;
    let mut button_available = false;
    let mut pressed = false;
    crate::capture::pcap::read_pcap(
        BufReader::new(File::open(pcap).map_err(|e| e.to_string())?),
        |record| {
            if let crate::capture::pcap::Record::Packet {
                event: Some(crate::capture::pcap::Event::Report(r)),
                ..
            } = record
            {
                if selected
                    != (
                        Some(r.bus as u64),
                        Some(r.device as u64),
                        Some(r.endpoint as u64),
                    )
                {
                    return;
                }
                if reports.len() > 10_000_000 {
                    error = Some("event recording exceeds ten million reports".to_string());
                    return;
                }
                let click = decoder.left_button(r.bytes());
                button_available |= click.is_some();
                if let Some(value) = click {
                    pressed = value;
                }
                let motion = decoder.decode(r.bytes());
                if click.is_some() || motion.is_some() {
                    let (x, y) = motion.unwrap_or((0, 0));
                    reports.push((r.t_us, x, y, pressed));
                }
            }
        },
    )
    .map_err(|e| e.to_string())?;
    if let Some(error) = error {
        return Err(error);
    }
    if !button_available {
        return Err("mouse layout has no decoded left button, reselect its Windows layout before recording events".into());
    }
    // checked before the ticks are sized from the last report, which a forward jump would outrun
    if reports.windows(2).any(|pair| pair[1].0 < pair[0].0) {
        return Err("mouse timestamps regressed".into());
    }
    let origin = reports.first().ok_or("mouse recording is empty")?.0;
    let end = reports.last().unwrap().0;
    if end - origin > MAX_EVENT_RECORDING_US {
        return Err("event recording must be at most twenty minutes".into());
    }
    let tick = |t: u64| tick(origin, t).expect("checked to be past the first report");
    let mut ticks = vec![[0i32; 2]; tick(end) + 1];
    let mut clicks = Vec::new();
    let mut down = false;
    for (t, x, y, pressed) in reports {
        let at = &mut ticks[tick(t)];
        at[0] = at[0].checked_add(x).ok_or("count overflow")?;
        at[1] = at[1].checked_add(y).ok_or("count overflow")?;
        if pressed && !down {
            clicks.push(tick(t));
        }
        down = pressed;
    }
    Ok(MouseTicks {
        origin,
        ticks,
        clicks,
    })
}

fn build(pcap: &Path) -> Result<usize, String> {
    // left by a build the app was closed in the middle of
    for partial in PARTIALS {
        match std::fs::remove_file(pcap.with_extension(partial)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
            _ => (),
        }
    }
    let evidence = whole_evidence(
        pcap,
        "screen recording is incomplete, raw evidence retained",
    )?;
    evidence.config.validate()?;
    if evidence.config.scenario.trim().is_empty() {
        return Err(
            "target capture had no scenario name, so this recording has no Full49 events".into(),
        );
    }
    let located = locate(pcap)?;
    let (metadata, status) = assess(&located);
    eligibility(&metadata, &status, &located.streams(), TARGETS)
        .map_err(|refusal| format!("mouse recording is incomplete: {refusal}"))?;
    // a finished recording always has a readable sidecar, the eligibility above made sure
    let metadata = metadata.valid().cloned().unwrap_or_default();
    let MouseTicks {
        origin,
        ticks,
        clicks,
    } = mouse_ticks(pcap, &metadata)?;
    let mut frames = Vec::new();
    for line in
        BufReader::new(File::open(pcap.with_extension("targets.jsonl")).map_err(|e| e.to_string())?)
            .lines()
    {
        if frames.len() > 300_000 {
            return Err("there are too many target frames".into());
        }
        let frame: Frame =
            serde_json::from_str(&line.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        frame.validate(&evidence)?;
        if frames
            .last()
            .is_some_and(|f: &Frame| f.source_us >= frame.source_us)
        {
            return Err("screen timestamps regressed".into());
        }
        frames.push(frame);
    }
    let cfg = &evidence.config;
    let mut cumulative = vec![[0.0; 2]; ticks.len() + 1];
    for (i, tick) in ticks.iter().enumerate() {
        cumulative[i + 1] = [
            cumulative[i][0] + tick[0] as f64,
            cumulative[i][1] + tick[1] as f64,
        ];
    }
    // each click's movement start and the target's place on screen through it
    let mut previous = 0;
    let candidates: Vec<_> = clicks
        .into_iter()
        .map(|click| {
            let gathered = (|| -> Result<(usize, Vec<Sighting>), String> {
                let seen = frames.iter().filter_map(|f| {
                    let t = f
                        .source_us
                        .saturating_sub(cfg.display_delay_ms as u64 * 1000);
                    Some((crate::signal::ticks::tick(origin, t)?, f))
                });
                // the event starts once the clicked target is gone and the next one is alone
                let from = seen
                    .clone()
                    .rfind(|(i, f)| {
                        (previous..previous + SWAP_MS).contains(i) && f.detections.len() != 1
                    })
                    .map_or(previous, |(i, _)| i + 1);
                let start = (from..click)
                    .find(|i| ticks[*i] != [0, 0])
                    .ok_or("no movement before click")?;
                if !MOVEMENT_MS.contains(&(click - start)) {
                    return Err("movement duration outside 20 to 3000 ms".into());
                }
                let observed = seen
                    .filter(|(i, _)| {
                        *i >= from.max(start.saturating_sub(cfg.max_frame_gap_ms as usize))
                            && *i <= click
                    })
                    .collect::<Vec<_>>();
                if observed.len() < 3 {
                    return Err("not enough target frames".into());
                }
                if observed.iter().any(|(_, f)| f.detections.len() != 1) {
                    return Err("target missing or multiple detections".into());
                }
                if observed
                    .windows(2)
                    .any(|v| v[1].0 - v[0].0 > cfg.max_frame_gap_ms as usize)
                    || observed[0].0 > start.max(from + cfg.max_frame_gap_ms as usize)
                    || click - observed.last().unwrap().0 > cfg.max_frame_gap_ms as usize
                {
                    return Err("screen coverage has a gap".into());
                }
                Ok((
                    start,
                    observed
                        .iter()
                        .map(|(i, f)| {
                            let (offset, radius) = cfg.geometry(&f.detections[0]);
                            (*i, offset, radius)
                        })
                        .collect(),
                ))
            })();
            let candidate = (click, previous, gathered);
            previous = click + 1;
            candidate
        })
        .collect();
    let camera = Camera::new(cfg);
    // how far, in counts, each event's observations point from their mean direction, summed over
    // the events; a stationary target stays in one direction, so the spread is the misfit
    let spread = |pitch: f64| -> f64 {
        candidates
            .iter()
            .filter_map(|(_, _, gathered)| gathered.as_ref().ok())
            .map(|(_, observed)| {
                let directions: Vec<_> = observed
                    .iter()
                    .map(|(i, offset, _)| camera.direction(*offset, cumulative[*i], pitch))
                    .collect();
                let mean = unit(
                    directions
                        .iter()
                        .fold([0.0; 3], |sum, d| [0, 1, 2].map(|a| sum[a] + d[a])),
                );
                directions
                    .iter()
                    .map(|d| camera.apart(*d, mean))
                    .sum::<f64>()
            })
            .sum()
    };
    let lowest = |pitches: &mut dyn Iterator<Item = f64>| {
        pitches
            .map(|pitch| (spread(pitch), pitch))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .unwrap()
            .1
    };
    // where the camera pointed vertically when the recording started is not recorded, and a target
    // above or below it moves on screen differently as the camera turns: it is found on a 2 degree
    // grid, then to a quarter degree and a fortieth, like the response-delay fit
    let pitch = candidates
        .iter()
        .any(|(_, _, gathered)| gathered.is_ok())
        .then(|| {
            let coarse = lowest(&mut (-40..=40).map(|step| step as f64 * 2.0));
            let refine =
                |around: f64, step: f64| lowest(&mut (-10..=10).map(|i| around + i as f64 * step));
            starting_pitch(refine(refine(coarse, 0.25), 0.025), spread)
        });
    let mut future = Vec::new();
    let mut prefix = Vec::new();
    let mut targets = Vec::new();
    let mut radii = Vec::new();
    let mut spans = Vec::new();
    let mut summaries = Vec::new();
    for (click, previous, gathered) in candidates {
        let outcome = gathered.and_then(|(start, observed)| {
            let pitch = pitch.unwrap_or_default();
            let (i, offset, radius) = observed[0];
            if radius <= 0.0 {
                return Err("invalid target radius".into());
            }
            let world = camera.direction(offset, cumulative[i], pitch);
            let distance = |i: usize| {
                let aim = camera.aim(world, cumulative[i], pitch);
                aim[0].hypot(aim[1])
            };
            let initial = distance(start);
            if initial <= radius * 2.0 {
                return Err("target starts too close to the crosshair".into());
            }
            let residual = observed
                .iter()
                .map(|(i, offset, _)| {
                    camera.apart(camera.direction(*offset, cumulative[*i], pitch), world)
                })
                .fold(0.0, f64::max);
            if residual > radius * cfg.max_residual_radius {
                return Err(format!(
                    "target/projection residual {residual:.1} counts exceeds tolerance, check calibration, display delay, or target motion"
                ));
            }
            let b = (start..click)
                .find(|i| distance(*i) <= radius + 0.2 * (initial - radius))
                .ok_or("movement never reaches B80")?;
            if b < PREFIX_TICKS || click - b < 8 {
                return Err("need a 256 ms prefix and at least 8 ms after B80".into());
            }
            let &(seen, offset, b_radius) = observed
                .iter()
                .rev()
                .find(|(i, _, _)| *i <= b)
                .ok_or("no target observation before B80")?;
            if b - seen > cfg.max_frame_gap_ms as usize {
                return Err("target observation at B80 is stale".into());
            }
            let target = camera.aim(
                camera.direction(offset, cumulative[seen], pitch),
                cumulative[b],
                pitch,
            );
            Ok((start, b, target, b_radius, residual))
        });
        match outcome {
            Ok((start, b, target, radius, residual)) => {
                future.push(ticks[b..=click].to_vec());
                prefix.push(ticks[b - PREFIX_TICKS..b].to_vec());
                targets.push(target);
                radii.push(radius);
                spans.push([
                    origin + (b - PREFIX_TICKS) as u64 * TICK_US,
                    origin + click as u64 * TICK_US,
                ]);
                summaries.push(EventSummary {
                    start_us: origin + start as u64 * TICK_US,
                    b_us: Some(origin + b as u64 * TICK_US),
                    click_us: origin + click as u64 * TICK_US,
                    accepted: true,
                    reason: None,
                    radius_counts: Some(radius),
                    residual_counts: Some(residual),
                });
            }
            Err(reason) => summaries.push(EventSummary {
                start_us: origin + previous as u64 * TICK_US,
                b_us: None,
                click_us: origin + click as u64 * TICK_US,
                accepted: false,
                reason: Some(reason),
                radius_counts: None,
                residual_counts: None,
            }),
        }
    }
    let summary = serde_json::json!({"schema":"motion-bench.events.v1","evidence":evidence,"protocol":"single stationary circular target; wire left-click end; first nonzero movement once the clicked target is replaced, up to 100 ms after the click; B80 target-edge progress; 256 ms prefix; 1 ms integer ticks","geometry":"bounding-box inscribed ball, its radius half the angle between the nearer pair of opposite box edges; target direction through the camera's whole rotation from a starting pitch fitted to every candidate event's observations; residual, distances, B80 and the target at B80 in the mouse counts that face the target, the radius in yaw counts; estimated","starting_pitch_degrees":pitch,"source_group":pcap.display().to_string(),"events":summaries,"accepted":future.len()});
    std::fs::write(
        pcap.with_extension("events.json.partial"),
        serde_json::to_vec_pretty(&summary).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    // a calibration recording has no clicks, so no events; why each was left out is above
    if future.is_empty() {
        commit(pcap, false)?;
        return Ok(0);
    }
    let n = future.len();
    let h = future.iter().map(Vec::len).max().unwrap();
    let temporary = pcap.with_extension("events.npz.partial");
    let mut zip = zip::ZipWriter::new(File::create(&temporary).map_err(|e| e.to_string())?);
    for (name, streams, width) in [("future", &future, h), ("prefix", &prefix, 256)] {
        let mut raw = Vec::new();
        let mut mask = Vec::new();
        for stream in streams {
            for j in 0..width {
                for v in stream.get(j).copied().unwrap_or([0, 0]) {
                    raw.extend(v.to_le_bytes());
                }
                mask.push(u8::from(j < stream.len()));
            }
        }
        npy(
            &mut zip,
            &format!("{name}_raw_dxdy"),
            "<i4",
            &[n, width, 2],
            &raw,
        )?;
        npy(&mut zip, &format!("{name}_mask"), "|b1", &[n, width], &mask)?;
    }
    for (name, values) in [
        (
            "target_rel_x_at_B",
            targets.iter().map(|v| v[0]).collect::<Vec<_>>(),
        ),
        ("target_rel_y_at_B", targets.iter().map(|v| v[1]).collect()),
        ("target_radius", radii),
    ] {
        npy(
            &mut zip,
            name,
            "<f8",
            &[n],
            &values
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>(),
        )?;
    }
    // on the pcap clock, so a crop kept in the app's library can pick events without this file
    // ever changing
    for (name, at) in [("event_from_us", 0), ("event_to_us", 1)] {
        npy(
            &mut zip,
            name,
            "<i8",
            &[n],
            &spans
                .iter()
                .flat_map(|span| (span[at] as i64).to_le_bytes())
                .collect::<Vec<_>>(),
        )?;
    }
    let identity = pcap.display().to_string();
    let width = identity.len();
    npy(
        &mut zip,
        "source_group",
        &format!("|S{width}"),
        &[n],
        &identity.as_bytes().repeat(n),
    )?;
    zip.finish()
        .map_err(|e| e.to_string())?
        .sync_all()
        .map_err(|e| e.to_string())?;
    commit(pcap, true)?;
    Ok(n)
}

#[cfg(test)]
pub(crate) mod tests;
