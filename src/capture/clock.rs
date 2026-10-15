use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn QueryPerformanceCounter(value: *mut i64) -> i32;
    fn QueryPerformanceFrequency(value: *mut i64) -> i32;
    fn GetSystemTimePreciseAsFileTime(value: *mut u64);
}

pub fn qpc() -> i64 {
    #[cfg(windows)]
    {
        let mut value = 0;
        unsafe { QueryPerformanceCounter(&mut value) };
        value
    }
    #[cfg(not(windows))]
    {
        static START: OnceLock<std::time::Instant> = OnceLock::new();
        START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_nanos() as i64
    }
}
pub fn frequency() -> i64 {
    static FREQUENCY: OnceLock<i64> = OnceLock::new();
    *FREQUENCY.get_or_init(|| {
        #[cfg(windows)]
        {
            let mut value = 0;
            unsafe { QueryPerformanceFrequency(&mut value) };
            value
        }
        #[cfg(not(windows))]
        {
            1_000_000_000
        }
    })
}
pub fn utc_us() -> u64 {
    #[cfg(windows)]
    {
        let mut value = 0;
        unsafe { GetSystemTimePreciseAsFileTime(&mut value) };
        value.saturating_sub(116_444_736_000_000_000) / 10
    }
    #[cfg(not(windows))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64
    }
}
pub fn elapsed_us(start: i64, end: i64) -> Option<u64> {
    let ticks = end.checked_sub(start)?;
    (ticks >= 0 && frequency() > 0)
        .then(|| (ticks as i128 * 1_000_000 / frequency() as i128) as u64)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Anchor {
    pub qpc_before: i64,
    pub qpc_after: i64,
    pub utc_us: u64,
}
impl Anchor {
    pub fn take() -> Self {
        let qpc_before = qpc();
        let utc_us = utc_us();
        Self {
            qpc_before,
            qpc_after: qpc(),
            utc_us,
        }
    }
    pub fn midpoint(&self) -> i64 {
        ((self.qpc_before as i128 + self.qpc_after as i128) / 2) as i64
    }
    pub fn uncertainty_us(&self, frequency: i64) -> f64 {
        if frequency <= 0 || self.qpc_after < self.qpc_before {
            return f64::INFINITY;
        }
        (self.qpc_after as i128 - self.qpc_before as i128) as f64 * 500_000.0 / frequency as f64
            + 1.0
    }
    /// `ticks` from before the anchor as utc, never earlier than it was: mapped from the qpc read
    /// before the utc one, so a thread held up inside the anchor only makes it late
    pub fn utc_never_early(&self, ticks: i64, frequency: i64) -> Option<u64> {
        if frequency <= 0 {
            return None;
        }
        u64::try_from(
            self.utc_us as i128
                + (ticks as i128 - self.qpc_before as i128) * 1_000_000 / frequency as i128,
        )
        .ok()
    }
    pub fn epoch_us(&self, ticks: i64, frequency: i64) -> Option<u64> {
        if frequency <= 0 || self.qpc_after < self.qpc_before {
            return None;
        }
        u64::try_from(
            self.utc_us as i128
                + (ticks as i128 - self.midpoint() as i128) * 1_000_000 / frequency as i128,
        )
        .ok()
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Summary {
    pub samples: u64,
    pub max_bracket_us: f64,
    pub max_mapping_change_us: f64,
    pub discontinuities: u64,
}
#[derive(Default)]
pub struct Monitor {
    pub summary: Summary,
    previous: Option<Anchor>,
}
impl Monitor {
    pub fn observe(&mut self, anchor: &Anchor, frequency: i64) -> Result<f64, String> {
        if frequency <= 0 || anchor.qpc_after < anchor.qpc_before {
            return Err("invalid QPC clock anchor".into());
        }
        let bracket = anchor.uncertainty_us(frequency);
        self.summary.samples += 1;
        self.summary.max_bracket_us = self.summary.max_bracket_us.max(bracket);
        let change = if let Some(previous) = &self.previous {
            let elapsed = (anchor.midpoint() as i128 - previous.midpoint() as i128) as f64
                * 1_000_000.0
                / frequency as f64;
            if elapsed <= 0.0 {
                return Err("QPC clock regressed".into());
            }
            (anchor.utc_us as i128 - previous.utc_us as i128) as f64 - elapsed
        } else {
            0.0
        };
        self.summary.max_mapping_change_us = self.summary.max_mapping_change_us.max(change.abs());
        let uncertain = self
            .previous
            .as_ref()
            .map_or(0.0, |p| p.uncertainty_us(frequency))
            + bracket;
        self.previous = Some(anchor.clone());
        if change.abs() > 1000.0 + uncertain {
            self.summary.discontinuities += 1;
            return Err(format!(
                "UTC/QPC mapping changed by {change:.0} µs; interval and alignment precision is compromised"
            ));
        }
        Ok(change)
    }
}

#[cfg(test)]
mod tests;
