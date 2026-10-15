//! the Timing tab: how the host observed the moving intervals. a gap is an observed gap, never
//! a skipped or missed poll, and jitter includes the host's own scheduling

use super::intervals::{Moving, PAUSE_US, group, median, nearest_rank};
use serde::{Deserialize, Serialize};

const DENSITY_COLUMNS: usize = 64;
const DENSITY_ROWS: usize = 48;
/// the density's interval axis runs to this many periods
const DENSITY_PERIODS: f64 = 4.0;
/// a gap of at least this many periods is "after a gap" in the density's readout
const AFTER_GAP_PERIODS: f64 = 1.5;
/// the burst navigator lists this many and counts the rest
const BURST_ROWS: usize = 500;
/// a burst's rate shows only from this many intervals, a few say little about a rate
const BURST_RATE_INTERVALS: usize = 20;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Timing {
    pub jitter: Jitter,
    /// moving intervals under 0.75 periods (bunched), about 1, 2 and 3 periods, and 4 or more
    pub groups: [u32; 5],
    /// `[threshold_us, share]`: the share of moving intervals longer than each threshold, on a
    /// log grid from 50 µs to 50 ms
    pub longer: Vec<[f64; 2]>,
    pub density: Density,
    /// 1 ms ticks while moving that hold 0, 1, 2, and 3 or more reports, on the judge's tick
    /// grid from the first report of the view
    pub ticks: [u32; 4],
    /// none live, the navigator is for saved recordings and a paused range
    pub bursts: Option<Bursts>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Jitter {
    /// the bins span this many microseconds either side of the period
    pub span_us: f64,
    /// bins of whole microseconds centred on whole multiples of their width, the middle one on
    /// the period itself
    pub bins: Vec<u32>,
    /// 1x intervals beyond the span
    pub outside: u32,
    /// the median distance of a 1x interval from the period
    pub mad_us: Option<f64>,
    pub p01_us: Option<f64>,
    pub p99_us: Option<f64>,
    /// the 1x moving intervals it holds, and the moving intervals that are not 1x
    pub intervals: usize,
    pub left_out: usize,
}

/// displacement against interval, binned: x the neighbours' counts per period, y the interval
/// in periods, rows from the bottom
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Density {
    pub columns: usize,
    pub rows: usize,
    /// x runs from 0 to this many counts per period, y from 0 to `periods`. a larger x is in the
    /// last column
    pub x_max: f64,
    pub periods: f64,
    pub cells: Vec<u32>,
    /// the moving intervals placed, those of `periods` or more among them, and those left out
    /// because neither neighbour ends a moving interval
    pub intervals: usize,
    pub outside: u32,
    pub alone: usize,
    pub after_gap: Option<AfterGap>,
}

/// after a gap of at least 1.5 periods, how many counts the next report held against what its
/// neighbours' counts per period predict for one report
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct AfterGap {
    /// the mean gap, in periods
    pub periods: f64,
    pub ratio: f64,
    pub intervals: usize,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Bursts {
    pub rows: Vec<Burst>,
    /// bursts past the listed ones
    pub more: usize,
}

/// a stretch of movement: moving reports no more than 50 ms apart
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Burst {
    pub from_us: u64,
    pub to_us: u64,
    /// every report from its first moving report to its last
    pub reports: usize,
    pub intervals: usize,
    pub median_us: Option<f64>,
    pub longest_us: u32,
    /// shown from 20 intervals
    pub rate_hz: Option<f64>,
}

/// the deviation of each 1x moving interval from the period, in microseconds
pub fn deviations(moving: &Moving) -> Vec<f64> {
    let Some(period) = moving.period_us else {
        return Vec::new();
    };
    moving
        .gaps()
        .map(f64::from)
        .filter(|&gap| group(gap, period) == 1)
        .map(|gap| gap - period)
        .collect()
}

/// the median distance of a 1x interval from the period
pub fn mad(deviations: &[f64]) -> Option<f64> {
    median(&mut deviations.iter().map(|d| d.abs()).collect::<Vec<_>>())
}

/// a span that holds the p01 to p99 values, rounded up to a whole number of whole
/// microsecond bins, since the host stamps reports in whole microseconds
pub fn jitter_span(deviations: &[f64]) -> f64 {
    let mut values = deviations.to_vec();
    let low = nearest_rank(&mut values, 0.01).unwrap_or(0.0);
    let high = nearest_rank(&mut values, 0.99).unwrap_or(0.0);
    let span = low.abs().max(high.abs()).max(2.0);
    let width = bin_width(span);
    (span / width).ceil() * width
}

fn bin_width(span: f64) -> f64 {
    (span / 30.0).ceil().max(1.0)
}

pub fn timing(moving: &Moving, span_us: f64, bursts: bool) -> Timing {
    Timing {
        jitter: jitter(moving, span_us),
        groups: groups(moving),
        longer: longer(moving),
        density: density(moving),
        ticks: ticks(moving),
        bursts: bursts.then(|| navigator(moving)),
    }
}

/// values binned either side of zero on whole microsecond bins of `span_us`'s width, the
/// middle bin centred on zero, and how many fell beyond the span
pub fn centred(values: &[f64], span_us: f64) -> (Vec<u32>, u32) {
    let width = bin_width(span_us);
    let half = (span_us / width).round() as i64;
    let mut bins = vec![0u32; (2 * half + 1) as usize];
    let mut outside = 0;
    for &v in values {
        let bin = (v / width).round() as i64;
        if bin.abs() > half {
            outside += 1;
        } else {
            bins[(bin + half) as usize] += 1;
        }
    }
    (bins, outside)
}

fn jitter(moving: &Moving, span_us: f64) -> Jitter {
    let mut deviations = deviations(moving);
    let (bins, outside) = centred(&deviations, span_us);
    Jitter {
        span_us,
        bins,
        outside,
        mad_us: mad(&deviations),
        p01_us: nearest_rank(&mut deviations, 0.01),
        p99_us: nearest_rank(&mut deviations, 0.99),
        intervals: deviations.len(),
        left_out: moving.ends.len() - deviations.len(),
    }
}

fn groups(moving: &Moving) -> [u32; 5] {
    let mut groups = [0; 5];
    if let Some(period) = moving.period_us {
        for gap in moving.gaps() {
            groups[group(gap as f64, period)] += 1;
        }
    }
    groups
}

fn longer(moving: &Moving) -> Vec<[f64; 2]> {
    let (low, high) = (50f64.ln(), (PAUSE_US as f64).ln());
    let thresholds: Vec<f64> = (0..=60)
        .map(|i| (low + (high - low) * i as f64 / 60.0).exp())
        .collect();
    // one pass, no sort: each gap counts towards every threshold below it
    let mut above = vec![0usize; thresholds.len() + 1];
    let mut total = 0;
    for gap in moving.gaps() {
        above[thresholds.partition_point(|&t| t < gap as f64)] += 1;
        total += 1;
    }
    if total == 0 {
        return Vec::new();
    }
    let mut longer = total;
    thresholds
        .iter()
        .enumerate()
        .map(|(i, &threshold)| {
            longer -= above[i];
            [threshold, longer as f64 / total as f64]
        })
        .collect()
}

fn density(moving: &Moving) -> Density {
    let mut out = Density {
        columns: DENSITY_COLUMNS,
        rows: DENSITY_ROWS,
        periods: DENSITY_PERIODS,
        cells: vec![0; DENSITY_COLUMNS * DENSITY_ROWS],
        ..Density::default()
    };
    let Some(period) = moving.period_us else {
        return out;
    };
    let mut points = Vec::with_capacity(moving.ends.len());
    for (&end, pace) in moving.ends.iter().zip(moving.paces()) {
        let Some(predicted) = pace else {
            out.alone += 1;
            continue;
        };
        points.push((
            predicted,
            moving.gap(end) as f64 / period,
            moving.reports[end].counts(),
        ));
    }
    out.intervals = points.len();
    let mut xs: Vec<f64> = points.iter().map(|p| p.0).collect();
    out.x_max = nearest_rank(&mut xs, 0.99).unwrap_or(0.0).ceil().max(1.0);
    let (mut held, mut predicted, mut periods, mut after) = (0.0, 0.0, 0.0, 0);
    for &(x, y, counts) in &points {
        if y >= AFTER_GAP_PERIODS {
            held += counts;
            predicted += x;
            periods += y;
            after += 1;
        }
        if y >= DENSITY_PERIODS {
            out.outside += 1;
            continue;
        }
        let column = ((x / out.x_max * DENSITY_COLUMNS as f64) as usize).min(DENSITY_COLUMNS - 1);
        let row = (y / DENSITY_PERIODS * DENSITY_ROWS as f64) as usize;
        out.cells[row * DENSITY_COLUMNS + column] += 1;
    }
    out.after_gap = (after > 0 && predicted > 0.0).then(|| AfterGap {
        periods: periods / after as f64,
        ratio: held / predicted,
        intervals: after,
    });
    out
}

fn ticks(moving: &Moving) -> [u32; 4] {
    let Some(origin) = moving.reports.first().map(|r| r.t_us) else {
        return [0; 4];
    };
    let tick = |t_us: u64| crate::signal::ticks::tick(origin, t_us).unwrap_or(0);
    let count = tick(moving.reports.last().unwrap().t_us) + 1;
    let mut held = vec![0u32; count];
    for report in &moving.reports {
        held[tick(report.t_us)] += 1;
    }
    let mut during = vec![false; count];
    for &end in &moving.ends {
        let (from, to) = (moving.reports[end - 1].t_us, moving.reports[end].t_us);
        during[tick(from)..=tick(to)].fill(true);
    }
    let mut out = [0; 4];
    for (n, _) in held.iter().zip(&during).filter(|(_, during)| **during) {
        out[(*n as usize).min(3)] += 1;
    }
    out
}

fn navigator(moving: &Moving) -> Bursts {
    let mut bursts = Bursts::default();
    let mut ends = moving.ends.iter().peekable();
    let moving_at: Vec<usize> = (0..moving.reports.len())
        .filter(|&i| moving.reports[i].moving())
        .collect();
    let mut start = 0;
    for k in 0..moving_at.len() {
        let closes = k + 1 == moving_at.len()
            || moving.reports[moving_at[k + 1]].t_us - moving.reports[moving_at[k]].t_us > PAUSE_US;
        if !closes {
            continue;
        }
        let (first, last) = (moving_at[start], moving_at[k]);
        start = k + 1;
        let mut gaps = Vec::new();
        while let Some(&&end) = ends.peek() {
            if end > last {
                break;
            }
            ends.next();
            if end > first {
                gaps.push(moving.gap(end) as f64);
            }
        }
        if gaps.is_empty() {
            continue;
        }
        if bursts.rows.len() == BURST_ROWS {
            bursts.more += 1;
            continue;
        }
        let longest_us = gaps.iter().copied().fold(0.0, f64::max) as u32;
        let intervals = gaps.len();
        let median_us = median(&mut gaps);
        bursts.rows.push(Burst {
            from_us: moving.reports[first].t_us,
            to_us: moving.reports[last].t_us,
            reports: last - first + 1,
            intervals,
            median_us,
            longest_us,
            rate_hz: median_us
                .filter(|_| intervals >= BURST_RATE_INTERVALS)
                .map(|m| 1e6 / m),
        });
    }
    bursts
}

#[cfg(test)]
mod tests;
