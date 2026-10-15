//! the saved recordings: their names, crops, plots and picked devices
use crate::{
    app::state::{Shared, off_main},
    recording,
};
use motion_bench::sync::LockExt;

use crate::library::{Run, import};
use motion_bench::{
    analysis::{
        compare::Selection,
        views::{
            numbers::{Numbers, numbers},
            plot::{Plot, Scale, Views, plot},
            relay,
        },
    },
    capture::{hid::Decoder, session},
    coordination::readiness,
    evidence::{
        reader::{
            layout::{Located, locate},
            recording::open_recording,
        },
        recorder,
    },
};
use std::path::Path;
use tauri::State;

#[tauri::command]
pub(crate) async fn delete_run(id: String, state: State<'_, Shared>) -> Result<(), String> {
    off_main(state, move |state| {
        if recording::broken(state) {
            return Err(readiness::BROKEN.into());
        }
        let (library, jobs) = {
            let bench = state.bench.locked();
            (bench.library.clone(), bench.jobs.clone())
        };
        library.remove(&id, &jobs)
    })
    .await
}

#[tauri::command]
pub(crate) async fn rename_run(
    id: String,
    name: String,
    state: State<'_, Shared>,
) -> Result<(), String> {
    off_main(state, move |state| {
        if recording::broken(state) {
            return Err(readiness::BROKEN.into());
        }
        state.library().rename(&id, &name)
    })
    .await
}

#[tauri::command]
pub(crate) async fn import_run(path: String, state: State<'_, Shared>) -> Result<Run, String> {
    if recording::broken(&state) {
        return Err(readiness::BROKEN.into());
    }
    let run = tauri::async_runtime::spawn_blocking(move || import(Path::new(&path)))
        .await
        .map_err(|e| e.to_string())??;
    state.library().add(run)
}

/// `crop` checked against the recording: at least 10 ms long, clamped to the selected mouse's
/// reports and holding some of them. one that holds every report is no crop
pub(crate) fn checked_crop(run: &Run, crop: Selection) -> Result<Option<Selection>, String> {
    let whole = Run {
        crop: None,
        ..run.clone()
    };
    let (_, data) = open_run(&whole)?;
    let stream = data
        .selected
        .and_then(|key| data.bus.streams.get(&key))
        .ok_or("pick the recording's mouse before cropping it")?;
    let (Some(first), Some(last)) = (stream.first_us(), stream.last_us()) else {
        return Err("the recording has no reports of its mouse to crop".into());
    };
    let clamped = Selection {
        from_us: crop.from_us.max(first),
        to_us: crop.to_us.min(last),
    };
    if clamped.to_us < clamped.from_us.saturating_add(10_000) {
        return Err("keep at least 10 ms of the recording".into());
    }
    if stream.view(clamped.from_us, clamped.to_us).reports() == 0 {
        return Err("the crop holds no reports of the recording's mouse".into());
    }
    let all = Selection {
        from_us: first,
        to_us: last,
    };
    Ok((clamped != all).then_some(clamped))
}

/// the span of a recording every analysis reads, or none for all of it. the recording's files
/// never change, the crop is kept in the library like the name
#[tauri::command]
pub(crate) async fn set_crop(
    id: String,
    crop: Option<Selection>,
    state: State<'_, Shared>,
) -> Result<(), String> {
    off_main(state, move |state| crop_run(state, &id, crop)).await
}
pub(crate) fn crop_run(state: &Shared, id: &str, crop: Option<Selection>) -> Result<(), String> {
    if recording::broken(state) {
        return Err(readiness::BROKEN.into());
    }
    let run = state.run(id)?;
    let crop = crop
        .map(|crop| checked_crop(&run, crop))
        .transpose()?
        .flatten();
    let (library, jobs) = {
        let bench = state.bench.locked();
        (bench.library.clone(), bench.jobs.clone())
    };
    library.set_crop(id, crop, &jobs)
}

/// a run's files and its capture read back, with the device it compares selected
pub(crate) fn open_run(run: &Run) -> Result<(Located, session::Data), String> {
    let located = locate(Path::new(&run.path))?;
    let mut data = open_recording(&located.pcap, Some(run.decoder.clone()))?.into_data();
    if let Some(selected) = &run.selected {
        data.select(selected, None)?;
    }
    // every view's range counts from the selected stream's first report, which becomes the crop's
    if let Some(crop) = run.crop {
        for stream in data.bus.streams.values_mut() {
            *stream = stream.copy(crop.from_us, crop.to_us);
        }
        if data
            .selected
            .is_some_and(|key| data.bus.streams[&key].first_us().is_none())
        {
            return Err("the crop holds no reports of the recording's mouse".into());
        }
    }
    Ok((located, data))
}
/// a run's plot, of the wire, or with `input` of what the box saw on its side of the mouse,
/// lined up with the wire's clock. `uncropped` shows the whole recording, for choosing a crop
#[tauri::command]
pub(crate) async fn run_plot(
    id: String,
    range: Option<[f64; 2]>,
    scale: Option<Scale>,
    input: Option<bool>,
    uncropped: Option<bool>,
    state: State<'_, Shared>,
) -> Result<Plot, String> {
    let run = plotted(&state, &id, uncropped)?;
    tauri::async_runtime::spawn_blocking(move || {
        if input == Some(true) {
            input_plot(&run, range, scale)
        } else {
            wire_plot(&run, range, scale, true)
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
/// where target capture saw several targets at once, for the crop to leave out
#[tauri::command]
pub(crate) async fn run_crowded(
    id: String,
    state: State<'_, Shared>,
) -> Result<Vec<[u64; 2]>, String> {
    let run = state.run(&id)?;
    tauri::async_runtime::spawn_blocking(move || {
        motion_bench::targets::vision::crowded_spans(std::path::Path::new(&run.path))
    })
    .await
    .map_err(|e| e.to_string())?
}
/// the run a plot reads, the whole recording with `uncropped`
pub(crate) fn plotted(state: &Shared, id: &str, uncropped: Option<bool>) -> Result<Run, String> {
    let mut run = state.run(id)?;
    if uncropped == Some(true) {
        run.crop = None;
    }
    Ok(run)
}

/// a run's figures for the Comparison tab's table, from the same plot its tabs show
#[tauri::command]
pub(crate) async fn run_numbers(
    id: String,
    range: Option<[f64; 2]>,
    input: Option<bool>,
    state: State<'_, Shared>,
) -> Result<Numbers, String> {
    let run = state.run(&id)?;
    // the figures need neither the relay nor a shared scale
    tauri::async_runtime::spawn_blocking(move || {
        if input == Some(true) {
            input_plot(&run, range, None)
        } else {
            wire_plot(&run, range, None, false)
        }
        .map(|plot| numbers(&plot))
    })
    .await
    .map_err(|e| e.to_string())?
}
/// `range`, seconds from `first`, the way every comparison reads a range
pub(crate) fn absolute(run: &Run, first: u64, range: [f64; 2]) -> Result<Selection, String> {
    Selection::from_seconds(first, range, run.crop).map_err(|e| e.to_string())
}
/// what the box saw on its side of the mouse, on the wire's clock
pub(crate) fn box_input(located: &Located) -> Result<motion_bench::signal::stats::Stream, String> {
    let (input, shift) = recorder::read_input(&located.input).map_err(|e| e.to_string())?;
    Ok(relay::aligned(&input, shift))
}
/// with `relay`, a run with the box's input also gets its Relay tab
pub(crate) fn wire_plot(
    run: &Run,
    range: Option<[f64; 2]>,
    scale: Option<Scale>,
    relay: bool,
) -> Result<Plot, String> {
    let (located, data) = open_run(run)?;
    let range = range
        .and_then(|r| {
            data.selected
                .and_then(|key| data.bus.streams.get(&key)?.first_us())
                .map(|first| absolute(run, first, r))
        })
        .transpose()?;
    let mut plot = plot(&data, range.as_ref(), scale, false, Views::All);
    // without a decoder the wire has no counts to set against the box's
    if relay
        && run.paired
        && plot.movement_available
        && let Some(key) = data.selected
    {
        let input = box_input(&located)?;
        let (from, to) = (plot.from_us, plot.to_us);
        plot.tabs.relay = Some(relay::relay(
            &data.bus.streams[&key].view(from, to),
            &input,
            plot.readouts.period_us,
        ));
    }
    Ok(plot)
}
#[tauri::command]
pub(crate) async fn select_run_device(
    id: String,
    device: String,
    state: State<'_, Shared>,
) -> Result<(), String> {
    if recording::broken(&state) {
        return Err(readiness::BROKEN.into());
    }
    let run = state.run(&id)?;
    let selected = device.clone();
    let decode = tauri::async_runtime::spawn_blocking(move || {
        let located = locate(Path::new(&run.path))?;
        let override_decoder =
            (!matches!(run.decoder, Decoder::Unavailable)).then_some(run.decoder);
        let mut data = open_recording(&located.pcap, override_decoder)?.into_data();
        data.select(&selected, None)?;
        Ok::<_, String>(
            data.decoders
                .get(&data.selected.unwrap())
                .cloned()
                .unwrap_or_default(),
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    state.library().pick(&id, device, decode)
}

pub(crate) fn input_plot(
    run: &Run,
    range: Option<[f64; 2]>,
    scale: Option<Scale>,
) -> Result<Plot, String> {
    {
        let (located, wire) = open_run(run)?;
        let key = wire.selected.ok_or("select endpoint first")?;
        let stream = &wire.bus.streams[&key];
        let first = stream.first_us().unwrap_or(0);
        let last = stream.last_us().unwrap_or(first);
        let range = match range {
            Some(r) => absolute(run, first, r)?,
            None => Selection {
                from_us: first,
                to_us: last,
            },
        };
        let mut data = session::Data::new(false);
        data.bus.streams.insert(key, box_input(&located)?);
        data.selected = Some(key);
        Ok(plot(&data, Some(&range), scale, false, Views::All))
    }
}

/// the library's runs, read when its revision moves
#[tauri::command]
pub(crate) fn runs(state: State<Shared>) -> crate::library::Listed {
    state.library().listed()
}

/// a recording with target capture, played back
#[tauri::command]
pub(crate) async fn run_replay(
    id: String,
    state: State<'_, Shared>,
) -> Result<motion_bench::targets::replay::Replay, String> {
    let run = state.run(&id)?;
    tauri::async_runtime::spawn_blocking(move || {
        motion_bench::targets::replay::replay(std::path::Path::new(&run.path))
    })
    .await
    .map_err(|e| e.to_string())?
}
