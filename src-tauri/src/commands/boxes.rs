//! the box's settings and actions
use crate::app::state::{Shared, allowed};
use motion_bench::sync::LockExt;

use motion_bench::hardware::medius::{Command, EmitPace, RenderMode};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::State;

#[derive(Deserialize, Serialize)]
pub(crate) struct BoxCommand {
    pub(crate) action: String,
    pub(crate) mode: Option<String>,
    pub(crate) full: Option<bool>,
    pub(crate) hz: Option<u16>,
    pub(crate) force: Option<u16>,
    pub(crate) percent: Option<u16>,
    pub(crate) milliseconds: Option<u64>,
    pub(crate) enabled: Option<bool>,
    pub(crate) key: Option<String>,
}
#[tauri::command]
pub(crate) fn box_command(command: BoxCommand, state: State<Shared>) -> Result<(), String> {
    send_box(command, &state)
}
pub(crate) fn send_box(command: BoxCommand, state: &Shared) -> Result<(), String> {
    let request = match command.action.as_str() {
        "connect" => Command::Connect,
        "disconnect" => Command::Disconnect,
        "pick" => Command::Pick(command.key.clone().ok_or("pick needs a box")?),
        "refresh" => Command::Refresh,
        "watch" => Command::Catch(command.enabled.unwrap_or(false)),
        "render" => Command::Render(
            match command.mode.as_deref() {
                Some("off") => RenderMode::Off,
                Some("stock") => RenderMode::Stock,
                Some("despiked") => RenderMode::Despiked,
                Some("unsmoothed") => RenderMode::Unsmoothed,
                _ => return Err("that render mode doesn't exist".into()),
            },
            command.full.unwrap_or(false),
        ),
        "emit" => Command::Emit(
            match command.mode.as_deref() {
                Some("learned") => EmitPace::Learned,
                Some("interval") => EmitPace::Interval,
                Some("fixed") => EmitPace::Fixed(
                    command
                        .hz
                        .filter(|v| *v > 0 && *v <= 8000)
                        .ok_or("fixed emit rate must be positive")?,
                ),
                _ => return Err("that emit mode doesn't exist".into()),
            },
            command.force,
        ),
        "spread" => Command::Spread(
            command
                .percent
                .filter(|v| *v <= 100)
                .ok_or("spread must be between 0 and 100 percent")?,
        ),
        "riding" => Command::Riding(command.milliseconds.map(Duration::from_millis)),
        "imperfect" => Command::Imperfect(command.enabled.unwrap_or(false)),
        _ => return Err("the box doesn't know that action".into()),
    };
    let mut bench = state.bench.locked();
    // it takes no gate, so an action on its way still stops it
    let readiness = bench.readiness(state.recording_hotkey.snapshot().busy);
    allowed(match command.action.as_str() {
        "connect" | "disconnect" | "pick" | "refresh" => readiness.box_connect,
        _ => readiness.box_settings,
    })?;
    if ["render", "emit", "spread", "riding", "imperfect"].contains(&command.action.as_str()) {
        bench.requested[&command.action] = serde_json::to_value(&command).unwrap();
    }
    bench.link.send(request);
    Ok(())
}
