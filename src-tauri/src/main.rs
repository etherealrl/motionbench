#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use motion_bench::sync::LockExt;
mod app;
mod capture;
mod commands;
mod config;
mod controller;
mod hardware;
mod library;
mod recording;
mod storage;
mod targets;

use app::{
    pump::pump,
    state::{Bench, Shared, saved_aim_mask},
};

use crate::hardware::buttons;

use motion_bench::analysis::compare;
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tauri::Manager;

/// the commands the ui may call, and `extra` ones; a build with developer tools adds the aim
/// controller's
macro_rules! handlers {
    ($($extra:path),*) => {
        tauri::generate_handler![
            commands::session::initialize,
            commands::session::settings,
            commands::session::update_settings,
            commands::session::subscribe,
            commands::session::acknowledge,
            commands::session::start_capture,
            commands::session::stop_capture,
            commands::session::select_device,
            commands::session::set_range,
            recording::hotkey::recording_toggle,
            recording::hotkey::recording_hotkey_learn,
            recording::hotkey::recording_hotkey_disable,
            commands::targets::screen_start,
            commands::targets::calibrate_response,
            commands::targets::screen_stop,
            commands::boxes::box_command,
            commands::runs::import_run,
            commands::runs::rename_run,
            commands::runs::set_crop,
            commands::runs::delete_run,
            commands::runs::run_plot,
            commands::runs::run_crowded,
            commands::runs::run_replay,
            commands::runs::run_numbers,
            commands::targets::model_classes,
            commands::targets::display_size,
            commands::targets::displays,
            commands::session::set_tab,
            commands::runs::select_run_device,
            commands::analysis::analyze,
            commands::analysis::cancel_analysis,
            commands::analysis::export_result,
            commands::analysis::analysis_result,
            commands::analysis::saved_result,
            commands::runs::runs,
            commands::session::open_folder,
            commands::session::set_recordings_folder,
            commands::session::recordings_folders,
            commands::session::restart_app,
            commands::session::check_update,
            commands::session::install_update,
            commands::session::open_judge_commit,
            $($extra),*
        ]
    };
}

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|a| a == "judge") {
        attach_console();
        std::process::exit(compare::cli(args));
    }
    let settings = Arc::new(config::Store::load());
    if let Some(folder) = settings.get().storage.folder {
        storage::folders::choose(folder);
    }
    let (recording_hotkey, recording_requests) = recording::hotkey::Controller::new(settings);
    let recording_hotkey = Arc::new(recording_hotkey);
    let mut bench = Bench::new(recording_hotkey.clone());
    bench.buttons = buttons::Buttons::new(saved_aim_mask(bench.settings()));
    let feed = bench.link.take_buttons();
    let shared = Shared::new(recording_hotkey.clone(), bench);
    if let Some(feed) = feed {
        let (buttons, shutdown) = (
            shared.bench.locked().buttons.clone(),
            shared.shutdown.clone(),
        );
        std::thread::spawn(move || buttons.route(feed, recording_hotkey, shutdown));
    }
    let worker = shared.clone();
    std::thread::spawn(move || {
        while !worker.shutdown.load(Ordering::Relaxed) {
            // a panic here must not freeze the window, the next round shows the app broken; it
            // waits a little so a panic that comes back every time does not spin
            let pause =
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pump(&worker))) {
                    Ok(()) => 2,
                    // a panic that poisoned a lock breaks the app through `broken` below, one in
                    // the plot or the send only cost this frame
                    Err(_) => {
                        // the frame it counted may never have been sent, the page would wait for
                        // it forever
                        worker
                            .ack
                            .store(worker.sequence.load(Ordering::Relaxed), Ordering::Relaxed);
                        100
                    }
                };
            recording::broken(&worker);
            std::thread::sleep(Duration::from_millis(pause));
        }
    });
    let recording_worker = shared.clone();
    std::thread::spawn(move || {
        while !recording_worker.shutdown.load(Ordering::Relaxed) {
            if let Ok(requested) = recording_requests.recv_timeout(Duration::from_millis(100)) {
                let toggled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    recording::toggle(&recording_worker, requested, None)
                }));
                // the press is given back, or the button would stay refused for good
                if toggled.is_err() {
                    recording_worker.recording.unpress();
                    recording::caught(&recording_worker);
                }
            }
        }
    });
    let app = tauri::Builder::default();
    // the end to end tests drive the app through it, a release never has it
    #[cfg(feature = "e2e")]
    let app = app.plugin(tauri_plugin_wdio_webdriver::init());
    let app = app
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(shared);
    #[cfg(feature = "developer-tools")]
    let app = app.invoke_handler(handlers!(
        commands::experiment::experiment_arm,
        commands::experiment::experiment_disarm,
        commands::experiment::experiment_learn
    ));
    #[cfg(not(feature = "developer-tools"))]
    let app = app.invoke_handler(handlers!());
    let app = app
        .build(tauri::generate_context!())
        .expect("could not start desktop app");
    app.run(|app, event| match event {
        // the window goes at once, the exit may take a while saving the recording
        tauri::RunEvent::ExitRequested { .. } => {
            for window in app.webview_windows().values() {
                let _ = window.hide();
            }
        }
        tauri::RunEvent::Exit => {
            recording::close(&app.state::<Shared>(), Duration::from_secs(30));
        }
        _ => {}
    });
}
fn attach_console() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(pid: u32) -> i32;
    }
    unsafe {
        AttachConsole(u32::MAX);
    }
}

#[cfg(test)]
#[path = "readiness/tests.rs"]
mod readiness_tests;
#[cfg(test)]
mod tests;
#[cfg(all(test, feature = "developer-tools"))]
mod ui_fixture;
