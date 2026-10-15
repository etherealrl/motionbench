use super::{
    intervals::{Moving, median},
    movement::{self, Movement},
    relay::Relay,
    timing::{self, Timing},
};
use crate::{
    analysis::compare::Selection,
    capture::session::{self, Data},
};
use serde::{Deserialize, Serialize};

/// the tabs over the readouts and the timeline. live, only the visible one is computed
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tab {
    #[default]
    Timing,
    Movement,
    Relay,
    Comparison,
}

/// which tabs a plot computes: none for a live frame between tab refreshes, one live, all for a
/// saved recording
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Views {
    Readouts,
    Tab(Tab),
    All,
}

impl Views {
    fn has(self, tab: Tab) -> bool {
        self == Views::All || self == Views::Tab(tab)
    }
}

/// axes two plots share so a baseline and a candidate line up: the timeline's interval band
/// and the jitter histogram's span either side of the period
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Scale {
    pub band: [u32; 2],
    pub jitter_us: f64,
}

/// the numbers above the tabs
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Readouts {
    /// live and unselected, about the last second of movement
    pub rate_hz: f64,
    pub period_us: Option<f64>,
    /// the median distance of a 1x moving interval from the period, as observed by the host
    pub jitter_us: Option<f64>,
    pub longest_moving_us: Option<u32>,
    /// the median of every gap, idle ones included
    pub median_gap_us: Option<f64>,
    pub moving: usize,
    pub gaps: usize,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Tabs {
    pub timing: Option<Timing>,
    pub movement: Option<Movement>,
    /// a saved recording with the box's input only, added by whoever holds that input
    pub relay: Option<Relay>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Plot {
    pub from_us: u64,
    pub to_us: u64,
    pub reports: usize,
    /// reports stamped before an earlier one, left out of every view
    pub regressions: u64,
    pub readouts: Readouts,
    pub scale: Scale,
    /// per column `[shortest, longest, median, moved]`, `moved` 1 when a moving report landed
    /// in it. none where no report completed
    pub timeline: Vec<Option<[u32; 4]>>,
    /// where the first timeline column starts and how wide each is, in the same clock as
    /// `from_us`, so a drawing can place the columns on their fixed grid
    pub timeline_start_us: u64,
    pub timeline_step_us: u64,
    pub tabs: Tabs,
    pub movement_available: bool,
}

/// the selected stream and the span a plot shows: a range when one is picked, else live the
/// last seconds up to where delivery has reached, else the whole recording
fn bounds(
    data: &Data,
    range: Option<&Selection>,
    live: bool,
    now: u64,
) -> Option<(crate::signal::stats::Key, u64, u64)> {
    let key = data.selected?;
    let stream = data.bus.streams.get(&key)?;
    let first = stream.first_us().unwrap_or(0);
    let last = stream.last_us().unwrap_or(first);
    let to = range.map_or_else(
        || match data.delivery.get(&key) {
            Some(delivery) if live => (now as i64 - delivery.delay_us).max(0) as u64,
            _ => last,
        },
        |r| r.to_us,
    );
    let from = range.map_or_else(
        || {
            if live {
                to.saturating_sub(crate::signal::stats::HISTORY_US)
            } else {
                first
            }
        },
        |r| r.from_us,
    );
    Some((key, from, to))
}

/// a copy of what a live plot reads, the selected stream's samples in the span it shows and
/// their delivery, taken under the capture's lock so the plot itself can be computed after it
/// is let go and the capture keeps reading meanwhile
pub fn detach(data: &Data, range: Option<&Selection>) -> Data {
    let mut out = Data::new(false);
    if let Some((key, from, to)) = bounds(data, range, true, session::wall_us()) {
        // live, past the span's end too, since the plot is computed a moment later
        let until = match range {
            Some(_) => to,
            None => to.saturating_add(crate::signal::stats::HISTORY_US),
        };
        out.bus
            .streams
            .insert(key, data.bus.streams[&key].copy(from, until));
        if let Some(delivery) = data.delivery.get(&key) {
            out.delivery.insert(key, *delivery);
        }
        out.selected = Some(key);
    }
    out
}

pub fn plot(
    data: &Data,
    range: Option<&Selection>,
    scale: Option<Scale>,
    live: bool,
    views: Views,
) -> Plot {
    plot_at(data, range, scale, live, views, session::wall_us())
}

pub(crate) fn plot_at(
    data: &Data,
    range: Option<&Selection>,
    scale: Option<Scale>,
    live: bool,
    views: Views,
    now: u64,
) -> Plot {
    let Some((key, from, to)) = bounds(data, range, live, now) else {
        return Plot::default();
    };
    let stream = &data.bus.streams[&key];
    let view = stream.view(from, to);
    let moving = Moving::of(&view);
    let deviations = timing::deviations(&moving);
    let mut gaps: Vec<f64> = view.gaps().map(f64::from).collect();
    let median_gap_us = median(&mut gaps);
    let band = moving
        .period_us
        .or(median_gap_us)
        .and_then(|m| view.on_time_range(m.round() as u32))
        .map_or([0, 1], |(lo, hi)| [lo, hi.max(lo + 1)]);
    let scale = scale.unwrap_or(Scale {
        band,
        jitter_us: timing::jitter_span(&deviations),
    });
    let (timeline_start_us, timeline_step_us, timeline) = view.timeline(800);
    let movement_available = stream.decoded > 0;
    Plot {
        from_us: from,
        to_us: to,
        reports: view.reports(),
        regressions: stream.regressions,
        readouts: Readouts {
            rate_hz: view.movement_rate_hz((live && range.is_none()).then_some(1_000_000)),
            period_us: moving.period_us,
            jitter_us: timing::mad(&deviations),
            longest_moving_us: moving.gaps().max(),
            median_gap_us,
            moving: moving.ends.len(),
            gaps: view.reports().saturating_sub(1),
        },
        scale,
        timeline,
        timeline_start_us,
        timeline_step_us,
        tabs: Tabs {
            // the burst navigator is for a saved recording or a paused range, not a sliding view
            timing: views
                .has(Tab::Timing)
                .then(|| timing::timing(&moving, scale.jitter_us, !live || range.is_some())),
            movement: (views.has(Tab::Movement) && movement_available)
                .then(|| movement::movement(&view, &moving)),
            relay: None,
        },
        movement_available,
    }
}

#[cfg(test)]
mod tests;
