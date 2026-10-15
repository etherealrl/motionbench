use crate::signal::stats::View;

/// a pause longer than this splits movement, it is no interval. the active rate uses the same
/// rule
pub const PAUSE_US: u64 = 50_000;

/// one report of the view: when the host saw it and the counts it carried, zero when its axes
/// were not decoded
#[derive(Clone, Copy)]
pub struct Report {
    pub t_us: u64,
    pub dx: i32,
    pub dy: i32,
}

impl Report {
    pub fn moving(&self) -> bool {
        self.dx != 0 || self.dy != 0
    }

    pub fn counts(&self) -> f64 {
        (self.dx as f64).hypot(self.dy as f64)
    }
}

/// the reports of a view and its moving intervals: the gaps between two back to back reports
/// that both carry decoded movement, at most 50 ms apart. every timing view uses this population
pub struct Moving {
    pub reports: Vec<Report>,
    /// the later report of each moving interval, in order
    pub ends: Vec<usize>,
    /// the median moving interval, which stands for the period wherever one is meant, so the 1x
    /// labels agree with the active rate
    pub period_us: Option<f64>,
}

impl Moving {
    pub fn of(view: &View<'_>) -> Self {
        let reports: Vec<Report> = view
            .motion()
            .map(|(t_us, dx, dy)| Report { t_us, dx, dy })
            .collect();
        let ends: Vec<usize> = (1..reports.len())
            .filter(|&i| {
                let (a, b) = (reports[i - 1], reports[i]);
                let gap = b.t_us - a.t_us;
                a.moving() && b.moving() && gap > 0 && gap <= PAUSE_US
            })
            .collect();
        let mut moving = Moving {
            reports,
            ends,
            period_us: None,
        };
        moving.period_us = median(&mut moving.gaps().map(f64::from).collect::<Vec<_>>());
        moving
    }

    /// the interval that ends at report `end`
    pub fn gap(&self, end: usize) -> u32 {
        (self.reports[end].t_us - self.reports[end - 1].t_us) as u32
    }

    pub fn gaps(&self) -> impl Iterator<Item = u32> + '_ {
        self.ends.iter().map(|&end| self.gap(end))
    }

    /// the neighbours' counts per period for each moving interval, in the order of `ends`: the
    /// mean over the reports either side that end a moving interval themselves, each over its
    /// own interval, so a report's own counts never predict themselves. none when neither does
    pub fn paces(&self) -> Vec<Option<f64>> {
        let Some(period) = self.period_us else {
            return vec![None; self.ends.len()];
        };
        let mut is_end = vec![false; self.reports.len()];
        for &end in &self.ends {
            is_end[end] = true;
        }
        let pace = |i: usize| -> Option<f64> {
            is_end
                .get(i)
                .copied()
                .unwrap_or(false)
                .then(|| self.reports[i].counts() * period / self.gap(i) as f64)
        };
        self.ends
            .iter()
            .map(|&end| match (pace(end - 1), pace(end + 1)) {
                (Some(a), Some(b)) => Some((a + b) / 2.0),
                (one, other) => one.or(other),
            })
            .collect()
    }

    /// every gap of the view that is not a moving interval, so a view can say what it left out
    pub fn left_out(&self) -> usize {
        self.reports.len().saturating_sub(1) - self.ends.len()
    }
}

/// interval groups by period multiple: under 0.75 (bunched), about 1, 2 and 3, then 4 or more.
/// rounding would count a bunched report as 1x and hide it
pub fn group(gap_us: f64, period_us: f64) -> usize {
    match gap_us / period_us {
        r if r < 0.75 => 0,
        r if r < 1.5 => 1,
        r if r < 2.5 => 2,
        r if r < 3.5 => 3,
        _ => 4,
    }
}

/// the middle value, or the mean of the two middle ones. reorders `values`
pub fn median(values: &mut [f64]) -> Option<f64> {
    let n = values.len();
    if n == 0 {
        return None;
    }
    let (_, upper, _) = values.select_nth_unstable_by(n / 2, f64::total_cmp);
    let upper = *upper;
    if n % 2 == 1 {
        return Some(upper);
    }
    let lower = values[..n / 2]
        .iter()
        .copied()
        .max_by(f64::total_cmp)
        .unwrap_or(upper);
    Some((lower + upper) / 2.0)
}

/// nearest rank, so every value returned is one that happened. reorders `values`
pub fn nearest_rank(values: &mut [f64], p: f64) -> Option<f64> {
    let n = values.len();
    if n == 0 {
        return None;
    }
    let rank = ((p * n as f64).ceil() as usize).clamp(1, n);
    Some(*values.select_nth_unstable_by(rank - 1, f64::total_cmp).1)
}

#[cfg(test)]
mod tests;
