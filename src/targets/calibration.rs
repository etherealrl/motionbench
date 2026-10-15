use crate::evidence::reader::{
    eligibility::{TARGETS, eligibility},
    layout::locate,
    status::assess,
};
use crate::{
    analysis::compare::Selection,
    capture::{hid::Decoder, pcap},
    targets::vision::{Camera, Config, Frame, starting_pitch, unit, whole_evidence},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

#[derive(Clone, Serialize, Deserialize)]
pub struct Response {
    pub schema: String,
    pub method: String,
    pub estimated_delay_ms: u32,
    pub admissible_delay_ms: [u32; 2],
    pub residual_counts: f64,
    pub radius_counts: f64,
    pub movement_span_counts: f64,
    pub frames: usize,
    pub span_ms: u64,
    pub selected_range_seconds: [f64; 2],
    pub single_target_segments: usize,
    pub excluded_samples: usize,
    pub identifiable: bool,
    pub reason: Option<String>,
    pub profile: Config,
    pub residual_curve: Vec<[f64; 2]>,
    pub absolute_hardware_latency_known: bool,
    pub applied_to_scores: bool,
    /// the span of the recording read, none for the whole of it
    pub crop_us: Option<Selection>,
}

fn upper_median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn position(motion: &[(u64, [f64; 2])], time: u64) -> [f64; 2] {
    let next = motion.partition_point(|(t, _)| *t <= time);
    motion[next.saturating_sub(1)].1
}

pub fn fit(
    profile: &Config,
    motion: &[(u64, [f64; 2])],
    frames: &[(u64, [f64; 2], f64)],
) -> Result<Response, String> {
    profile.validate()?;
    if motion.len() < 2 || frames.len() < 90 {
        return Err("record at least three seconds of movement around one stationary target without clicking".into());
    }
    if motion.iter().any(|(_, p)| p.iter().any(|v| !v.is_finite()))
        || motion.windows(2).any(|w| w[0].0 >= w[1].0)
        || frames.windows(2).any(|w| w[0].0 >= w[1].0)
        || frames
            .iter()
            .any(|(_, p, r)| p.iter().any(|v| !v.is_finite()) || !r.is_finite() || *r <= 0.0)
    {
        return Err("the calibration's positions or timestamps aren't valid".into());
    }
    let start = motion[0]
        .0
        .checked_add(250_000)
        .ok_or("calibration timestamp overflow")?;
    let end = motion.last().unwrap().0;
    let frames: Vec<_> = frames
        .iter()
        .filter(|(t, _, _)| *t >= start && *t <= end)
        .collect();
    if frames.len() < 90 || frames.last().unwrap().0 - frames[0].0 < 3_000_000 {
        return Err("not enough shared mouse/screen coverage, record at least three seconds without clicking".into());
    }
    if frames
        .windows(2)
        .any(|w| w[1].0 - w[0].0 > profile.max_frame_gap_ms as u64 * 1000)
    {
        return Err("screen coverage has gaps, reduce game/GPU load before calibration".into());
    }
    let radius_counts = upper_median(&mut frames.iter().map(|v| v.2).collect::<Vec<_>>());
    let mut extent = [[f64::INFINITY, f64::NEG_INFINITY]; 2];
    for (t, _, _) in &frames {
        let p = position(motion, *t);
        for axis in 0..2 {
            extent[axis][0] = extent[axis][0].min(p[axis]);
            extent[axis][1] = extent[axis][1].max(p[axis]);
        }
    }
    let span = (extent[0][1] - extent[0][0]).hypot(extent[1][1] - extent[1][0]);
    if span < (radius_counts * 4.0).max(100.0) {
        return Err("too little movement to estimate response delay, move back and forth around the same target".into());
    }
    let camera = Camera::new(profile);
    // the p90 distance, in counts, of each frame's target direction from their middle one; a
    // stationary target stays in one direction, so the spread is the misfit
    let spread = |delay: u32, pitch: f64| {
        let directions: Vec<_> = frames
            .iter()
            .map(|(t, p, _)| camera.direction(*p, position(motion, t - delay as u64 * 1000), pitch))
            .collect();
        let middle =
            unit([0, 1, 2].map(|axis| {
                upper_median(&mut directions.iter().map(|d| d[axis]).collect::<Vec<_>>())
            }));
        let mut residual: Vec<_> = directions
            .iter()
            .map(|d| camera.apart(*d, middle))
            .collect();
        residual.sort_by(f64::total_cmp);
        residual[(residual.len() - 1) * 9 / 10]
    };
    // where the camera pointed vertically when the mouse started is not recorded: it is found on
    // a coarse grid of delays, then refined at the best one
    let mut start = (f64::INFINITY, 0, 0.0);
    for delay in (0..=250).step_by(25) {
        for step in -40..=40 {
            let pitch = step as f64 * 2.0;
            let residual = spread(delay, pitch);
            if residual < start.0 {
                start = (residual, delay, pitch);
            }
        }
    }
    let (_, delay, coarse) = start;
    // to a quarter degree, then a fortieth: a pitch left off by more turns into a wrong delay
    let refine = |around: f64, step: f64| {
        (-10..=10)
            .map(|i| around + i as f64 * step)
            .min_by(|a, b| spread(delay, *a).total_cmp(&spread(delay, *b)))
            .unwrap()
    };
    let pitch = starting_pitch(refine(refine(coarse, 0.25), 0.025), |pitch| {
        spread(delay, pitch)
    });
    let curve: Vec<_> = (0..=250u32)
        .map(|delay| [delay as f64, spread(delay, pitch)])
        .collect();
    let best = curve.iter().min_by(|a, b| a[1].total_cmp(&b[1])).unwrap();
    // this is a residual tolerance band, not a statistical confidence interval
    let tolerance = best[1] + (best[1] * 0.2).max(1.0);
    let compatible: Vec<_> = curve.iter().filter(|v| v[1] <= tolerance).collect();
    let range = [
        compatible.first().unwrap()[0] as u32,
        compatible.last().unwrap()[0] as u32,
    ];
    let reason = if best[0] == 0.0 || best[0] == 250.0 {
        Some("best fit is at the search boundary, delay is not resolved".into())
    } else if range[1] - range[0] > 10 {
        Some(
            "multiple delays fit, use irregular back-and-forth movement around one fixed target"
                .into(),
        )
    } else if best[1] > radius_counts * profile.max_residual_radius {
        Some(
            "target residual is too large, check projection, detector geometry, or target motion"
                .into(),
        )
    } else {
        None
    };
    Ok(Response {
        schema: "motion-bench.response-calibration.v1".into(),
        method:
            "stationary target direction from the camera's rotation; starting pitch fitted; 0 to 250 ms sweep; p90 angular residual; shared coverage"
                .into(),
        estimated_delay_ms: best[0] as u32,
        admissible_delay_ms: range,
        residual_counts: best[1],
        radius_counts,
        movement_span_counts: span,
        frames: frames.len(),
        span_ms: (frames.last().unwrap().0 - frames[0].0) / 1000,
        selected_range_seconds: [
            (frames[0].0 - motion[0].0) as f64 / 1e6,
            (frames.last().unwrap().0 - motion[0].0) as f64 / 1e6,
        ],
        single_target_segments: 1,
        excluded_samples: 0,
        identifiable: reason.is_none(),
        reason,
        profile: profile.clone(),
        residual_curve: curve,
        absolute_hardware_latency_known: false,
        applied_to_scores: false,
        crop_us: None,
    })
}

/// the estimate from a stationary target recording, only its `crop` when it has one. the frames
/// follow from the movement read, which already lies inside the crop
pub fn recording(pcap: &Path, crop: Option<Selection>) -> Result<Response, String> {
    let located = locate(pcap)?;
    let (metadata, status) = assess(&located);
    eligibility(&metadata, &status, &located.streams(), TARGETS)
        .map_err(|refusal| format!("calibration needs a complete mouse recording: {refusal}"))?;
    // a finished recording always has a readable sidecar, the eligibility above made sure
    let metadata = metadata.valid().cloned().unwrap_or_default();
    let evidence = whole_evidence(
        pcap,
        "calibration needs complete mouse and target recordings",
    )?;
    if evidence.config.scenario.trim().is_empty() {
        return Err(
            "target capture had no scenario name, so this recording cannot calibrate".into(),
        );
    }
    let decoder: Decoder =
        serde_json::from_value(metadata["decoder"].clone()).map_err(|e| e.to_string())?;
    let selected = &metadata["selection"];
    let mut motion = Vec::new();
    let mut origin = None;
    let mut sum = [0.0; 2];
    let mut clicks = Vec::new();
    let mut click_start = None;
    let mut last = None;
    let mut error = None;
    pcap::read_pcap(
        BufReader::new(File::open(pcap).map_err(|e| e.to_string())?),
        |record| {
            if error.is_some() {
                return;
            }
            if let pcap::Record::Packet {
                event: Some(pcap::Event::Report(r)),
                ..
            } = record
            {
                if selected["bus"].as_u64() != Some(r.bus as u64)
                    || selected["address"].as_u64() != Some(r.device as u64)
                    || selected["endpoint"].as_u64() != Some(r.endpoint as u64)
                {
                    return;
                }
                if last.is_some_and(|t| r.t_us < t) {
                    error = Some("mouse timestamps regressed".to_string());
                }
                last = Some(r.t_us);
                if crop.is_some_and(|c| !c.contains(r.t_us)) {
                    return;
                }
                let first = *origin.get_or_insert(r.t_us);
                if r.t_us.saturating_sub(first) > 60_000_000 {
                    return;
                }
                let click = decoder.left_button(r.bytes());
                if click == Some(true) {
                    click_start.get_or_insert(r.t_us);
                } else if let Some(start) = click_start.take() {
                    clicks.push((start, r.t_us));
                }
                if let Some((dx, dy)) = decoder.decode(r.bytes()) {
                    sum[0] += dx as f64;
                    sum[1] += dy as f64;
                    if motion.last().is_some_and(|(t, _)| *t == r.t_us) {
                        motion.last_mut().unwrap().1 = sum;
                    } else {
                        motion.push((r.t_us, sum));
                    }
                    if motion.len() > 500_000 {
                        error =
                            Some("calibration exceeds 500,000 reports in its first minute".into());
                    }
                }
            }
        },
    )
    .map_err(|e| e.to_string())?;
    if let Some(error) = error {
        return Err(error);
    }
    let end = motion
        .last()
        .ok_or("no decoded movement for calibration")?
        .0;
    if let Some(start) = click_start {
        clicks.push((start, end));
    }
    let mut segments = vec![Vec::new()];
    let mut last_source = None;
    let mut last_sample = 0;
    for line in
        BufReader::new(File::open(pcap.with_extension("targets.jsonl")).map_err(|e| e.to_string())?)
            .lines()
    {
        let frame: Frame =
            serde_json::from_str(&line.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        frame.validate(&evidence)?;
        if last_source.is_some_and(|t| frame.source_us <= t) {
            return Err("screen timestamps regressed".into());
        }
        if last_source
            .is_some_and(|t| frame.source_us - t > evidence.config.max_frame_gap_ms as u64 * 1000)
            && !segments.last().unwrap().is_empty()
        {
            segments.push(Vec::new());
        }
        last_source = Some(frame.source_us);
        if frame.source_us > end {
            break;
        }
        if frame.source_us < origin.unwrap() {
            continue;
        }
        // keep the entire delay sweep after release clear of click-induced target changes
        let near_click = clicks.iter().any(|(start, release)| {
            frame.source_us >= *start && frame.source_us <= release.saturating_add(250_000)
        });
        if frame.detections.len() != 1 || near_click {
            if !segments.last().unwrap().is_empty() {
                segments.push(Vec::new());
            }
            continue;
        }
        if frame.source_us - last_sample < 10_000 {
            continue;
        }
        last_sample = frame.source_us;
        let (position, radius) = evidence.config.geometry(&frame.detections[0]);
        segments
            .last_mut()
            .unwrap()
            .push((frame.source_us, position, radius));
    }
    segments.retain(|s| !s.is_empty());
    let frames = segments
        .iter()
        .max_by_key(|s| s.last().unwrap().0 - s[0].0)
        .ok_or("no uninterrupted click-free single-target coverage")?;
    let mut result = fit(&evidence.config, &motion, frames)?;
    result
        .method
        .push_str("; longest click-free single-target segment; 250 ms release guard");
    result.crop_us = crop;
    result.single_target_segments = segments.len();
    result.excluded_samples = segments.iter().map(Vec::len).sum::<usize>() - result.frames;
    Ok(result)
}

#[cfg(test)]
mod tests;
