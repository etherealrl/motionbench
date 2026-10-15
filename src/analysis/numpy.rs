//! just enough of numpy.random to reproduce `default_rng(seed).permutation(n)` bit for bit:
//! SeedSequence's pool hash, the PCG64 XSL-RR generator, and the masked rejection draw shuffle
//! uses. every constant is numpy's
//!
//! and numpy's mean, population std and default linear quantile, which the features, the folds
//! and the judge all reduce with

const INIT_A: u32 = 0x43b0_d7e5;
const MULT_A: u32 = 0x931e_8875;
const INIT_B: u32 = 0x8b51_f9dd;
const MULT_B: u32 = 0x58f3_8ded;
const MIX_MULT_L: u32 = 0xca01_f9dd;
const MIX_MULT_R: u32 = 0x4973_f715;
const XSHIFT: u32 = 16;
const POOL: usize = 4;
const PCG_MULT: u128 = 0x2360_ED05_1FC6_5DA4_4385_DF64_9FCC_F645;

struct Hash(u32);

impl Hash {
    fn mix(&mut self, mut value: u32) -> u32 {
        value ^= self.0;
        self.0 = self.0.wrapping_mul(MULT_A);
        value = value.wrapping_mul(self.0);
        value ^ (value >> XSHIFT)
    }
}

fn mix(x: u32, y: u32) -> u32 {
    let result = MIX_MULT_L
        .wrapping_mul(x)
        .wrapping_sub(MIX_MULT_R.wrapping_mul(y));
    result ^ (result >> XSHIFT)
}

pub struct SeedSequence {
    pool: [u32; POOL],
}

impl SeedSequence {
    /// entropy words as numpy coerces a list of small ints, one u32 each
    pub fn new(entropy: &[u32]) -> Self {
        let mut hash = Hash(INIT_A);
        let mut pool = [0u32; POOL];
        for (i, slot) in pool.iter_mut().enumerate() {
            *slot = hash.mix(entropy.get(i).copied().unwrap_or(0));
        }
        for src in 0..POOL {
            for dst in 0..POOL {
                if src != dst {
                    pool[dst] = mix(pool[dst], hash.mix(pool[src]));
                }
            }
        }
        for &word in entropy.iter().skip(POOL) {
            for slot in pool.iter_mut() {
                *slot = mix(*slot, hash.mix(word));
            }
        }
        SeedSequence { pool }
    }

    pub fn generate_state(&self, words: usize) -> Vec<u32> {
        let mut hash = INIT_B;
        (0..words)
            .map(|i| {
                let mut value = self.pool[i % POOL];
                value ^= hash;
                hash = hash.wrapping_mul(MULT_B);
                value = value.wrapping_mul(hash);
                value ^ (value >> XSHIFT)
            })
            .collect()
    }

    /// numpy views pairs of u32 as u64 little endian, low word first
    pub fn generate_state_u64(&self, words: usize) -> Vec<u64> {
        self.generate_state(words * 2)
            .chunks(2)
            .map(|pair| pair[0] as u64 | ((pair[1] as u64) << 32))
            .collect()
    }
}

/// PCG64 as numpy's default bit generator: 128 bit lcg, xsl-rr output, and the 32 bit
/// half buffer next_uint32 draws from
pub struct Pcg64 {
    state: u128,
    inc: u128,
    buffered: Option<u32>,
}

impl Pcg64 {
    pub fn from_seed(seed: u32) -> Self {
        let words = SeedSequence::new(&[seed]).generate_state_u64(4);
        let initstate = ((words[0] as u128) << 64) | words[1] as u128;
        let initseq = ((words[2] as u128) << 64) | words[3] as u128;
        let mut rng = Pcg64 {
            state: 0,
            inc: (initseq << 1) | 1,
            buffered: None,
        };
        rng.step();
        rng.state = rng.state.wrapping_add(initstate);
        rng.step();
        rng
    }

    fn step(&mut self) {
        self.state = self.state.wrapping_mul(PCG_MULT).wrapping_add(self.inc);
    }

    pub fn next_u64(&mut self) -> u64 {
        self.step();
        let value = ((self.state >> 64) as u64) ^ (self.state as u64);
        value.rotate_right((self.state >> 122) as u32)
    }

    pub fn next_u32(&mut self) -> u32 {
        if let Some(value) = self.buffered.take() {
            return value;
        }
        let next = self.next_u64();
        self.buffered = Some((next >> 32) as u32);
        next as u32
    }

    pub fn integer(&mut self, upper: usize) -> usize {
        if upper <= 1 {
            return 0;
        }
        let range = upper as u32;
        let threshold = range.wrapping_neg() % range;
        loop {
            let product = self.next_u32() as u64 * range as u64;
            if product as u32 >= threshold {
                return (product >> 32) as usize;
            }
        }
    }

    /// numpy's random_interval: a value in 0..=max by masked rejection off next_uint32
    pub fn interval(&mut self, max: u64) -> u64 {
        if max == 0 {
            return 0;
        }
        let mut mask = max;
        for shift in [1, 2, 4, 8, 16, 32] {
            mask |= mask >> shift;
        }
        loop {
            let value = if max <= 0xffff_ffff {
                self.next_u32() as u64 & mask
            } else {
                self.next_u64() & mask
            };
            if value <= max {
                return value;
            }
        }
    }

    /// Generator.permutation(n): arange(n) shuffled from the top down
    pub fn permutation(&mut self, n: usize) -> Vec<usize> {
        let mut out: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            let j = self.interval(i as u64) as usize;
            out.swap(i, j);
        }
        out
    }
}

pub fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

/// numpy std, population, ddof 0
pub fn std(v: &[f64]) -> f64 {
    let m = mean(v);
    mean(&v.iter().map(|x| (x - m) * (x - m)).collect::<Vec<_>>()).sqrt()
}

/// numpy quantile with its default linear interpolation, q in 0..=1
pub fn linear_quantile(sorted: &[f64], q: f64) -> f64 {
    let n = sorted.len();
    if n == 0 {
        return 0.0;
    }
    let pos = q * (n - 1) as f64;
    let lo = pos.floor() as usize;
    let frac = pos - lo as f64;
    if lo + 1 >= n {
        sorted[n - 1]
    } else {
        sorted[lo] + frac * (sorted[lo + 1] - sorted[lo])
    }
}

pub fn sorted(v: &[f64]) -> Vec<f64> {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    s
}

#[cfg(test)]
mod tests;
