//! texture19, ported from abcurves/texture.py and held to it by the parity test at the bottom.
//! every function here mirrors a numpy call, down to how it rounds and interpolates, because the
//! number on screen is only worth something if it is the reference's number

use crate::analysis::numpy::{linear_quantile, mean, sorted, std};
use std::f64::consts::PI;

pub const NAMES: [&str; 19] = [
    "zero_rate",
    "nonzero_mag_mean_log1p",
    "nonzero_mag_max_log1p",
    "sign_flip_rate",
    "zero_run_mean_log1p",
    "active_run_mean_log1p",
    "large_packet_rate",
    "bp_10_30",
    "bp_30_80",
    "bp_80_200",
    "bp_200_500",
    "hf_power_log1p",
    "nonzero_mag_p90_log1p",
    "nonzero_mag_p99_log1p",
    "iri_std_log1p",
    "iri_p95_log1p",
    "mag_ac_lag1",
    "mag_ac_lag5",
    "mag_entropy",
];

const BANDS: [(f64, f64); 4] = [(10.0, 30.0), (30.0, 80.0), (80.0, 200.0), (200.0, 500.0)];

pub fn is_idle(window: &[[i32; 2]]) -> bool {
    window.iter().all(|v| v[0] == 0 && v[1] == 0)
}

/// numpy sign: zero stays zero, which f64::signum does not do
fn sign(v: f64) -> f64 {
    if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        0.0
    }
}

/// lengths of the false runs and the true runs, in order of appearance
fn run_lengths(active: &[bool]) -> (Vec<f64>, Vec<f64>) {
    let (mut zero, mut on) = (Vec::new(), Vec::new());
    let Some(&first) = active.first() else {
        return (zero, on);
    };
    let (mut current, mut length) = (first, 1usize);
    for &value in &active[1..] {
        if value == current {
            length += 1;
        } else {
            if current { &mut on } else { &mut zero }.push(length as f64);
            current = value;
            length = 1;
        }
    }
    if current { &mut on } else { &mut zero }.push(length as f64);
    (zero, on)
}

fn autocorrelation(values: &[f64], lag: usize) -> f64 {
    if values.len() < lag + 4 {
        return 0.0;
    }
    let n = values.len();
    let (left, right) = (&values[..n - lag], &values[lag..]);
    let (ml, mr) = (mean(left), mean(right));
    let (mut ll, mut rr, mut lr) = (0.0, 0.0, 0.0);
    for (a, b) in left.iter().zip(right) {
        let (a, b) = (a - ml, b - mr);
        ll += a * a;
        rr += b * b;
        lr += a * b;
    }
    let denominator = (ll * rr).sqrt();
    if denominator >= 1e-9 {
        lr / denominator
    } else {
        0.0
    }
}

/// |rfft(x)|^2 on numpy's exact bin grid for any n. a direct dft, not a butterfly, so there is
/// no padding to a power of two shifting the frequencies the bands are cut at
fn rfft_power(x: &[f64]) -> Vec<f64> {
    let n = x.len();
    (0..=n / 2)
        .map(|k| {
            let (mut re, mut im) = (0.0, 0.0);
            for (t, v) in x.iter().enumerate() {
                let angle = -2.0 * PI * ((k * t) % n) as f64 / n as f64;
                re += v * angle.cos();
                im += v * angle.sin();
            }
            re * re + im * im
        })
        .collect()
}

pub fn features(window: &[[i32; 2]]) -> [f64; 19] {
    let mut row = [0.0f64; 19];
    let n = window.len();
    if n == 0 {
        return row;
    }

    let packet: Vec<f64> = window
        .iter()
        .map(|[x, y]| x.unsigned_abs().max(y.unsigned_abs()) as f64)
        .collect();
    let active: Vec<bool> = packet.iter().map(|m| *m > 0.0).collect();
    let active_mag: Vec<f64> = packet.iter().copied().filter(|m| *m > 0.0).collect();

    let flip = |axis: usize| {
        let signs: Vec<f64> = window
            .iter()
            .map(|v| sign(v[axis] as f64))
            .filter(|s| *s != 0.0)
            .collect();
        if signs.len() > 1 {
            let flips = signs.windows(2).filter(|w| w[1] * w[0] < 0.0).count();
            flips as f64 / (signs.len() - 1) as f64
        } else {
            0.0
        }
    };
    let (zero_runs, active_runs) = run_lengths(&active);

    row[0] = 1.0 - active.iter().filter(|a| **a).count() as f64 / n as f64;
    row[1] = if active_mag.is_empty() {
        0.0
    } else {
        mean(&active_mag).ln_1p()
    };
    row[2] = active_mag
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max)
        .ln_1p()
        .max(0.0);
    if active_mag.is_empty() {
        row[2] = 0.0;
    }
    row[3] = (flip(0) + flip(1)) / 2.0;
    row[4] = mean(&zero_runs).ln_1p();
    row[5] = mean(&active_runs).ln_1p();
    row[6] = packet.iter().filter(|m| **m >= 3.0).count() as f64 / n as f64;

    if n >= 8 {
        let magnitude: Vec<f64> = window
            .iter()
            .map(|[x, y]| ((*x as f64).powi(2) + (*y as f64).powi(2)).sqrt())
            .collect();
        let m = mean(&magnitude);
        let centered: Vec<f64> = magnitude.iter().map(|v| v - m).collect();
        let power = rfft_power(&centered);
        let frequency = |k: usize| k as f64 * (1.0 / (n as f64 * 0.001));
        let total = power[1..].iter().sum::<f64>().max(1e-9);
        let band = |low: f64, high: f64| {
            power
                .iter()
                .enumerate()
                .filter(|(k, _)| frequency(*k) >= low && frequency(*k) < high)
                .map(|(_, p)| p)
                .sum::<f64>()
        };
        for (i, (low, high)) in BANDS.iter().enumerate() {
            row[7 + i] = band(*low, *high) / total;
        }
        row[11] = (band(30.0, 500.0) / n as f64).ln_1p();

        let active_norm = sorted(
            &magnitude
                .iter()
                .copied()
                .filter(|v| *v > 0.0)
                .collect::<Vec<_>>(),
        );
        row[12] = if active_norm.is_empty() {
            0.0
        } else {
            linear_quantile(&active_norm, 0.90).ln_1p()
        };
        row[13] = if active_norm.is_empty() {
            0.0
        } else {
            linear_quantile(&active_norm, 0.99).ln_1p()
        };

        let on: Vec<bool> = magnitude.iter().map(|v| *v > 0.0).collect();
        let (intervals, _) = run_lengths(&on);
        row[14] = if intervals.len() >= 2 {
            std(&intervals).ln_1p()
        } else {
            0.0
        };
        row[15] = if intervals.is_empty() {
            0.0
        } else {
            linear_quantile(&sorted(&intervals), 0.95).ln_1p()
        };

        row[16] = autocorrelation(&magnitude, 1);
        row[17] = autocorrelation(&magnitude, 5);

        let mut histogram = [0.0f64; 11];
        for v in &magnitude {
            histogram[v.round_ties_even().clamp(0.0, 10.0) as usize] += 1.0;
        }
        let count = histogram.iter().sum::<f64>().max(1.0);
        row[18] = -histogram
            .iter()
            .filter(|h| **h > 0.0)
            .map(|h| {
                let p = h / count;
                p * p.ln()
            })
            .sum::<f64>()
            / (11.0f64).ln();
    }

    for v in &mut row {
        if !v.is_finite() {
            *v = 0.0;
        }
    }
    row
}

/// per-feature wasserstein-1 on a shared 199 quantile grid, standardised by the real side's
/// spread with the reference's 1e-6 floor. the same table abcurves prints as its diagnostic
pub fn w1<R: AsRef<[f64]>>(real: &[R], fake: &[R]) -> Vec<f64> {
    let Some(first) = real.first() else {
        return Vec::new();
    };
    if fake.is_empty() {
        return Vec::new();
    }
    (0..first.as_ref().len())
        .map(|j| {
            let column = |rows: &[R]| rows.iter().map(|r| r.as_ref()[j]).collect::<Vec<_>>();
            let (r, f) = (column(real), column(fake));
            let mut scale = std(&r);
            if scale < 1e-6 {
                scale = 1.0;
            }
            let r = sorted(&r.iter().map(|v| v / scale).collect::<Vec<_>>());
            let f = sorted(&f.iter().map(|v| v / scale).collect::<Vec<_>>());
            (1..=199)
                .map(|i| {
                    let q = i as f64 / 200.0;
                    (linear_quantile(&f, q) - linear_quantile(&r, q)).abs()
                })
                .sum::<f64>()
                / 199.0
        })
        .collect()
}

#[cfg(test)]
mod tests;
