//! the Relay tab: what the box saw on its side of the mouse against what reached the wire, on
//! the wire's clock through the recording's fixed shift. only a recording with the box's input
//! has it. the shift absorbs the constant part of the delay, so only its variation shows; no
//! absolute latency is measured here

use super::intervals::{Report, median, nearest_rank};
use super::timing::{centred, jitter_span};
use crate::signal::stats::{Stream, View};
use serde::{Deserialize, Serialize};

const COLUMNS: u64 = 800;
const WINDOW_US: u64 = 100_000;
/// a box report and a wire report pair up within this far of each other when no period is
/// known
const TOLERANCE_US: f64 = 1000.0;
/// the box's input is read this far past the range, more than any pair is apart
const MARGIN_US: u64 = 50_000;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Relay {
    /// dx and dy summed over the view, on each side
    pub box_counts: [i64; 2],
    pub wire_counts: [i64; 2],
    /// `[seconds from the view's start, box minus wire dx, box minus wire dy]`, summed from the
    /// view's start with the box's reports moved by the median offset, the smallest difference
    /// in each of about 800 columns. a lossless relay stays at zero
    pub conserved: Vec<[f64; 3]>,
    /// box minus wire dx and dy in each 100 ms window from the view's start, the box's reports
    /// moved by the same median offset
    pub windows: Vec<[i64; 2]>,
    /// moving reports paired with the nearest wire report of the same counts within one period
    /// of where they landed, and those left alone on either side
    pub matched: usize,
    pub box_only: usize,
    pub wire_only: usize,
    /// how far each pair's offset sat from the median offset
    pub offset: Offset,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Offset {
    pub span_us: f64,
    /// centred bins of whole microseconds, the middle one the median offset
    pub bins: Vec<u32>,
    pub outside: u32,
    pub p01_us: Option<f64>,
    pub p99_us: Option<f64>,
}

/// the box's input moved onto the wire's clock by the recording's fixed shift; a row the shift
/// would put before the clock's start is left out
pub fn aligned(input: &Stream, shift_us: i64) -> Stream {
    let mut out = Stream::new(u64::MAX);
    for (t, dx, dy) in input.view(0, u64::MAX).motion() {
        if let Ok(time) = u64::try_from(t as i128 - shift_us as i128) {
            out.push_decoded(time, Some((dx, dy)));
        }
    }
    out
}

fn moving(view: &View<'_>) -> Vec<Report> {
    view.motion()
        .map(|(t_us, dx, dy)| Report { t_us, dx, dy })
        .filter(Report::moving)
        .collect()
}

/// the wire report of the same counts nearest `at`, within `tolerance` either side, and not
/// `taken` yet
fn nearest(wire: &[Report], b: &Report, at: f64, tolerance: f64, taken: &[bool]) -> Option<usize> {
    let start = wire.partition_point(|w| (w.t_us as f64) < at - tolerance);
    (start..wire.len())
        .take_while(|&i| wire[i].t_us as f64 <= at + tolerance)
        .filter(|&i| !taken[i] && wire[i].dx == b.dx && wire[i].dy == b.dy)
        .min_by(|&i, &j| {
            (wire[i].t_us as f64 - at)
                .abs()
                .total_cmp(&(wire[j].t_us as f64 - at).abs())
        })
}

/// `wire` is the range on the wire; `input` the box's whole input already on the wire's clock,
/// read past the range so a report whose partner is inside is never cut off. `period_us` sets
/// how far apart a pair may be, one period either side of where the box report landed
pub fn relay(wire: &View<'_>, input: &Stream, period_us: Option<f64>) -> Relay {
    let from = wire.start();
    let to = from + (wire.seconds() * 1e6).round() as u64;
    let tolerance = period_us.unwrap_or(TOLERANCE_US);
    let wire = moving(wire);
    let around = moving(&input.view(from.saturating_sub(MARGIN_US), to.saturating_add(MARGIN_US)));
    // the constant part of the offset, the median from each box report to the wire report of
    // its counts nearest it. the recording's fixed shift leaves less than half a period here;
    // more, and a run of equal counts would pair each report with its neighbour's; the box's reports are moved by it before anything is counted, so a
    // count still in flight is not counted as lost and pairs are looked for where they land
    let free = vec![false; wire.len()];
    let mut nearby: Vec<f64> = around
        .iter()
        .filter_map(|b| {
            nearest(&wire, b, b.t_us as f64, tolerance, &free)
                .map(|i| wire[i].t_us as f64 - b.t_us as f64)
        })
        .collect();
    let lead = median(&mut nearby).unwrap_or(0.0).round() as i64;
    let landed = |r: &Report| r.t_us as i64 + lead;
    let mut out = Relay::default();
    let mut taken = vec![false; wire.len()];
    let mut offsets = Vec::new();
    // a box report belongs to the range when it landed in it, or, landing just outside, when
    // a wire report in the range is left for it once those inside are paired; so a partner is
    // never cut off and never taken from a report inside
    let reach = tolerance.ceil() as i64;
    let inside = |r: &Report| (from as i64..=to as i64).contains(&landed(r));
    let edge =
        |r: &Report| !inside(r) && (from as i64 - reach..=to as i64 + reach).contains(&landed(r));
    let mut boxed = Vec::new();
    for (b, alone_counts) in around
        .iter()
        .filter(|r| inside(r))
        .map(|r| (r, true))
        .chain(around.iter().filter(|r| edge(r)).map(|r| (r, false)))
    {
        match nearest(&wire, b, landed(b) as f64, tolerance, &taken) {
            Some(i) => {
                taken[i] = true;
                offsets.push(wire[i].t_us as f64 - b.t_us as f64);
                boxed.push(*b);
            }
            None if alone_counts => {
                out.box_only += 1;
                boxed.push(*b);
            }
            None => {}
        }
    }
    boxed.sort_by_key(|r| r.t_us);
    let sum = |reports: &[Report]| {
        reports
            .iter()
            .fold([0i64; 2], |s, r| [s[0] + r.dx as i64, s[1] + r.dy as i64])
    };
    out.box_counts = sum(&boxed);
    out.wire_counts = sum(&wire);
    out.matched = offsets.len();
    out.wire_only = taken.iter().filter(|t| !**t).count();
    if let Some(middle) = median(&mut offsets.clone()) {
        let mut spread: Vec<f64> = offsets.iter().map(|o| o - middle).collect();
        let span_us = jitter_span(&spread);
        let (bins, outside) = centred(&spread, span_us);
        out.offset = Offset {
            span_us,
            bins,
            outside,
            p01_us: nearest_rank(&mut spread, 0.01),
            p99_us: nearest_rank(&mut spread, 0.99),
        };
    }
    // the running difference after each report, box ones where they landed
    let mut events: Vec<(i64, [i64; 2])> = boxed
        .iter()
        .map(|r| (landed(r), [r.dx as i64, r.dy as i64]))
        .chain(
            wire.iter()
                .map(|r| (r.t_us as i64, [-(r.dx as i64), -(r.dy as i64)])),
        )
        .collect();
    events.sort_by_key(|e| e.0);
    let step = (to.saturating_sub(from) / COLUMNS).max(1);
    let mut running = [0i64; 2];
    let mut next = events.iter().peekable();
    let mut end = from;
    while end < to {
        end = (end + step).min(to);
        // the smallest difference seen in the column: a count in flight between its two
        // reports comes and goes inside it, a lost one stays in every value after it
        let mut least = running;
        while let Some(&&(at, delta)) = next.peek() {
            if at > end as i64 {
                break;
            }
            next.next();
            running = [running[0] + delta[0], running[1] + delta[1]];
            if running[0].abs() + running[1].abs() < least[0].abs() + least[1].abs() {
                least = running;
            }
        }
        out.conserved
            .push([(end - from) as f64 / 1e6, least[0] as f64, least[1] as f64]);
    }
    out.windows = vec![[0; 2]; to.saturating_sub(from).div_ceil(WINDOW_US) as usize];
    for &(at, delta) in &events {
        let window = (at.max(from as i64) as u64 - from) / WINDOW_US;
        if let Some(window) = out.windows.get_mut(window as usize) {
            window[0] += delta[0];
            window[1] += delta[1];
        }
    }
    out
}

#[cfg(test)]
mod tests;
