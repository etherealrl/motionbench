//! locks that keep working after a panic in whoever held them last

use std::sync::{Mutex, MutexGuard};

pub trait LockExt<T> {
    /// the guard, even when a panic poisoned the lock. the app finds out it is broken from
    /// `is_poisoned` and goes on only far enough to say so, disarm and close
    fn locked(&self) -> MutexGuard<'_, T>;
}

impl<T> LockExt<T> for Mutex<T> {
    #[allow(clippy::disallowed_methods)]
    fn locked(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
