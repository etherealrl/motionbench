//! the Movement tab: what the decoded counts were, report by report, and how the intervals
//! behave at each speed. needs a decoder; the plot leaves the tab out without one

use super::intervals::{Moving, PAUSE_US, median};
use crate::signal::stats::View;
use serde::{Deserialize, Serialize};

/// counts per report are binned 0 to 10, then one bin for more
const COUNT_BINS: usize = 12;
/// speed bin edges in counts per millisecond, doubling, with one bin under the first and one
/// past the last
const SPEED_EDGES: [f64; 8] = [0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0];
/// the lag 1 correlation shows from this many pairs, and a speed's median and longest from
/// this many intervals; a few say nothing
const LAG_PAIRS: usize = 20;
const SPEED_INTERVALS: usize = 20;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Movement {
    /// `[seconds from the view's start, dx, dy, magnitude]`, the extremes of each stretch kept
    pub motion: Vec<[f64; 4]>,
    /// `[from, to]` in seconds from the view's start where no report came for over 50 ms, so
    /// a drawing breaks its line there and nowhere else
    pub pauses: Vec<[f64; 2]>,
    pub path: Vec<[f64; 2]>,
    /// reports that carried movement by |dx| and by |dy|: 0 to 10, then more than 10
    pub dx: Vec<u32>,
    pub dy: Vec<u32>,
    /// the correlation of one report's dx (or dy) with the next one's, over the moving intervals'
    /// pairs, none under 20 pairs; smoothing pushes it towards 1, and so does a hand that
    /// changes speed slowly
    pub lag1_dx: Option<f64>,
    pub lag1_dy: Option<f64>,
    pub lag1_pairs: usize,
    pub speed: Vec<Speed>,
}

/// moving intervals whose neighbours moved at this speed
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Speed {
    /// counts per millisecond, none for an open end
    pub from: Option<f64>,
    pub to: Option<f64>,
    pub intervals: usize,
    pub median_us: Option<f64>,
    pub longest_us: Option<u32>,
}

pub fn movement(view: &View<'_>, moving: &Moving) -> Movement {
    let mut dx = vec![0; COUNT_BINS];
    let mut dy = vec![0; COUNT_BINS];
    for report in moving.reports.iter().filter(|r| r.moving()) {
        dx[(report.dx.unsigned_abs() as usize).min(COUNT_BINS - 1)] += 1;
        dy[(report.dy.unsigned_abs() as usize).min(COUNT_BINS - 1)] += 1;
    }
    let pairs = |axis: fn(&super::intervals::Report) -> i32| -> Vec<(f64, f64)> {
        moving
            .ends
            .iter()
            .map(|&end| {
                (
                    axis(&moving.reports[end - 1]) as f64,
                    axis(&moving.reports[end]) as f64,
                )
            })
            .collect()
    };
    let enough = moving.ends.len() >= LAG_PAIRS;
    Movement {
        dx,
        dy,
        lag1_dx: correlation(&pairs(|r| r.dx)).filter(|_| enough),
        lag1_dy: correlation(&pairs(|r| r.dy)).filter(|_| enough),
        lag1_pairs: moving.ends.len(),
        speed: speed(moving),
        ..preview(view)
    }
}

/// pearson's correlation, none when either side never varies
fn correlation(pairs: &[(f64, f64)]) -> Option<f64> {
    let n = pairs.len() as f64;
    if pairs.len() < 2 {
        return None;
    }
    // sums first: a constant side then has exactly no spread
    let (sa, sb) = pairs
        .iter()
        .fold((0.0, 0.0), |(a, b), p| (a + p.0, b + p.1));
    let (ma, mb) = (sa / n, sb / n);
    let (mut ab, mut aa, mut bb) = (0.0, 0.0, 0.0);
    for (a, b) in pairs {
        ab += (a - ma) * (b - mb);
        aa += (a - ma) * (a - ma);
        bb += (b - mb) * (b - mb);
    }
    (aa > 0.0 && bb > 0.0).then(|| ab / (aa * bb).sqrt())
}

/// the median and longest moving interval at each speed of the neighbours, so two conditions
/// that were not moved alike can still be set side by side speed for speed
fn speed(moving: &Moving) -> Vec<Speed> {
    let Some(period) = moving.period_us else {
        return Vec::new();
    };
    let mut bins: Vec<Vec<f64>> = vec![Vec::new(); SPEED_EDGES.len() + 1];
    for (&end, pace) in moving.ends.iter().zip(moving.paces()) {
        let Some(pace) = pace else { continue };
        let per_ms = pace * 1000.0 / period;
        let bin = SPEED_EDGES.partition_point(|&edge| edge <= per_ms);
        bins[bin].push(moving.gap(end) as f64);
    }
    bins.into_iter()
        .enumerate()
        .map(|(i, mut gaps)| {
            let enough = gaps.len() >= SPEED_INTERVALS;
            Speed {
                from: i.checked_sub(1).map(|i| SPEED_EDGES[i]),
                to: SPEED_EDGES.get(i).copied(),
                intervals: gaps.len(),
                longest_us: gaps
                    .iter()
                    .copied()
                    .reduce(f64::max)
                    .map(|g| g as u32)
                    .filter(|_| enough),
                median_us: median(&mut gaps).filter(|_| enough),
            }
        })
        .collect()
}

/// a preview of the counts over time, the extremes of each stretch of reports kept so a spike
/// never drops out, the pauses the line breaks at, and the path they trace
fn preview(view: &View<'_>) -> Movement {
    let mut motion = Vec::new();
    let mut pauses = Vec::new();
    let mut path = Vec::new();
    let mut last: Option<u64> = None;
    let seconds = |t: u64| (t - view.start()) as f64 / 1e6;
    let mut cumulative = [0.0; 2];
    let mut extremes: Vec<[f64; 4]> = Vec::new();
    let stride = (view.reports() / 1000).max(1);
    for (i, (time, dx, dy)) in view.motion().enumerate() {
        cumulative[0] += dx as f64;
        cumulative[1] += dy as f64;
        if let Some(last) = last.filter(|last| time - last > PAUSE_US) {
            extremes.sort_by(|a, b| a[0].total_cmp(&b[0]));
            extremes.dedup();
            motion.append(&mut extremes);
            pauses.push([seconds(last), seconds(time)]);
        }
        last = Some(time);
        let point = [
            seconds(time),
            dx as f64,
            dy as f64,
            (dx as f64).hypot(dy as f64),
        ];
        if extremes.is_empty() {
            extremes = vec![point; 5];
        } else {
            if point[1] < extremes[0][1] {
                extremes[0] = point;
            }
            if point[1] > extremes[1][1] {
                extremes[1] = point;
            }
            if point[2] < extremes[2][2] {
                extremes[2] = point;
            }
            if point[2] > extremes[3][2] {
                extremes[3] = point;
            }
            if point[3] > extremes[4][3] {
                extremes[4] = point;
            }
        }
        if (i + 1) % stride == 0 || i + 1 == view.reports() {
            extremes.sort_by(|a, b| a[0].total_cmp(&b[0]));
            extremes.dedup();
            motion.append(&mut extremes);
            path.push(cumulative);
        }
    }
    Movement {
        motion,
        pauses,
        path,
        ..Movement::default()
    }
}

#[cfg(test)]
mod tests;
