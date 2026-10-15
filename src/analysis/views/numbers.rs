//! the Comparison tab's numbers: one recording's figures from its plot, so a table can set
//! every source recording side by side with each condition's median

use super::plot::Plot;
use serde::{Deserialize, Serialize};

/// none wherever the plot has no such figure
#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct Numbers {
    pub rate_hz: Option<f64>,
    pub period_us: Option<f64>,
    pub jitter_us: Option<f64>,
    pub longest_moving_us: Option<f64>,
    /// shares of the moving intervals
    pub bunched: Option<f64>,
    pub beyond_1x: Option<f64>,
    /// the share of 1 ms ticks while moving that held no report
    pub empty_ticks: Option<f64>,
    /// counts held after a gap against the neighbours' pace
    pub after_gap: Option<f64>,
    pub lag1_dx: Option<f64>,
    pub moving: usize,
}

fn share(part: u32, whole: u32) -> Option<f64> {
    (whole > 0).then(|| part as f64 / whole as f64)
}

pub fn numbers(plot: &Plot) -> Numbers {
    let readouts = &plot.readouts;
    let timing = plot.tabs.timing.as_ref();
    let groups = timing.map(|t| t.groups).unwrap_or_default();
    let moving: u32 = groups.iter().sum();
    let ticks = timing.map(|t| t.ticks).unwrap_or_default();
    Numbers {
        rate_hz: (readouts.rate_hz > 0.0).then_some(readouts.rate_hz),
        period_us: readouts.period_us,
        jitter_us: readouts.jitter_us,
        longest_moving_us: readouts.longest_moving_us.map(f64::from),
        bunched: share(groups[0], moving),
        beyond_1x: share(groups[2..].iter().sum(), moving),
        empty_ticks: share(ticks[0], ticks.iter().sum()),
        after_gap: timing.and_then(|t| t.density.after_gap.as_ref().map(|a| a.ratio)),
        lag1_dx: plot.tabs.movement.as_ref().and_then(|m| m.lag1_dx),
        moving: readouts.moving,
    }
}

#[cfg(test)]
mod tests;
