use crate::{
    app::state::{Shared, allowed, off_main},
    config::{Button, Store},
    recording::{self, Recording},
};
use motion_bench::capture::clock::Anchor;
use motion_bench::sync::LockExt;
use serde::Serialize;
use serde_json::Value;
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, SyncSender},
    },
};
use tauri::State;

#[derive(Clone, Default, Serialize)]
pub struct Status {
    pub hotkey_mask: Option<u32>,
    pub ready: bool,
    pub learning: bool,
    pub held_mask: u32,
    pub binding_seq: u64,
    pub busy: bool,
    pub message: Option<String>,
    pub error: Option<String>,
}

pub struct Controller {
    state: Mutex<Status>,
    recording: Arc<Recording>,
    requests: SyncSender<Anchor>,
    settings: Arc<Store>,
}
impl Controller {
    /// the saved binding, never pressed: restoring it records nothing
    pub fn new(settings: Arc<Store>) -> (Self, Receiver<Anchor>) {
        let state = Status {
            hotkey_mask: settings.get().recording.button.map(Button::mask),
            ..Status::default()
        };
        let (requests, receiver) = mpsc::sync_channel(1);
        (
            Self {
                state: Mutex::new(state),
                recording: Arc::default(),
                requests,
                settings,
            },
            receiver,
        )
    }
    pub fn settings(&self) -> &Arc<Store> {
        &self.settings
    }
    pub fn recording(&self) -> &Arc<Recording> {
        &self.recording
    }
    /// a controller whose settings live in `folder`'s config.json
    #[cfg(test)]
    pub(crate) fn for_test(folder: &Path) -> (Arc<Self>, Receiver<Anchor>) {
        let store = Arc::new(Store::open(folder.join("config.json")));
        let (controller, requests) = Self::new(store);
        (Arc::new(controller), requests)
    }
    pub fn snapshot(&self) -> Status {
        let mut state = self.state.locked().clone();
        state.busy = self.recording.busy();
        state
    }
    pub fn update(&self, action: impl FnOnce(&mut Status)) {
        action(&mut self.state.locked());
    }
    pub fn ready(&self) {
        self.update(|s| {
            s.ready = true;
            s.error = None;
        });
    }
    pub fn unavailable(&self, error: Option<String>) {
        self.update(|s| {
            s.ready = false;
            s.learning = false;
            s.held_mask = 0;
            if error.is_some() {
                s.error = error;
            }
        });
    }
    pub fn learn(&self, cancel: bool) {
        self.update(|s| {
            s.learning = !cancel;
            if !cancel {
                s.error = None;
            }
        });
    }
    pub fn disable(&self) -> Result<(), String> {
        self.settings.update(|s| s.recording.button = None)?;
        self.update(|s| {
            s.hotkey_mask = None;
            s.learning = false;
            s.error = None;
        });
        Ok(())
    }
    pub fn buttons(
        &self,
        mask: u32,
        aim_learning: bool,
        aim_mask: u32,
        outside_app: bool,
        requested: Anchor,
    ) {
        let mut state = self.state.locked();
        let pressed = mask & !state.held_mask;
        state.held_mask = mask;
        if !state.ready || aim_learning {
            return;
        }
        if state.learning {
            if pressed.count_ones() != 1 {
                return;
            }
            if pressed == aim_mask {
                state.error = Some(
                    "that button is the aim hold binding, press a different mouse button".into(),
                );
                return;
            }
            match Button::try_from(pressed)
                .and_then(|button| self.settings.update(|s| s.recording.button = Some(button)))
            {
                Ok(_) => {
                    state.hotkey_mask = Some(pressed);
                    state.binding_seq += 1;
                    state.learning = false;
                    state.error = None;
                }
                Err(error) => {
                    state.error = Some(format!("could not keep the recording button: {error}"))
                }
            }
            return;
        }
        // a left or right click in the app's own window is the app being used, not a recording
        let clicks_the_app = |binding: u32| !outside_app && binding & 0b11 != 0;
        if state.hotkey_mask.is_none_or(|binding| {
            pressed & binding == 0 || binding == aim_mask || clicks_the_app(binding)
        }) {
            return;
        }
        if !self.recording.press() {
            state.error = Some("recording is still finalizing, wait before pressing again".into());
            return;
        }
        if self.requests.try_send(requested).is_err() {
            self.recording.unpress();
            state.error = Some("recording button worker is unavailable".into());
        }
    }
}

pub fn outside_app() -> bool {
    #[cfg(all(windows, not(test)))]
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId,
        };
        let window = GetForegroundWindow();
        let mut process = 0;
        GetWindowThreadProcessId(window, Some(&mut process));
        process != 0 && process != std::process::id()
    }
    #[cfg(any(not(windows), test))]
    {
        cfg!(test)
    }
}

/// a high tone when a recording starts, a lower one when it stops, two low ones when it failed,
/// at `volume` from 0 to 1. played off the caller's thread, since a cue lasts a couple hundred ms
pub fn cue(started: Option<bool>, volume: f64) {
    if volume <= 0.0 {
        return;
    }
    let tones: &[u32] = match started {
        Some(true) => &[1000],
        Some(false) => &[600],
        None => &[250, 0, 250],
    };
    let wave = tone_wave(tones, volume);
    #[cfg(all(windows, not(test)))]
    std::thread::spawn(move || unsafe {
        use windows::Win32::Media::Audio::{PlaySoundW, SND_MEMORY, SND_SYNC};
        // the wave stays alive until it has played, which an async play couldn't promise
        let _ = PlaySoundW(
            windows::core::PCWSTR(wave.as_ptr().cast()),
            None,
            SND_MEMORY | SND_SYNC,
        );
    });
    #[cfg(any(not(windows), test))]
    let _ = wave;
}

const CUE_RATE: u32 = 44_100;

/// a mono 16 bit wav of 100 ms per tone, 0 being a pause. each tone fades in and out over a few
/// ms so it doesn't click, and the level goes by the square of `volume` since loudness is heard
/// that way
pub(crate) fn tone_wave(tones: &[u32], volume: f64) -> Vec<u8> {
    let per_tone = (CUE_RATE / 10) as usize;
    let fade = (CUE_RATE / 200) as usize;
    let peak = volume.clamp(0.0, 1.0).powi(2) * 0.8 * f64::from(i16::MAX);
    let mut samples = Vec::with_capacity(per_tone * tones.len());
    for &hz in tones {
        for n in 0..per_tone {
            let edge = n.min(per_tone - 1 - n).min(fade) as f64 / fade as f64;
            let phase = std::f64::consts::TAU * f64::from(hz) * n as f64 / f64::from(CUE_RATE);
            samples.push((phase.sin() * peak * edge) as i16);
        }
    }
    let data = (samples.len() * 2) as u32;
    let mut wave = Vec::with_capacity(44 + data as usize);
    wave.extend_from_slice(b"RIFF");
    wave.extend_from_slice(&(36 + data).to_le_bytes());
    wave.extend_from_slice(b"WAVEfmt ");
    wave.extend_from_slice(&16u32.to_le_bytes());
    wave.extend_from_slice(&1u16.to_le_bytes());
    wave.extend_from_slice(&1u16.to_le_bytes());
    wave.extend_from_slice(&CUE_RATE.to_le_bytes());
    wave.extend_from_slice(&(CUE_RATE * 2).to_le_bytes());
    wave.extend_from_slice(&2u16.to_le_bytes());
    wave.extend_from_slice(&16u16.to_le_bytes());
    wave.extend_from_slice(b"data");
    wave.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        wave.extend_from_slice(&sample.to_le_bytes());
    }
    wave
}

/// the control journal's own end: when the stop completed, and whether the journal saw the
/// recording through or ended on `error`. it is saved before the recording's metadata, which
/// keeps a copy of it, so the journal says how control went and the metadata how the
/// recording did
pub(crate) fn finish_control(control: &mut Value, error: Option<&str>) -> Result<(), String> {
    control["stop_completed"] =
        serde_json::to_value(motion_bench::capture::clock::Anchor::take()).unwrap();
    control["complete"] = error.is_none().into();
    control["error"] = error.into();
    let path = Path::new(
        control["path"]
            .as_str()
            .ok_or("control evidence has no capture path")?,
    );
    let journal = path.with_extension("recording-control.json");
    let temporary = journal.with_extension("json.tmp");
    let saved = std::fs::write(&temporary, serde_json::to_vec_pretty(&*control).unwrap())
        .and_then(|()| std::fs::rename(&temporary, journal));
    if saved.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    saved.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn recording_toggle(recording: bool, state: State<'_, Shared>) -> Result<String, String> {
    let requested = motion_bench::capture::clock::Anchor::take();
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        recording::toggle(&shared, requested, Some(recording))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn recording_hotkey_learn(cancel: bool, state: State<Shared>) -> Result<Status, String> {
    learn_button(&state, cancel)
}
pub(crate) fn learn_button(state: &Shared, cancel: bool) -> Result<Status, String> {
    let _claim = Recording::claim(state)?;
    if cancel {
        state.recording_hotkey.learn(true);
        return Ok(state.recording_hotkey.snapshot());
    }
    allowed(state.bench.locked().readiness(false).recording_button)?;
    state.recording_hotkey.learn(false);
    Ok(state.recording_hotkey.snapshot())
}
#[tauri::command]
pub async fn recording_hotkey_disable(state: State<'_, Shared>) -> Result<(), String> {
    off_main(state, clear_button).await
}
pub(crate) fn clear_button(state: &Shared) -> Result<(), String> {
    let _claim = Recording::claim(state)?;
    allowed(state.bench.locked().readiness(false).clear_recording_button)?;
    state.recording_hotkey.disable()
}

#[cfg(test)]
mod tests;
