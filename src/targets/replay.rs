//! a recording with target capture played back: what the screen showed and where the mouse
//! turned the camera, on one clock

use super::vision::{Frame, mouse_ticks};
use crate::evidence::reader::{layout::locate, status::assess};
use serde::Serialize;
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};

/// what the replay draws, the camera's settings with the mouse and the screen on one timeline
#[derive(Serialize)]
pub struct Replay {
    /// the captured screen, in pixels
    pub width: u32,
    pub height: u32,
    pub horizontal_fov: f64,
    /// degrees per count, yaw then pitch
    pub degrees: [f64; 2],
    /// where the camera pointed vertically when the recording started, from the target events
    /// when they fitted one, level otherwise
    pub starting_pitch: f64,
    /// the first mouse report, on the system clock, the timeline's zero
    pub from_us: u64,
    /// the mouse's running counts at the end of each 1 ms tick
    pub mouse: Vec<[i64; 2]>,
    /// the ticks the left button went down in
    pub clicks: Vec<usize>,
    pub frames: Vec<Shown>,
}

/// one captured screen frame
#[derive(Serialize)]
pub struct Shown {
    /// when it was on screen, in ms from the first report. the display delay is taken off, so it
    /// lines up with the counts that put the camera there
    pub at_ms: f64,
    /// each target's box, left, top, right, bottom in pixels
    pub boxes: Vec<[f64; 4]>,
}

/// the replay of `pcap`, which needs its target capture and its mouse
pub fn replay(pcap: &Path) -> Result<Replay, String> {
    let evidence = super::vision::whole_evidence(
        pcap,
        "this recording's target capture didn't finish, so it can't be played back",
    )?;
    let config = evidence.config;
    let located = locate(pcap)?;
    let (metadata, _) = assess(&located);
    let metadata = metadata
        .valid()
        .cloned()
        .ok_or("this recording has no readable sidecar, so its mouse can't be played back")?;
    let mouse = mouse_ticks(pcap, &metadata)?;
    let mut sum = [0i64; 2];
    let running = mouse
        .ticks
        .iter()
        .map(|tick| {
            sum = [sum[0] + i64::from(tick[0]), sum[1] + i64::from(tick[1])];
            sum
        })
        .collect();
    let delay_us = u64::from(config.display_delay_ms) * 1000;
    let mut frames = Vec::new();
    let file = File::open(pcap.with_extension("targets.jsonl")).map_err(|e| e.to_string())?;
    for line in BufReader::new(file).lines() {
        let frame: Frame =
            serde_json::from_str(&line.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let at_us = frame.source_us.saturating_sub(delay_us) as f64 - mouse.origin as f64;
        frames.push(Shown {
            at_ms: at_us / 1000.0,
            boxes: frame.detections.iter().map(|d| d.bounds).collect(),
        });
    }
    let starting_pitch = std::fs::read(pcap.with_extension("events.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|events| events["starting_pitch_degrees"].as_f64())
        .unwrap_or(0.0);
    Ok(Replay {
        width: config.width,
        height: config.height,
        horizontal_fov: config.horizontal_fov,
        degrees: [
            config.sensitivity * config.yaw,
            config.sensitivity * config.pitch,
        ],
        starting_pitch,
        from_us: mouse.origin,
        mouse: running,
        clicks: mouse.clicks,
        frames,
    })
}

#[cfg(test)]
mod tests;
