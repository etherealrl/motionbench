//! the box's buttons for everything that listens to them: the recording button and the aim
//! hold. one thread reads the link's feed; it is the only owner of the aim hold binding, so
//! the binding, its learning and the held buttons never have two writers

#[cfg(feature = "developer-tools")]
use crate::controller::experiment;
use crate::recording::hotkey;
use motion_bench::hardware::medius::Feed;
use motion_bench::sync::LockExt;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    mpsc::{Receiver, RecvTimeoutError, SyncSender},
};
use std::time::Duration;

/// the aim hold binding and whether the aim controller is using it
#[derive(Clone, Default)]
pub struct Aim {
    pub mask: u32,
    pub learning: bool,
    pub armed: bool,
    pub seq: u64,
    pub error: Option<String>,
}

pub struct Buttons {
    held: AtomicU32,
    /// how many reports of the held buttons came in, so a press can be told from a guess
    reports: AtomicU64,
    /// when a button last went down on the box, on the wall clock in microseconds, 0 before one
    pressed_us: AtomicU64,
    ready: AtomicBool,
    /// why the feed went, until it comes back
    lost: Mutex<Option<String>>,
    aim: Mutex<Aim>,
    /// the aim loop's wakeup, so a released button stops it at once
    wake: Mutex<Option<SyncSender<()>>>,
}

impl Buttons {
    pub fn new(aim_mask: u32) -> Arc<Self> {
        Arc::new(Self {
            held: AtomicU32::new(0),
            reports: AtomicU64::new(0),
            pressed_us: AtomicU64::new(0),
            ready: AtomicBool::new(false),
            lost: Mutex::new(None),
            aim: Mutex::new(Aim {
                mask: aim_mask,
                ..Aim::default()
            }),
            wake: Mutex::new(None),
        })
    }

    /// when a button last went down on the box, on the wall clock
    pub fn pressed_us(&self) -> Option<u64> {
        Some(self.pressed_us.load(Ordering::Acquire)).filter(|&us| us != 0)
    }

    /// the buttons held now; none while the feed is not up, since a lost feed's are unknown
    #[cfg(any(test, feature = "developer-tools"))]
    pub fn held(&self) -> u32 {
        self.held.load(Ordering::Acquire)
    }

    /// reports of the held buttons so far; the held set after a comeback is a guess until the
    /// box reports one
    #[cfg(feature = "developer-tools")]
    pub fn reports(&self) -> u64 {
        self.reports.load(Ordering::Acquire)
    }

    pub fn ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    pub fn aim(&self) -> Aim {
        self.aim.locked().clone()
    }

    /// whatever needs the buttons asks this first, so it fails with the reason they are gone
    pub fn require(&self) -> Result<(), String> {
        if self.ready() {
            return Ok(());
        }
        Err(self
            .lost
            .locked()
            .clone()
            .unwrap_or_else(|| "connect a box to use its buttons".into()))
    }

    #[cfg(feature = "developer-tools")]
    pub fn learn_aim(&self, cancel: bool) -> Result<(), String> {
        let mut aim = self.aim.locked();
        if !cancel && aim.armed {
            return Err("disarm before changing the hotkey".into());
        }
        aim.learning = !cancel;
        aim.error = None;
        drop(aim);
        self.wake();
        Ok(())
    }

    #[cfg(feature = "developer-tools")]
    pub fn arm(&self, armed: bool) {
        let mut aim = self.aim.locked();
        aim.armed = armed;
        if armed {
            aim.learning = false;
            aim.error = None;
        }
    }

    #[cfg(feature = "developer-tools")]
    pub fn wake_with(&self, wakeup: Option<SyncSender<()>>) {
        *self.wake.locked() = wakeup;
    }

    fn wake(&self) {
        if let Some(wakeup) = &*self.wake.locked() {
            let _ = wakeup.try_send(());
        }
    }

    /// the aim controller's status as the buttons have it; a button that could not be saved
    /// or a lost feed is the buttons' trouble, not a failed controller
    #[cfg(feature = "developer-tools")]
    pub fn status(&self, status: &mut experiment::Status) {
        let aim = self.aim();
        status.ready = self.ready();
        status.learning = aim.learning;
        status.hotkey_mask = aim.mask;
        status.held_mask = self.held();
        status.binding_seq = aim.seq;
        status.buttons_error = aim.error.or_else(|| self.lost.locked().clone());
    }

    fn feed(&self, lost: Option<String>) {
        let ready = lost.is_none();
        *self.lost.locked() = lost;
        self.held.store(0, Ordering::Release);
        self.ready.store(ready, Ordering::Release);
        if !ready {
            self.aim.locked().learning = false;
        }
        self.wake();
    }

    /// the aim button learned from `pressed`, saved before it is used; the lock is let go
    /// while the file is written
    fn learn_from(&self, pressed: u32, recording: &hotkey::Controller) -> bool {
        let learning = self.aim.locked().learning;
        if !learning
            || pressed.count_ones() != 1
            || recording.snapshot().hotkey_mask == Some(pressed)
        {
            return learning;
        }
        let saved = crate::config::Button::try_from(pressed)
            .and_then(|button| recording.settings().update(|s| s.aim.button = button));
        let mut aim = self.aim.locked();
        match saved {
            Ok(_) => {
                aim.mask = pressed;
                aim.seq += 1;
                aim.learning = false;
                aim.error = None;
            }
            Err(error) => aim.error = Some(format!("could not keep the aim button: {error}")),
        }
        true
    }

    /// reads the link's feed until it ends or `shutdown` is set
    pub fn route(
        self: Arc<Self>,
        feed: Receiver<Feed>,
        recording: Arc<hotkey::Controller>,
        shutdown: Arc<AtomicBool>,
    ) {
        // a route that ends or panics leaves no button held, or an armed aim and the recording
        // button would keep acting on the last state
        let routed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.clone().routing(&feed, &recording, &shutdown)
        }));
        let why = match routed {
            Ok(()) => "the box's button feed ended",
            Err(_) => "reading the box's buttons failed unexpectedly",
        };
        if !shutdown.load(Ordering::Relaxed) {
            recording.unavailable(Some(why.into()));
            self.feed(Some(why.into()));
        }
    }

    fn routing(
        self: Arc<Self>,
        feed: &Receiver<Feed>,
        recording: &hotkey::Controller,
        shutdown: &AtomicBool,
    ) {
        while !shutdown.load(Ordering::Relaxed) {
            match feed.recv_timeout(Duration::from_millis(100)) {
                // the recording binding first, so whoever sees the buttons change sees it too
                Ok(Feed::Ready) => {
                    recording.ready();
                    self.feed(None);
                }
                Ok(Feed::Lost(error)) => {
                    recording.unavailable(Some(error.clone()));
                    self.feed(Some(error));
                }
                Ok(Feed::Buttons { mask, at }) => {
                    let pressed = mask & !self.held.swap(mask, Ordering::AcqRel);
                    self.reports.fetch_add(1, Ordering::AcqRel);
                    if pressed != 0 {
                        self.pressed_us.store(at.utc_us, Ordering::Release);
                    }
                    let aim_learning = self.learn_from(pressed, recording);
                    let aim = self.aim();
                    recording.buttons(
                        mask,
                        aim_learning,
                        if aim.armed || aim.learning {
                            aim.mask
                        } else {
                            0
                        },
                        hotkey::outside_app(),
                        at,
                    );
                    self.wake();
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

#[cfg(test)]
mod tests;
