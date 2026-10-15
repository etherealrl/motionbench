//! the 1 ms grid every analysis bins movement on. a report lands in the tick its time floors to,
//! counted from an origin the caller picks, and how many ticks a caller keeps is up to it

/// one tick, the grid the judges were trained on
pub const TICK_US: u64 = 1000;

/// the tick `t_us` lands in, none before the origin
pub fn tick(origin: u64, t_us: u64) -> Option<usize> {
    t_us.checked_sub(origin)
        .map(|since| (since / TICK_US) as usize)
}
