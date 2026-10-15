use serde::{Deserialize, Serialize};

pub const REVISION: &str = "linear-v2";

/// the aim settings, already within the limits the app saves them under
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub speed: f64,
    pub deadzone_counts: f64,
    pub hotkey_mask: u32,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            speed: 2000.0,
            deadzone_counts: 12.0,
            hotkey_mask: 8,
        }
    }
}

#[derive(Default)]
pub struct Linear {
    remainder: [f64; 2],
    settled: bool,
}
impl Linear {
    pub fn reset(&mut self) {
        self.remainder = [0.0; 2];
        self.settled = false;
    }
    pub fn settled(&self) -> bool {
        self.settled
    }
    pub fn hold(&mut self, target: [f64; 2], deadzone: f64) -> bool {
        let distance = target[0].hypot(target[1]);
        let stop = deadzone.max(0.5);
        let resume = (deadzone * 1.5).max(stop + 1.0);
        self.settled =
            !distance.is_finite() || distance <= stop || (self.settled && distance <= resume);
        if self.settled {
            self.remainder = [0.0; 2];
        }
        self.settled
    }
    pub fn step(&mut self, target: [f64; 2], config: &Config, seconds: f64) -> [i16; 2] {
        let distance = target[0].hypot(target[1]);
        if !distance.is_finite() || !seconds.is_finite() || seconds <= 0.0 {
            self.reset();
            return [0; 2];
        }
        if self.hold(target, config.deadzone_counts) {
            return [0; 2];
        }
        let length =
            (config.speed * seconds.min(0.05)).min((distance - config.deadzone_counts).max(0.0));
        let mut result = [0; 2];
        for axis in 0..2 {
            let value = target[axis] / distance * length + self.remainder[axis];
            result[axis] = value.trunc().clamp(i16::MIN as f64, i16::MAX as f64) as i16;
            self.remainder[axis] = value - result[axis] as f64;
        }
        result
    }
}

#[cfg(test)]
mod tests;
