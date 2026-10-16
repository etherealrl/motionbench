//! target capture and the response delay
use crate::{
    app::state::{Aim, Shared, allowed, off_main},
    recording,
};
use motion_bench::sync::LockExt;

use crate::targets::vision;

use crate::library::Run;
use crate::recording::Recording;
use motion_bench::evidence::reader::layout::locate;
use std::path::Path;
use tauri::State;

#[tauri::command]
pub(crate) async fn screen_start(state: State<'_, Shared>) -> Result<(), String> {
    off_main(state, start_screen_capture).await
}

pub(crate) fn start_screen_capture(state: &Shared) -> Result<(), String> {
    let _claim = Recording::claim(state)?;
    let (aim, previous) = {
        let mut bench = state.bench.locked();
        allowed(bench.readiness(false).screen_capture)?;
        (bench.aim(), bench.vision.take())
    };
    if let Some(previous) = previous {
        stop_screen(state, aim, previous)?;
    }
    let mut bench = state.bench.locked();
    bench.vision = Some(vision::Worker::start(bench.settings().get().vision)?);
    Ok(())
}

/// target capture stopped, with the aim that followed it disarmed first; both wait on hardware, so
/// they run without the bench lock
pub(crate) fn stop_screen(
    state: &Shared,
    aim: Aim,
    mut vision: vision::Worker,
) -> Result<(), String> {
    let disarmed = recording::disarm_within(state, aim);
    vision.stop();
    disarmed
}

/// the estimate is its own file beside the recording. the recording's metadata was published
/// once when it finished and is never rewritten
pub(crate) fn save_calibration(
    pcap: &Path,
    result: &motion_bench::targets::calibration::Response,
) -> Result<(), String> {
    let path = pcap.with_extension("calibration.json");
    let temporary = path.with_extension("json.tmp");
    std::fs::write(
        &temporary,
        serde_json::to_vec_pretty(result).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::rename(temporary, path).map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn calibrate_response(
    id: String,
    stationary_target: bool,
    state: State<'_, Shared>,
) -> Result<motion_bench::targets::calibration::Response, String> {
    if !stationary_target {
        return Err(
            "confirm that this recording kept the same stationary target and used no clicks".into(),
        );
    }
    // a saved recording is read from its files, capture going on meanwhile changes nothing
    let run = state.run(&id)?;
    tauri::async_runtime::spawn_blocking(move || calibrate(&run))
        .await
        .map_err(|e| e.to_string())?
}
/// the response delay from a run's recording, inside its crop, saved beside it
pub(crate) fn calibrate(run: &Run) -> Result<motion_bench::targets::calibration::Response, String> {
    let pcap = locate(Path::new(&run.path))?.pcap;
    let result = motion_bench::targets::calibration::recording(&pcap, run.crop)?;
    save_calibration(&pcap, &result)?;
    Ok(result)
}
#[tauri::command]
pub(crate) async fn screen_stop(state: State<'_, Shared>) -> Result<(), String> {
    off_main(state, stop_screen_capture).await
}
pub(crate) fn stop_screen_capture(state: &Shared) -> Result<(), String> {
    let _claim = Recording::claim(state)?;
    let (aim, vision) = {
        let mut bench = state.bench.locked();
        // with nothing running the readiness is the start's, and a stop has nothing to do
        if bench.vision.is_none() && !bench.recording() {
            return Ok(());
        }
        allowed(bench.readiness(false).screen_capture)?;
        (bench.aim(), bench.vision.take())
    };
    vision.map_or(Ok(()), |vision| stop_screen(state, aim, vision))
}

/// the size of the display target capture would copy, so the FOV can be shown along either axis
/// before it starts. none when there's no such display
#[tauri::command]
pub(crate) fn display_size(adapter: u32, output: u32) -> Option<[u32; 2]> {
    #[cfg(windows)]
    {
        use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
        let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.ok()?;
        let adapter = unsafe { factory.EnumAdapters1(adapter) }.ok()?;
        let output = unsafe { adapter.EnumOutputs(output) }.ok()?;
        let area = unsafe { output.GetDesc() }.ok()?.DesktopCoordinates;
        Some([
            (area.right - area.left) as u32,
            (area.bottom - area.top) as u32,
        ])
    }
    #[cfg(not(windows))]
    {
        let _ = (adapter, output);
        None
    }
}

/// the class names a target model lists in its metadata, none when it lists none
#[tauri::command]
pub(crate) async fn model_classes(path: String) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
        motion_bench::targets::classes::class_names(&mut file).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
