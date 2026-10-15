use std::collections::{BTreeMap, VecDeque};
use std::ops::Range;

/// live history is kept by time, not by sample count, so the view spans the same seconds at any
/// rate. recorded plots retain their decoded samples; judge ingestion streams windows
pub const HISTORY_US: u64 = 8_000_000;
const MAX_SAMPLES: usize = 1 << 21;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Key {
    pub bus: u16,
    pub device: u16,
    pub endpoint: u8,
}

#[derive(Clone, Copy)]
struct Sample {
    t_us: u64,
    /// every byte past the first was zero, so nothing moved and no wheel turned
    still: bool,
    /// zero for reports without decoded axes
    dx: i32,
    dy: i32,
}

pub struct Stream {
    pub payload_len: usize,
    pub total: u64,
    /// samples whose axes were decoded, so a plot can tell a still mouse from one it cannot read
    pub decoded: u64,
    /// samples stamped before the latest one, left out because every view and tick count
    /// assumes time only moves forward
    pub regressions: u64,
    retain: u64,
    /// samples from here on stay past `retain` while a paused view still shows them, up to
    /// the sample cap
    pinned_from: Option<u64>,
    samples: VecDeque<Sample>,
}

impl Default for Stream {
    fn default() -> Self {
        Stream::new(HISTORY_US)
    }
}

impl Stream {
    pub fn push_decoded(&mut self, t_us: u64, motion: Option<(i32, i32)>) {
        let (dx, dy) = motion.unwrap_or((0, 0));
        self.push_sample(
            Sample {
                t_us,
                still: motion.is_some_and(|m| m == (0, 0)),
                dx,
                dy,
            },
            motion.is_some(),
        );
    }
    pub fn new(retain: u64) -> Self {
        Stream {
            payload_len: 0,
            total: 0,
            decoded: 0,
            regressions: 0,
            retain,
            pinned_from: None,
            samples: VecDeque::new(),
        }
    }

    /// a report of the test traffic's layout, decoded as it goes in
    #[cfg(any(test, feature = "testkit"))]
    pub fn push(&mut self, t_us: u64, payload: &[u8]) {
        self.payload_len = payload.len();
        let motion = crate::capture::pcap::motion(payload);
        let (dx, dy) = motion.unwrap_or((0, 0));
        self.push_sample(
            Sample {
                t_us,
                still: payload.len() > 1 && payload[1..].iter().all(|b| *b == 0),
                dx: dx as i32,
                dy: dy as i32,
            },
            motion.is_some(),
        );
    }

    /// a report the box saw on its side of the mouse, already decoded, on the box's own clock
    pub fn push_motion(&mut self, t_us: u64, dx: i16, dy: i16, dz: i16) {
        self.push_sample(
            Sample {
                t_us,
                still: dx == 0 && dy == 0 && dz == 0,
                dx: dx as i32,
                dy: dy as i32,
            },
            true,
        );
    }

    fn push_sample(&mut self, sample: Sample, decoded: bool) {
        let t_us = sample.t_us;
        if self.last_us().is_some_and(|last| t_us < last) {
            self.regressions += 1;
            return;
        }
        self.total += 1;
        self.decoded += u64::from(decoded);
        self.samples.push_back(sample);
        while (self.retain != u64::MAX && self.samples.len() > MAX_SAMPLES)
            || self.samples.front().is_some_and(|first| {
                t_us.saturating_sub(first.t_us) > self.retain
                    && self.pinned_from.is_none_or(|from| first.t_us < from)
            })
        {
            self.samples.pop_front();
        }
    }

    pub fn first_us(&self) -> Option<u64> {
        self.samples.front().map(|s| s.t_us)
    }

    pub fn last_us(&self) -> Option<u64> {
        self.samples.back().map(|s| s.t_us)
    }

    /// what the wire really delivered in the last second, used to spot which device is the mouse
    pub fn reports_last_second(&self) -> usize {
        let Some(last) = self.last_us() else {
            return 0;
        };
        self.samples
            .iter()
            .rev()
            .take_while(|s| last - s.t_us < 1_000_000)
            .count()
    }

    /// the samples from `from` to `to` in a stream of their own, kept whole, with this
    /// stream's counters
    pub fn copy(&self, from: u64, to: u64) -> Stream {
        let start = self.samples.partition_point(|s| s.t_us < from);
        let end = self.samples.partition_point(|s| s.t_us <= to).max(start);
        Stream {
            payload_len: self.payload_len,
            total: self.total,
            decoded: self.decoded,
            regressions: self.regressions,
            retain: u64::MAX,
            pinned_from: None,
            samples: self.samples.range(start..end).copied().collect(),
        }
    }

    pub fn view(&self, from: u64, to: u64) -> View<'_> {
        let start = self.samples.partition_point(|s| s.t_us < from);
        let end = self.samples.partition_point(|s| s.t_us <= to);
        View {
            samples: &self.samples,
            range: start..end.max(start),
            from,
            to,
        }
    }
}

/// every number on screen comes from one of these, so the readout and the plot always describe
/// the same slice of time
pub struct View<'a> {
    samples: &'a VecDeque<Sample>,
    range: Range<usize>,
    from: u64,
    to: u64,
}

impl View<'_> {
    /// usb reports per second while the mouse moves: one over the median interval between back
    /// to back reports that both carry movement. a pause over 50 ms splits movement, it is no
    /// interval. the median keeps slow movement, which leaves polls empty, from pulling the
    /// rate down. with `recent`, only the last that many microseconds of movement count, so a
    /// live readout says what the mouse does now, not over the whole view
    pub fn movement_rate_hz(&self, recent: Option<u64>) -> f64 {
        let moving = |s: &Sample| s.dx != 0 || s.dy != 0;
        let mut gaps = Vec::new();
        // the window ends at the latest report that moved, even one with no moving neighbour,
        // so an older burst never stands in for what the mouse does now
        let latest = self
            .samples
            .range(self.range.clone())
            .rev()
            .find(|s| moving(s))
            .map(|s| s.t_us);
        for i in (self.range.start + 1..self.range.end).rev() {
            let (a, b) = (&self.samples[i - 1], &self.samples[i]);
            if let (Some(span), Some(latest)) = (recent, latest)
                && latest.saturating_sub(b.t_us) >= span
            {
                break;
            }
            let gap = b.t_us.saturating_sub(a.t_us);
            if moving(a) && moving(b) && gap > 0 && gap <= 50_000 {
                gaps.push(gap);
            }
        }
        if gaps.is_empty() {
            return 0.0;
        }
        gaps.sort_unstable();
        let mid = gaps.len() / 2;
        let median = if gaps.len() % 2 == 0 {
            (gaps[mid - 1] + gaps[mid]) as f64 / 2.0
        } else {
            gaps[mid] as f64
        };
        1e6 / median
    }

    pub fn seconds(&self) -> f64 {
        self.to.saturating_sub(self.from) as f64 / 1_000_000.0
    }

    pub fn start(&self) -> u64 {
        self.from
    }

    /// the last report inside the view, which is where its data ends whatever `to` says
    pub fn last(&self) -> Option<u64> {
        self.range
            .end
            .checked_sub(1)
            .filter(|i| *i >= self.range.start)
            .map(|i| self.samples[i].t_us)
    }

    pub fn reports(&self) -> usize {
        self.range.len()
    }

    pub fn still(&self) -> usize {
        self.samples
            .range(self.range.clone())
            .filter(|s| s.still)
            .count()
    }

    pub fn motion(&self) -> impl Iterator<Item = (u64, i32, i32)> + '_ {
        self.samples
            .range(self.range.clone())
            .map(|s| (s.t_us, s.dx, s.dy))
    }

    pub fn gaps(&self) -> impl Iterator<Item = u32> + '_ {
        let tail = (self.range.start + 1).min(self.range.end)..self.range.end;
        self.samples
            .range(self.range.clone())
            .zip(self.samples.range(tail))
            .map(|(a, b)| b.t_us.saturating_sub(a.t_us).min(u32::MAX as u64) as u32)
    }

    /// spread of the polls that landed in their own slot, which is what jitter actually means
    pub fn on_time_range(&self, median: u32) -> Option<(u32, u32)> {
        let (half, high) = (median as u64, median as u64 * 3);
        let mut on: Vec<u32> = self
            .gaps()
            .filter(|g| {
                let g = *g as u64 * 2;
                g > half && g < high
            })
            .collect();
        if on.is_empty() {
            return None;
        }
        on.sort_unstable();
        let trim = on.len() / 100;
        let (lo, hi) = (on[trim], on[on.len() - 1 - trim]);
        Some(if lo == hi {
            (lo.saturating_sub(1), hi.saturating_add(1))
        } else {
            (lo, hi)
        })
    }

    /// `[shortest, longest, median, moved]` interval in each column of about `columns` columns,
    /// `moved` 1 when a report that landed there carried movement. a column with no reports
    /// stays none, because silence on the wire has to look like silence, not like a missing
    /// sample. the columns sit on a fixed grid of absolute time, starting at the returned time
    /// and the returned step apart, so a report stays in its column while a live view slides
    /// past it; columns measured from the moving left edge regrouped the reports every frame
    /// and made spikes flicker
    pub fn timeline(&self, columns: usize) -> (u64, u64, Vec<Option<[u32; 4]>>) {
        let span = self.to.saturating_sub(self.from);
        if span == 0 || columns == 0 {
            return (self.from, 0, Vec::new());
        }
        let step = (span / columns as u64).max(1);
        let start = self.from / step * step;
        let count = (self.to - start).div_ceil(step) as usize;
        let tail = (self.range.start + 1).min(self.range.end)..self.range.end;
        let mut out = vec![None; count];
        // reports arrive in time order, so each column's gaps come in one run
        let mut gaps = Vec::new();
        let mut moved = false;
        let mut current = None;
        for (sample, gap) in self.samples.range(tail).zip(self.gaps()) {
            let column = (((sample.t_us.saturating_sub(start)) / step) as usize).min(count - 1);
            if let Some(previous) = current.filter(|c| *c != column) {
                out[previous] = column_summary(&mut gaps, moved);
                moved = false;
            }
            current = Some(column);
            gaps.push(gap);
            moved |= sample.dx != 0 || sample.dy != 0;
        }
        if let Some(column) = current {
            out[column] = column_summary(&mut gaps, moved);
        }
        (start, step, out)
    }
}

fn column_summary(gaps: &mut Vec<u32>, moved: bool) -> Option<[u32; 4]> {
    let (lo, hi) = (*gaps.iter().min()?, *gaps.iter().max()?);
    let middle = gaps.len() / 2;
    let upper = *gaps.select_nth_unstable(middle).1;
    // an even count takes the mean of the two middle gaps, like every other median here
    let median = match gaps[..middle].iter().max() {
        Some(&lower) if gaps.len().is_multiple_of(2) => ((lower as u64 + upper as u64) / 2) as u32,
        _ => upper,
    };
    gaps.clear();
    Some([lo, hi, median, u32::from(moved)])
}

pub struct Bus {
    pub streams: BTreeMap<Key, Stream>,
    retain: u64,
    pinned: Option<u64>,
}

impl Default for Bus {
    fn default() -> Self {
        Bus::live()
    }
}

impl Bus {
    pub fn retain_live(&self) -> bool {
        self.retain != u64::MAX
    }

    /// keeps every stream's samples from `from` on while a paused view shows them, or lets
    /// them go again. a range picked inside a paused one keeps the wider one whole, so the
    /// view can go back to it
    pub fn pin(&mut self, from: Option<u64>) {
        self.pinned = from.map(|from| self.pinned.map_or(from, |pinned| pinned.min(from)));
        for stream in self.streams.values_mut() {
            stream.pinned_from = self.pinned;
        }
    }

    pub fn live() -> Self {
        Bus {
            streams: BTreeMap::new(),
            retain: HISTORY_US,
            pinned: None,
        }
    }

    /// a replayed recording is not a sliding view of now, it is the whole thing
    pub fn whole() -> Self {
        Bus {
            streams: BTreeMap::new(),
            retain: u64::MAX,
            pinned: None,
        }
    }

    #[cfg(any(test, feature = "testkit"))]
    pub fn push(&mut self, report: &crate::capture::pcap::Report) {
        let key = Key {
            bus: report.bus,
            device: report.device,
            endpoint: report.endpoint,
        };
        let retain = self.retain;
        self.streams
            .entry(key)
            .or_insert_with(|| Stream::new(retain))
            .push(report.t_us, report.bytes());
    }

    pub fn span(&self) -> Option<(u64, u64)> {
        let first = self.streams.values().filter_map(|s| s.first_us()).min()?;
        let last = self.streams.values().filter_map(|s| s.last_us()).max()?;
        Some((first, last))
    }

    /// the mouse is whatever reports fastest while you move it
    pub fn busiest(&self) -> Option<Key> {
        self.streams
            .iter()
            .max_by_key(|(_, s)| s.reports_last_second())
            .map(|(k, _)| *k)
    }
}

#[cfg(test)]
mod tests;
