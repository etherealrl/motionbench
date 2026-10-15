use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Clone, Serialize, Deserialize)]
pub struct Estimate {
    pub method: String,
    pub box_origin_us: u64,
    pub host_origin_qpc: i64,
    pub qpc_frequency: i64,
    pub observed_rate_ppm: f64,
    pub residual_p05_us: f64,
    pub residual_p95_us: f64,
    pub span_us: u64,
    pub envelopes: usize,
    pub reports: u64,
    pub absolute_delay_known: bool,
    pub applied_to_scores: bool,
}
pub struct Arrivals {
    frequency: i64,
    origin: Option<(u64, i64)>,
    envelopes: VecDeque<(u64, f64)>,
    reports: u64,
    previous: Option<(u64, i64)>,
}
impl Default for Arrivals {
    fn default() -> Self {
        Self {
            frequency: crate::capture::clock::frequency(),
            origin: None,
            envelopes: VecDeque::new(),
            reports: 0,
            previous: None,
        }
    }
}
impl Arrivals {
    pub fn push(&mut self, box_us: u64, host_qpc: i64) -> Result<(), String> {
        if self
            .previous
            .is_some_and(|(b, q)| box_us < b || host_qpc < q)
        {
            return Err("Medius or serial receipt clock regressed".into());
        }
        self.previous = Some((box_us, host_qpc));
        let (box_origin, host_origin) = *self.origin.get_or_insert((box_us, host_qpc));
        let delta = box_us
            .checked_sub(box_origin)
            .ok_or("Medius clock regressed")?;
        let ticks = host_qpc
            .checked_sub(host_origin)
            .ok_or("QPC range overflow")?;
        if ticks < 0 {
            return Err("serial receipt QPC regressed".into());
        }
        let offset = ticks as f64 * 1_000_000.0 / self.frequency as f64 - delta as f64;
        self.reports += 1;
        let bucket = delta / 1_000_000;
        match self.envelopes.back_mut() {
            Some((time, best)) if *time / 1_000_000 == bucket => {
                if offset < *best {
                    *time = delta;
                    *best = offset;
                }
            }
            Some((time, _)) if delta < *time => {
                return Err("Medius envelope clock regressed".into());
            }
            _ => {
                self.envelopes.push_back((delta, offset));
                if self.envelopes.len() > 256 {
                    self.envelopes.pop_front();
                }
            }
        }
        Ok(())
    }
    pub fn estimate(&self) -> Option<Estimate> {
        let (box_origin_us, host_origin_qpc) = self.origin?;
        if self.envelopes.len() < 5 {
            return None;
        }
        let points: Vec<_> = self.envelopes.iter().copied().collect();
        let span_us = points.last()?.0 - points.first()?.0;
        if span_us < 5_000_000 {
            return None;
        }
        let mut slopes = Vec::new();
        for (i, &(time, offset)) in points.iter().enumerate() {
            for &(next, other) in &points[i + 1..] {
                if next - time >= 2_000_000 {
                    slopes.push((other - offset) / (next - time) as f64);
                }
            }
        }
        slopes.sort_by(f64::total_cmp);
        let slope = *slopes.get(slopes.len() / 2)?;
        let mut offsets: Vec<_> = points.iter().map(|&(t, y)| y - slope * t as f64).collect();
        offsets.sort_by(f64::total_cmp);
        let intercept = offsets[offsets.len() / 2];
        Some(Estimate {
            method: "one-second minimum serial receipt envelopes; median pairwise slope; diagnostic only".into(),
            box_origin_us, host_origin_qpc, qpc_frequency: self.frequency,
            observed_rate_ppm: slope * 1_000_000.0,
            residual_p05_us: offsets[(offsets.len() - 1) / 20] - intercept,
            residual_p95_us: offsets[(offsets.len() - 1) * 19 / 20] - intercept,
            span_us, envelopes: points.len(), reports: self.reports,
            absolute_delay_known: false, applied_to_scores: false,
        })
    }
}
#[cfg(test)]
mod tests;
