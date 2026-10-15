//! what the live view computes each frame: the readouts, the timeline and the shown tab, on
//! axes that hold still once there is enough to fix them by

use super::plot::{Plot, Scale, Tab, Views};

/// the live axes hold still once this many moving intervals fixed them; fewer leave the jitter
/// span to a handful of intervals for the whole session
pub const SETTLED_INTERVALS: usize = 500;

#[derive(Default)]
pub struct Live {
    tab: Tab,
    scale: Option<Scale>,
    /// how many times the axes started again; a plot computed before the last one says nothing
    /// about the view now
    resets: u64,
}

impl Live {
    pub fn show(&mut self, tab: Tab) {
        self.tab = tab;
    }

    /// a new range or another mouse: the axes start again
    pub fn reset(&mut self) {
        self.scale = None;
        self.resets += 1;
    }

    /// what a plot computed now is for, given back to `settle`
    pub fn resets(&self) -> u64 {
        self.resets
    }

    pub fn scale(&self) -> Option<Scale> {
        self.scale
    }

    /// what a frame computes: the readouts, the timeline and the tab on show
    pub fn views(&self) -> Views {
        Views::Tab(self.tab)
    }

    /// fixes the axes once the plot has enough to fix them by. a recording without a decoder
    /// has no moving intervals, its band is fixed from its reports alone. `resets` is what
    /// `resets()` said when the plot was computed: one from before a reset is ignored
    pub fn settle(&mut self, plot: &Plot, resets: u64) {
        let settled = plot.readouts.moving >= SETTLED_INTERVALS
            || (!plot.movement_available && plot.reports > 20);
        if self.scale.is_none() && settled && resets == self.resets {
            self.scale = Some(plot.scale);
        }
    }
}

#[cfg(test)]
mod tests;
