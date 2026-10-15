//! the developer-only aim controller
use crate::{
    app::state::{Bench, Shared, allowed, off_main},
    recording,
};
use motion_bench::sync::LockExt;

#[cfg(feature = "developer-tools")]
use crate::controller::experiment;

use crate::recording::Recording;
use std::sync::Arc;
use tauri::State;

#[cfg(feature = "developer-tools")]
pub(crate) fn experiment_worker(bench: &mut Bench) -> Result<&experiment::Worker, String> {
    if bench.experiment.is_none() {
        let worker = experiment::Worker::start(bench.link.injection_port()?, bench.buttons.clone());
        bench.experiment = Some(Arc::new(worker));
    }
    Ok(bench.experiment.as_ref().unwrap())
}
#[cfg(feature = "developer-tools")]
#[tauri::command]
pub(crate) fn experiment_learn(cancel: bool, state: State<Shared>) -> Result<(), String> {
    let bench = state.bench.locked();
    if cancel {
        return bench.buttons.learn_aim(true);
    }
    // it takes no gate, so an action on its way still stops it
    allowed(
        bench
            .readiness(state.recording_hotkey.snapshot().busy)
            .aim_button,
    )?;
    bench.buttons.learn_aim(false)
}
#[cfg(feature = "developer-tools")]
#[tauri::command]
pub(crate) async fn experiment_arm(state: State<'_, Shared>) -> Result<(), String> {
    off_main(state, arm_experiment).await
}

#[cfg(feature = "developer-tools")]
pub(crate) fn arm_experiment(state: &Shared) -> Result<(), String> {
    let aim = state.recording_hotkey.settings().get().aim;
    let config = motion_bench::controller::linear::Config {
        speed: aim.speed,
        deadzone_counts: aim.deadzone_counts,
        hotkey_mask: aim.button.mask(),
    };
    let _claim = Recording::claim(state)?;
    allowed(state.bench.locked().readiness(false).arm)?;
    // a worker that failed is replaced; stopping it waits on the box, so not under the lock
    let failed = {
        let mut bench = state.bench.locked();
        bench
            .experiment
            .take_if(|worker| worker.snapshot().error.is_some())
    };
    // it goes before the new one is made, and touches nothing the new one shares
    if let Some(failed) = failed {
        failed.retire();
        recording::bounded(state, move || failed.stop());
    }
    let mut bench = state.bench.locked();
    allowed(bench.readiness(false).arm)?;
    let vision = bench
        .vision
        .as_ref()
        .ok_or("start target capture in Capture first")?;
    let source = experiment::Source {
        config: vision.config.clone(),
        status: vision.status.clone(),
    };
    experiment_worker(&mut bench)?.arm(config, source)
}
#[cfg(feature = "developer-tools")]
#[tauri::command]
pub(crate) async fn experiment_disarm(state: State<'_, Shared>) -> Result<(), String> {
    off_main(state, disarm_experiment).await
}
/// the box's discard waits on it, so it runs without the bench lock
#[cfg(feature = "developer-tools")]
pub(crate) fn disarm_experiment(state: &Shared) -> Result<(), String> {
    // a stop on its way disarms already and owns the recording it saves; a broken app takes no
    // claim, and disarming always stays possible
    let _claim = if recording::broken(state) {
        None
    } else {
        Some(Recording::claim(state)?)
    };
    let (aim, data) = {
        let bench = state.bench.locked();
        (bench.aim(), bench.capture.as_ref().map(|c| c.data.clone()))
    };
    if aim.0.is_some() {
        let disarmed = recording::disarm_within(state, aim);
        // the condition being recorded ended with it, disarming always stays possible
        if let Some(data) = data {
            data.locked()
                .fail_injection("the aim controller was disarmed during the recording");
        }
        disarmed?;
    }
    Ok(())
}
