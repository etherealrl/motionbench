//! starting the app, its settings, monitoring and the picked mouse
use crate::{
    app::{
        pump::key_of,
        state::{Bench, Shared, Snapshot, off_main},
    },
    commands::targets::start_screen_capture,
    config, recording,
};
use motion_bench::sync::LockExt;

use crate::capture::{connected_hid, observers};

use crate::recording::Recording;
use motion_bench::{
    analysis::{
        compare::Selection,
        views::plot::{Tab, Views},
    },
    capture::{hid::Decoder, pcap, session::Capture},
    hardware::medius::Command,
};
use serde_json::{Value, json};
use std::sync::atomic::Ordering;
use tauri::{State, ipc::Channel};

#[tauri::command]
pub(crate) async fn initialize(state: State<'_, Shared>) -> Result<Value, String> {
    let library = state.library();
    let restore = !library.restored();
    let library_warnings = if !restore {
        Vec::new()
    } else {
        tauri::async_runtime::spawn_blocking(move || library.restore())
            .await
            .map_err(|e| e.to_string())?
    };
    let mut warnings = Vec::new();
    // a folder on a drive that's gone would just look like an empty library until a recording fails
    let folder = state.folder.clone();
    if let Err(e) = tauri::async_runtime::spawn_blocking(move || writable(&folder))
        .await
        .map_err(|e| e.to_string())?
    {
        warnings.push(format!(
            "{e}, so recordings can't go there. plug its drive back in or pick another folder in the settings"
        ));
    }
    let roots = tauri::async_runtime::spawn_blocking(|| {
        pcap::usbpcapcmd()
            .map(|exe| pcap::roots(&exe))
            .unwrap_or(Ok(Vec::new()))
    })
    .await
    .map_err(|e| e.to_string())?
    .unwrap_or_else(|error| {
        warnings.push(error);
        Vec::new()
    });
    let (root, start_targets) = {
        let mut bench = state.bench.locked();
        bench.library_loaded = true;
        let config = bench.settings().get();
        warnings.extend(bench.settings().notice());
        if let Some(saved_root) = &config.capture.root
            && !roots.contains(saved_root)
        {
            warnings.push(format!(
            "the saved USB root {saved_root} isn't there anymore, so choose one that's connected"
        ));
        }
        let root = config
            .capture
            .root
            .filter(|root| roots.contains(root))
            .or_else(|| roots.first().cloned());
        if !bench.startup_attempted {
            bench.startup_attempted = true;
            bench.link.send(Command::Start(config.capture.box_.clone()));
            if let Some(root) = &root {
                if let Err(error) = start_capture_inner(root, &mut bench) {
                    bench.last_health.error = Some(format!(
                        "USB auto-start failed: {error}. start monitoring to try again"
                    ));
                }
            } else {
                bench.last_health.error = Some(
                "USB auto-start found no root hubs, so either USBPcap isn't installed or you have to start monitoring by hand"
                    .into(),
            );
            }
        }
        // only the first initialize starts anything, a reload of the window doesn't
        (root, restore && config.targets.start_on_launch)
    };
    if start_targets {
        let shared = state.inner().clone();
        let started = tauri::async_runtime::spawn_blocking(move || start_screen_capture(&shared))
            .await
            .map_err(|e| e.to_string())?;
        if let Err(error) = started {
            warnings.push(format!("target capture didn't start by itself: {error}"));
        }
    }
    let bench = state.bench.locked();
    Ok(
        json!({"roots":roots,"root":root,"folder":state.folder,"warnings":warnings,"library_warnings":library_warnings,"snapshot":bench.snapshot(0, Views::Readouts)}),
    )
}
#[tauri::command]
pub(crate) fn settings(state: State<Shared>) -> config::Settings {
    state.recording_hotkey.settings().get()
}
/// the settings as saved after the change, or why the limits refused it
#[tauri::command]
pub(crate) async fn update_settings(
    patch: config::Patch,
    state: State<'_, Shared>,
) -> Result<config::Settings, String> {
    off_main(state, move |state| {
        state.recording_hotkey.settings().update(|s| patch.apply(s))
    })
    .await
}
#[tauri::command]
pub(crate) fn subscribe(channel: Channel<Snapshot>, state: State<Shared>) {
    state
        .ack
        .store(state.sequence.load(Ordering::Relaxed), Ordering::Relaxed);
    state.bench.locked().channel = Some(channel);
}
#[tauri::command]
pub(crate) fn acknowledge(seq: u64, state: State<Shared>) {
    state.ack.fetch_max(
        seq.min(state.sequence.load(Ordering::Relaxed)),
        Ordering::Relaxed,
    );
}
#[tauri::command]
pub(crate) async fn start_capture(root: String, state: State<'_, Shared>) -> Result<(), String> {
    off_main(state, move |state| start_capture_at(state, root)).await
}

pub(crate) fn start_capture_at(state: &Shared, root: String) -> Result<(), String> {
    // a capture stop in flight has let go of the lock but not of the capture
    let _claim = Recording::claim(state)?;
    let mut bench = state.bench.locked();
    start_capture_inner(&root, &mut bench)?;
    // the limits never refuse a capture choice; one that is not written shows as unsaved
    let _ = bench
        .settings()
        .update(|s| s.capture.root = Some(root.clone()));
    Ok(())
}
pub(crate) fn start_capture_inner(root: &str, bench: &mut Bench) -> Result<(), String> {
    if bench.capture.is_some() {
        return Err("capture is already running".into());
    }
    bench.capture = Some(Capture::start(root)?);
    bench.observers = Some(observers::Worker::start());
    bench.range = None;
    bench.reset_view();
    Ok(())
}

#[tauri::command]
pub(crate) async fn stop_capture(state: State<'_, Shared>) -> Result<Option<String>, String> {
    off_main(state, recording::stop_capture).await
}

/// the recordings folder in Explorer
#[tauri::command]
pub(crate) fn open_folder(state: State<'_, Shared>) -> Result<(), String> {
    std::process::Command::new("explorer")
        .arg(&state.folder)
        .spawn()
        .map(drop)
        .map_err(|e| format!("couldn't open {}: {e}", state.folder.display()))
}
/// where recordings go from the next start on, or back to the default with none. the folder has to take
/// a file now, so a drive that's gone or read only says so here rather than at the next recording
#[tauri::command]
pub(crate) async fn set_recordings_folder(
    folder: Option<String>,
    state: State<'_, Shared>,
) -> Result<config::Settings, String> {
    off_main(state, move |state| {
        let folder = folder.map(std::path::PathBuf::from);
        if let Some(folder) = &folder {
            writable(folder)?;
        }
        state
            .recording_hotkey
            .settings()
            .update(|s| s.storage.folder = folder)
    })
    .await
}
/// the folder this run keeps recordings in, and the one it would use with none picked
#[tauri::command]
pub(crate) fn recordings_folders(state: State<'_, Shared>) -> serde_json::Value {
    json!({"running": state.folder, "fallback": crate::storage::folders::default_recordings()})
}
pub(crate) fn writable(folder: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(folder)
        .map_err(|e| format!("couldn't make {}: {e}", folder.display()))?;
    // a name nothing else uses and made new, so the check never touches a file that was already there
    let probe = folder.join(format!(".motion-bench-write-check-{}", std::process::id()));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .and_then(|_| std::fs::remove_file(&probe))
        .map_err(|e| format!("can't write in {}: {e}", folder.display()))
}
/// a recording running, starting or still saving would be cut short by closing the app
fn refuse_while_recording(state: &Shared) -> Result<(), String> {
    if state.recording.busy() || state.recording.recording() {
        return Err("a recording is still running or saving, try again once it's done".into());
    }
    Ok(())
}
/// starts the app again, which is how a new recordings folder gets used. it goes through the
/// app's normal exit, since a plain restart from here skips it and with it closing capture
#[tauri::command]
pub(crate) fn restart_app(app: tauri::AppHandle, state: State<'_, Shared>) -> Result<(), String> {
    refuse_while_recording(&state)?;
    app.request_restart();
    Ok(())
}
/// the version of a newer release, if there is one
#[tauri::command]
pub(crate) async fn check_update(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_updater::UpdaterExt;
    let update = app.updater().map_err(|e| e.to_string())?.check().await;
    Ok(update.map_err(|e| e.to_string())?.map(|u| u.version))
}
/// downloads the newer release and runs its installer, which closes the app and opens the new one.
/// the recording is checked again after the download, a mouse button can start one meanwhile. the
/// installer ends the process without the app's normal exit, so that runs first
#[tauri::command]
pub(crate) async fn install_update(
    app: tauri::AppHandle,
    state: State<'_, Shared>,
) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    refuse_while_recording(&state)?;
    let closing = state.inner().clone();
    let update = app
        .updater_builder()
        .on_before_exit(move || recording::close(&closing, std::time::Duration::from_secs(30)))
        .build()
        .map_err(|e| e.to_string())?
        .check()
        .await;
    let Some(update) = update.map_err(|e| e.to_string())? else {
        return Ok(());
    };
    let bytes = update
        .download(|_, _| {}, || {})
        .await
        .map_err(|e| format!("couldn't download the update: {e}"))?;
    refuse_while_recording(&state)?;
    update
        .install(bytes)
        .map_err(|e| format!("couldn't install the update: {e}"))
}
#[tauri::command]
pub(crate) async fn select_device(
    id: String,
    state: State<'_, Shared>,
) -> Result<Option<String>, String> {
    let identity = {
        let bench = state.bench.locked();
        let capture = bench.capture.as_ref().ok_or("start capture first")?;
        let data = capture.data.locked();
        if data.recorder.is_some() {
            return Err("stop recording before changing the mouse layout".into());
        }
        // the pick is by id, and the same mouse at a new address is still the one that was picked
        let device = data.device(&id).ok_or("device is no longer available")?;
        let key = key_of(&device);
        if data
            .decoders
            .get(&key)
            .is_none_or(|d| matches!(d, Decoder::Unavailable))
        {
            device
                .vid
                .zip(device.pid)
                .zip(data.endpoints.get(&key).map(|e| e.interface))
                .map(|((vid, pid), interface)| (vid, pid, interface))
        } else {
            None
        }
    };
    let mut warning = None;
    let mut windows_layout = None;
    if let Some((vid, pid, interface)) = identity {
        match tauri::async_runtime::spawn_blocking(move || connected_hid::read(vid, pid, interface))
            .await
            .map_err(|e| e.to_string())?
        {
            Ok(connected) => {
                warning = Some(format!(
                    "movement layout detected from Windows, the backup descriptor is at {}",
                    connected.path.display()
                ));
                windows_layout = Some(connected.layout);
            }
            Err(error) => {
                warning = Some(format!(
                    "timing works but movement decoding doesn't: {error}. the thing is automatic decoding can also use a report descriptor captured when the mouse connects"
                ))
            }
        }
    }
    let mut bench = state.bench.locked();
    let capture = bench.capture.as_mut().ok_or("start capture first")?;
    let mut data = capture.data.locked();
    if let Some(layout) = windows_layout {
        let device = data
            .device(&id)
            .ok_or("device disconnected during layout lookup")?;
        data.remember_windows_layout(key_of(&device), layout);
    }
    data.select(&id, None)?;
    // a view paused on the old selection says nothing about the new one
    data.bus.pin(None);
    let device = data.device(&id).ok_or("selected mouse disappeared")?;
    let root = capture.root.clone();
    drop(data);
    bench.range = None;
    bench.reset_view();
    let _ = bench.settings().update(|s| {
        s.capture.root = Some(root);
        s.capture.mouse = Some(config::SavedMouse {
            vid: device.vid,
            pid: device.pid,
            bus: device.bus,
            endpoint: device.endpoint,
        });
    });
    Ok(warning)
}
#[tauri::command]
pub(crate) fn set_range(range: Option<Selection>, state: State<Shared>) -> Result<(), String> {
    if range.as_ref().is_some_and(|r| r.to_us <= r.from_us) {
        return Err("that time range is empty".into());
    }
    let mut bench = state.bench.locked();
    // a range picked from the live view stays whole while it is shown, the live view keeps
    // only its last seconds otherwise
    if let Some(capture) = &bench.capture {
        capture
            .data
            .locked()
            .bus
            .pin(range.as_ref().map(|r| r.from_us));
    }
    bench.range = range;
    bench.reset_view();
    Ok(())
}
/// the live view computes only the tab it shows
#[tauri::command]
pub(crate) fn set_tab(tab: Tab, state: State<Shared>) {
    state.bench.locked().live.show(tab);
}

/// the judge's commit that made a result, in the browser. only that repository is opened
#[tauri::command]
pub(crate) fn open_judge_commit(revision: String) -> Result<(), String> {
    if revision.len() != 40 || !revision.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("that isn't a commit".into());
    }
    let url = format!("https://github.com/optima-manent/ABCurves/commit/{revision}");
    // explorer hands a url to the default browser
    std::process::Command::new("explorer")
        .arg(&url)
        .spawn()
        .map(drop)
        .map_err(|e| format!("couldn't open {url}: {e}"))
}
