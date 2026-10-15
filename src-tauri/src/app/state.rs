//! the state every command and the pump share: the bench behind its lock, what the page is
//! sent, and the aim controller's handle
use crate::{
    app::pump::{box_label, clone_product, cloned_by, key_of, windows_name},
    config, library, recording, storage,
};
use motion_bench::sync::LockExt;

use crate::capture::observers;
#[cfg(feature = "developer-tools")]
use crate::controller::experiment;
use crate::hardware::buttons;
use crate::targets::vision;

use crate::library::{Library, Run};
use crate::recording::Recording;
use motion_bench::{
    analysis::{
        compare::Selection,
        views::{
            live::Live,
            plot::{Plot, Scale, Views, detach, plot},
        },
    },
    capture::session::{self, Capture, Device, Health},
    coordination::readiness::{self, Blocker, Facts, Readiness},
    hardware::medius,
    signal::stats::Key,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tauri::{State, ipc::Channel};

pub(crate) struct Bench {
    pub(crate) recording_hotkey: Arc<recording::hotkey::Controller>,
    pub(crate) observers: Option<observers::Worker>,
    pub(crate) last_observers: observers::Status,
    #[cfg(feature = "developer-tools")]
    pub(crate) experiment: Option<Arc<experiment::Worker>>,
    /// the box's buttons, routed to the recording button and the aim hold
    pub(crate) buttons: Arc<buttons::Buttons>,
    pub(crate) vision: Option<vision::Worker>,
    pub(crate) library_loaded: bool,
    pub(crate) startup_attempted: bool,
    pub(crate) requested: Value,
    pub(crate) last_health: Health,
    pub(crate) capture: Option<Capture>,
    pub(crate) link: medius::Link,
    pub(crate) library: Arc<Library>,
    pub(crate) jobs: library::jobs::Jobs,
    pub(crate) channel: Option<Channel<Snapshot>>,
    pub(crate) range: Option<Selection>,
    /// the tab the live view shows, its data as last computed and the axes it holds
    pub(crate) live: Live,
    pub(crate) seen_serial: u64,
    pub(crate) seen_discontinuities: u64,
    /// what Windows calls the captured endpoints, by (vendor, product, interface), and the set it
    /// was read for: the pump reads them again only when the devices change, outside this lock
    pub(crate) device_names: HashMap<(u16, u16, u8), String>,
    pub(crate) names_for: Vec<(u16, u16, u8)>,
    /// the endpoint a press on the box showed up on, and the box link's generation it holds for
    pub(crate) box_endpoint: Option<(Key, u64)>,
    /// the app is closing; the same flag `Shared` hands to its threads
    pub(crate) closing: Arc<AtomicBool>,
}
#[derive(Clone, Serialize)]
pub(crate) struct Snapshot {
    pub(crate) dev_mode: bool,
    pub(crate) recording_hotkey: recording::hotkey::Status,
    pub(crate) observers: observers::Status,
    /// whether this build has the developer tools, the aim controller among them
    pub(crate) developer_tools: bool,
    /// why the last settings change was not written, until one is
    pub(crate) settings_unsaved: Option<String>,
    #[cfg(feature = "developer-tools")]
    pub(crate) experiment: experiment::Status,
    pub(crate) vision: vision::Status,
    pub(crate) seq: u64,
    pub(crate) live: bool,
    pub(crate) devices: Vec<Device>,
    pub(crate) selected: Option<String>,
    pub(crate) health: Health,
    pub(crate) recording: Option<String>,
    pub(crate) recording_elapsed_ms: Option<u64>,
    pub(crate) plot: Plot,
    /// the live view is paused on this range, none while it follows the clock
    pub(crate) range: Option<Selection>,
    #[serde(rename = "box")]
    pub(crate) box_: Value,
    /// the library's revision, the runs themselves are read with `runs` when it moves, since
    /// a frame carrying them all costs milliseconds and megabytes once the library grows
    pub(crate) library: u64,
    pub(crate) job: Option<library::jobs::Job>,
    /// what each control may do now, decided with the gate's busy
    pub(crate) readiness: Readiness,
}
#[derive(Clone)]
pub(crate) struct Shared {
    pub(crate) recording_hotkey: Arc<recording::hotkey::Controller>,
    pub(crate) recording: Arc<Recording>,
    /// where recordings and the library are kept: the user's Documents in the app, a temporary
    /// folder in tests
    pub(crate) folder: PathBuf,
    /// how long a stop waits for every participant together before it goes on without them
    pub(crate) stop_within: Duration,
    pub(crate) bench: Arc<Mutex<Bench>>,
    pub(crate) ack: Arc<AtomicU64>,
    pub(crate) sequence: Arc<AtomicU64>,
    pub(crate) shutdown: Arc<AtomicBool>,
}
impl Shared {
    pub(crate) fn new(recording_hotkey: Arc<recording::hotkey::Controller>, bench: Bench) -> Self {
        Self {
            recording: recording_hotkey.recording().clone(),
            recording_hotkey,
            folder: storage::folders::recordings(),
            stop_within: Duration::from_secs(3),
            shutdown: bench.closing.clone(),
            bench: Arc::new(Mutex::new(bench)),
            ack: Arc::new(AtomicU64::new(0)),
            sequence: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl Shared {
    /// the library, taken from the bench and used with the bench let go
    pub(crate) fn library(&self) -> Arc<Library> {
        self.bench.locked().library.clone()
    }
    /// recordings are made and listed in `folder`
    #[cfg(test)]
    pub(crate) fn in_folder(mut self, folder: &Path) -> Self {
        self.folder = folder.to_path_buf();
        self.bench.locked().library = Arc::new(Library::new(folder.to_path_buf()));
        self
    }
}

impl Bench {
    pub(crate) fn settings(&self) -> &config::Store {
        self.recording_hotkey.settings()
    }
    pub(crate) fn new(recording_hotkey: Arc<recording::hotkey::Controller>) -> Self {
        Self {
            recording_hotkey,
            observers: None,
            last_observers: observers::Status::default(),
            #[cfg(feature = "developer-tools")]
            experiment: None,
            buttons: buttons::Buttons::new(0),
            vision: None,
            requested: json!({}),
            library_loaded: false,
            startup_attempted: false,
            last_health: Health::default(),
            capture: None,
            link: medius::Link::spawn(),
            library: Arc::new(Library::new(storage::folders::recordings())),
            jobs: library::jobs::Jobs::default(),
            channel: None,
            range: None,
            live: Live::default(),
            seen_serial: 0,
            seen_discontinuities: 0,
            device_names: HashMap::new(),
            names_for: Vec::new(),
            box_endpoint: None,
            closing: Arc::new(AtomicBool::new(false)),
        }
    }
}
impl Shared {
    /// a copy of a run from the library, to work on without holding the lock
    pub(crate) fn run(&self, id: &str) -> Result<Run, String> {
        self.library().run(id)
    }
}
impl Bench {
    pub(crate) fn recording(&self) -> bool {
        self.capture
            .as_ref()
            .is_some_and(|c| c.data.locked().recorder.is_some())
    }

    /// the live view's axes and tab data start again, for a new range or another mouse
    pub(crate) fn reset_view(&mut self) {
        self.live.reset();
    }

    pub(crate) fn snapshot(&self, seq: u64, views: Views) -> Snapshot {
        let (mut snapshot, unplotted) = self.snapshot_unplotted(seq);
        snapshot.plot = unplotted.plot(views);
        snapshot
    }

    /// the snapshot, and what its plot is computed from: a copy of the shown span, so the plot
    /// needs no lock
    pub(crate) fn snapshot_unplotted(&self, seq: u64) -> (Snapshot, Unplotted) {
        let firmware = self.link.snapshot();
        let (devices, selected, health, recording, recording_elapsed_ms, shown) =
            if let Some(capture) = &self.capture {
                let data = capture.data.locked();
                let mut devices = data.devices();
                for device in &mut devices {
                    let interface = data
                        .endpoints
                        .get(&key_of(device))
                        .map(|endpoint| endpoint.interface);
                    device.box_name = cloned_by(device, &firmware).or_else(|| {
                        self.box_endpoint
                            .filter(|(key, _)| *key == key_of(device))
                            .and_then(|_| box_label(&firmware))
                    });
                    device.name = windows_name(device, interface, &self.device_names)
                        .or_else(|| device.box_name.as_ref().and(clone_product(&firmware)));
                }
                (
                    devices,
                    data.selected
                        .map(|key| format!("{}:{}:{}", key.bus, key.device, key.endpoint)),
                    data.health.clone(),
                    data.recorder.as_ref().map(|r| r.stem().into()),
                    data.recorder.as_ref().map(|r| r.elapsed_ms()),
                    Some(detach(&data, self.range.as_ref())),
                )
            } else {
                (Vec::new(), None, self.last_health.clone(), None, None, None)
            };
        let unplotted = Unplotted {
            data: shown,
            range: self.range,
            scale: self.live.scale(),
            resets: self.live.resets(),
        };
        let recording_hotkey = self.recording_hotkey.snapshot();
        let readiness = readiness::readiness(&self.facts_from(
            recording_hotkey.busy,
            &firmware,
            &recording_hotkey,
        ));
        let job = self.jobs.current();
        let snapshot = Snapshot {
            dev_mode: cfg!(debug_assertions),
            recording_hotkey,
            observers: self
                .observers
                .as_ref()
                .map(|o| o.snapshot())
                .unwrap_or_else(|| self.last_observers.clone()),
            developer_tools: cfg!(feature = "developer-tools"),
            settings_unsaved: self.settings().unsaved(),
            #[cfg(feature = "developer-tools")]
            experiment: self.experiment.as_ref().map_or_else(
                || {
                    let mut status = experiment::Status::default();
                    self.buttons.status(&mut status);
                    status
                },
                |e| e.snapshot(),
            ),
            vision: self
                .vision
                .as_ref()
                .map(|v| v.snapshot())
                .unwrap_or_default(),
            seq,
            live: self.capture.is_some(),
            devices,
            selected,
            health,
            recording,
            recording_elapsed_ms,
            plot: Plot::default(),
            range: self.range,
            box_: firmware.view(),
            library: self.library.revision(),
            job,
            readiness,
        };
        (snapshot, unplotted)
    }

    /// what readiness decides from; `busy` is false for a command that holds the gate itself
    pub(crate) fn facts(&self, busy: bool) -> Facts {
        self.facts_from(
            busy,
            &self.link.snapshot(),
            &self.recording_hotkey.snapshot(),
        )
    }

    /// the facts, with the box and the recording button as the caller already read them
    pub(crate) fn facts_from(
        &self,
        busy: bool,
        firmware: &medius::Snapshot,
        hotkey: &recording::hotkey::Status,
    ) -> Facts {
        let (capture, selection, recording) = match &self.capture {
            None => (readiness::Capture::Stopped, None, false),
            Some(capture) => {
                let data = capture.data.locked();
                let state = if data.capture_failed() {
                    readiness::Capture::Failed
                } else {
                    readiness::Capture::Running
                };
                (state, data.checked(), data.recorder.is_some())
            }
        };
        let aim = self.buttons.aim();
        Facts {
            broken: self.recording_hotkey.recording().is_broken(),
            closing: self.closing.load(Ordering::Relaxed),
            busy,
            library_loaded: self.library_loaded,
            recording,
            capture,
            selection: match selection {
                None => readiness::Selection::None,
                Some(false) => readiness::Selection::Checking,
                Some(true) => readiness::Selection::Checked,
            },
            observers: self.observers.as_ref().map(|observer| {
                let status = observer.snapshot();
                readiness::Observers {
                    ready: status.raw_ready,
                    error: status.error,
                }
            }),
            vision: match &self.vision {
                Some(worker) => {
                    let status = worker.status.locked();
                    readiness::Vision {
                        started: true,
                        ready: status.ready,
                        error: status.error.clone(),
                        scenario_named: !worker.config.scenario.trim().is_empty(),
                        model_chosen: true,
                        preparing: vision::preparing(),
                    }
                }
                None => readiness::Vision {
                    started: false,
                    model_chosen: !self.settings().get().vision.model.trim().is_empty(),
                    preparing: vision::preparing(),
                    ..readiness::Vision::default()
                },
            },
            box_link: readiness::BoxLink {
                connected: firmware.connected,
                connecting: firmware.connecting,
                picking: firmware.picking,
                has_mouse: firmware.has_mouse,
                pending_commands: firmware.pending_commands,
                error: firmware.error.clone(),
            },
            buttons: self.buttons.require().err(),
            recording_button: readiness::RecordingButton {
                mask: hotkey.hotkey_mask,
                learning: hotkey.learning,
            },
            aim: readiness::Aim {
                armed: aim.armed,
                learning: aim.learning,
                error: self.aim_error(),
                mask: aim.mask,
            },
        }
    }

    pub(crate) fn readiness(&self, busy: bool) -> Readiness {
        readiness::readiness(&self.facts(busy))
    }
}

/// a command's own check: refused with the words the snapshot shows
pub(crate) fn allowed(blocker: Option<Blocker>) -> Result<(), String> {
    blocker.map_or(Ok(()), |blocker| Err(blocker.message.into_owned()))
}

/// the live span copied out of the capture, with the range and axes it is shown at
pub(crate) struct Unplotted {
    pub(crate) data: Option<session::Data>,
    pub(crate) range: Option<Selection>,
    pub(crate) scale: Option<Scale>,
    /// the live view's resets when the copy was taken
    pub(crate) resets: u64,
}
impl Unplotted {
    pub(crate) fn plot(&self, views: Views) -> Plot {
        self.data.as_ref().map_or_else(Plot::default, |data| {
            plot(data, self.range.as_ref(), self.scale, true, views)
        })
    }
}

/// `work` on a blocking thread: a command that writes files or waits on hardware holds neither
/// the window, which runs commands without `async` on its own thread, nor the async runtime
pub(crate) async fn off_main<T: Send + 'static>(
    state: State<'_, Shared>,
    work: impl FnOnce(&Shared) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || work(&shared))
        .await
        .map_err(|e| e.to_string())?
}

/// the aim hold button saved last time
#[cfg(feature = "developer-tools")]
pub(crate) fn saved_aim_mask(settings: &config::Store) -> u32 {
    settings.get().aim.button.mask()
}

/// without the aim controller no button is the aim's
#[cfg(not(feature = "developer-tools"))]
pub(crate) fn saved_aim_mask(_settings: &config::Store) -> u32 {
    0
}

/// the aim controller as a handle taken under the bench lock: disarming and stopping wait on the
/// box, so they run once the lock is let go
#[derive(Clone, Default)]
pub(crate) struct Aim(
    #[cfg(feature = "developer-tools")] pub(crate) Option<Arc<experiment::Worker>>,
);
#[cfg(feature = "developer-tools")]
impl Aim {
    pub(crate) fn disarm(&self) -> Result<(), String> {
        self.0.as_ref().map_or(Ok(()), |worker| worker.disarm())
    }
    pub(crate) fn end_recording(&self) -> Result<(), String> {
        self.0
            .as_ref()
            .map_or(Ok(()), |worker| worker.end_recording())
    }
    /// stopped, and why it failed if it did
    pub(crate) fn stop(&self) -> Option<String> {
        let worker = self.0.as_ref()?;
        worker.stop();
        worker.snapshot().error
    }
    /// whether the injection takes part in the recording starting at `pcap`
    pub(crate) fn begin(&self, pcap: &Path) -> Result<bool, String> {
        let Some(worker) = &self.0 else {
            return Ok(false);
        };
        worker.begin(pcap)?;
        Ok(worker.snapshot().recording)
    }
    pub(crate) fn abandon(&self, why: &str) {
        if let Some(worker) = &self.0 {
            worker.abandon(why);
        }
    }
}
#[cfg(not(feature = "developer-tools"))]
impl Aim {
    pub(crate) fn disarm(&self) -> Result<(), String> {
        Ok(())
    }
    pub(crate) fn end_recording(&self) -> Result<(), String> {
        Ok(())
    }
    pub(crate) fn stop(&self) -> Option<String> {
        None
    }
    pub(crate) fn begin(&self, _pcap: &Path) -> Result<bool, String> {
        Ok(false)
    }
    pub(crate) fn abandon(&self, _why: &str) {}
}

/// the aim controller, only in a build with the developer tools
#[cfg(feature = "developer-tools")]
impl Bench {
    pub(crate) fn aim(&self) -> Aim {
        Aim(self.experiment.clone())
    }
    /// `aim` is no longer the bench's, if it still was
    pub(crate) fn retire_aim(&mut self, aim: &Aim) {
        if let Some(stuck) = &aim.0 {
            stuck.retire();
        }
        self.experiment.take_if(|worker| {
            aim.0
                .as_ref()
                .is_some_and(|stuck| Arc::ptr_eq(stuck, worker))
        });
    }
    pub(crate) fn take_aim(&mut self) -> Aim {
        Aim(self.experiment.take())
    }
    pub(crate) fn aim_error(&self) -> Option<String> {
        self.experiment.as_ref().and_then(|e| e.snapshot().error)
    }
    pub(crate) fn aim_provenance(&self) -> Value {
        self.experiment
            .as_ref()
            .map_or(json!({"enabled": false}), |e| e.provenance())
    }
    /// whether the injection takes part in the recording starting at `pcap`
    pub(crate) fn aim_armed(&self) -> bool {
        self.experiment.as_ref().is_some_and(|e| e.snapshot().armed)
    }
}
#[cfg(not(feature = "developer-tools"))]
impl Bench {
    pub(crate) fn aim(&self) -> Aim {
        Aim()
    }
    pub(crate) fn retire_aim(&mut self, _aim: &Aim) {}
    pub(crate) fn take_aim(&mut self) -> Aim {
        Aim()
    }
    pub(crate) fn aim_error(&self) -> Option<String> {
        None
    }
    pub(crate) fn aim_provenance(&self) -> Value {
        json!({"enabled": false})
    }
    pub(crate) fn aim_armed(&self) -> bool {
        false
    }
}
