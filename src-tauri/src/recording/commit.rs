//! a finished recording's metadata, published once
use crate::recording;

use motion_bench::{
    capture::session::Closing,
    evidence::{
        commit::{
            Participant,
            participant::{OBSERVERS, RECORDING_CONTROL},
        },
        reason::{Reason, StopReason},
    },
};
use serde_json::Value;
use std::path::PathBuf;

/// what the other participants recorded goes into the recording's metadata, which is then
/// published once, outside the capture's lock. the observers wrote their own file when they
/// stopped, the metadata keeps a copy; the control journal is finished and saved first, as a
/// participant with its own outcome. a journal that cannot be saved does not fail the usb
/// evidence, its copy says it failed and the recording reads as degraded. returns the commit
/// and any failure to save the journal
pub(crate) fn commit_recording(
    mut closing: Closing,
    stop: StopReason,
    control: Option<Value>,
) -> (
    Result<PathBuf, motion_bench::evidence::reason::Failure>,
    Option<String>,
) {
    let mut participants = Vec::new();
    match std::fs::read(closing.path("observers.json")) {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(observers) => participants.push(Participant {
                id: OBSERVERS,
                // the observers say themselves whether they saw the recording through
                error: observers["error"].as_str().map(str::to_string).or_else(|| {
                    (observers["complete"] == false)
                        .then(|| "the input observers did not complete".to_string())
                }),
                section: observers,
            }),
            Err(error) => closing.fail(
                Reason::ParticipantFailed,
                format!("input observer evidence cannot be read: {error}"),
            ),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => closing.fail(
            Reason::ParticipantFailed,
            format!("input observer evidence cannot be read: {error}"),
        ),
    }
    let mut saved = None;
    if let Some(mut control) = control {
        if let Err(error) = recording::hotkey::finish_control(&mut control, None) {
            // the copy says what happened to the journal it stands for
            control["complete"] = false.into();
            control["error"] = format!("the journal could not be saved: {error}").into();
            saved = Some(error);
        }
        participants.push(Participant {
            id: RECORDING_CONTROL,
            error: control["error"].as_str().map(str::to_string),
            section: control,
        });
    }
    (closing.commit(stop, participants), saved)
}
