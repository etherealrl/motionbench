//! what the recording's status line and cue say after a drive, and the order they are said in

use super::Driven;
use crate::{app::state::Shared, recording::hotkey};
use motion_bench::sync::LockExt;
use motion_bench::{coordination::recording::Attempt, evidence::reason::StopReason};

/// what a recording's status line and cue say after a drive
struct Words {
    message: Option<String>,
    error: Option<String>,
    /// started, saved, or failed
    cue: Option<bool>,
}

impl Words {
    fn failed(error: &str) -> Self {
        Words {
            message: Some("Recording request failed".into()),
            error: Some(error.into()),
            cue: None,
        }
    }
}

/// a request refused before anything ran, said as such
pub(super) fn refused(state: &Shared, error: String) -> String {
    say(state, None, false, Words::failed(&error));
    error
}

/// the words of `attempt`, unless a later recording has spoken already; a start's words only
/// while that recording is still the one on its way
fn say(state: &Shared, attempt: Option<Attempt>, started: bool, words: Words) {
    let mut said = state.recording.said.locked();
    if let Some(attempt) = attempt {
        if said.is_some_and(|said| said > attempt)
            || started && state.recording.attempt() != Some(attempt)
        {
            return;
        }
        *said = Some(attempt);
    }
    let volume = state.recording_hotkey.settings().get().display.cue_volume();
    hotkey::cue(words.cue, volume);
    state.recording_hotkey.update(|s| {
        s.message = words.message;
        s.error = words.error;
    });
}

/// says what a drive did and returns what its command returns: the stem of a recording that
/// started, the path of one that was saved
pub(crate) fn announce(state: &Shared, driven: Driven) -> Result<Option<String>, String> {
    let attempt = driven.attempt;
    // only a start that made a recording speaks for one still on its way
    let started = driven.stop.is_none() && driven.stem.is_some();
    let (result, words) = outcome(driven);
    if let Some(words) = words {
        say(state, attempt, started, words);
    }
    result
}

fn outcome(driven: Driven) -> (Result<Option<String>, String>, Option<Words>) {
    let warnings = |mut warnings: Vec<String>, more: Option<String>| {
        warnings.extend(more);
        (!warnings.is_empty()).then(|| warnings.join("; "))
    };
    let Some(stop) = driven.stop else {
        // stopping capture with no recording says nothing
        if driven.attempt.is_none() {
            return (Ok(None), None);
        }
        let Some(stem) = driven.stem else {
            let error = driven
                .begin_error
                .or_else(|| driven.warnings.into_iter().next())
                .unwrap_or_else(|| "the recording could not be started".into());
            let words = Words::failed(&error);
            return (Err(error), Some(words));
        };
        let words = Words {
            message: Some(format!("Recording {stem}")),
            error: warnings(driven.warnings, None),
            cue: Some(true),
        };
        return (Ok(Some(stem)), Some(words));
    };
    let committed = driven
        .committed
        .unwrap_or_else(|| Err("the recording was not saved".into()))
        .map(|path| path.display().to_string());
    let listing = match &driven.listed {
        Some(Err(error)) => Some(error.clone()),
        _ => None,
    };
    let error = match stop {
        // nothing is said while the app closes
        StopReason::AppExit => return (committed.map(Some), None),
        StopReason::StartFailed => driven
            .begin_error
            .unwrap_or_else(|| "the recording could not be started whole".into()),
        StopReason::RecordingFailed => {
            let message = match (&committed, &listing) {
                (Err(error), None) => {
                    format!("the recording failed and was stopped ({error}), it is in Runs")
                }
                (Err(error), _) | (_, Some(error)) => {
                    format!("the recording failed and was stopped: {error}")
                }
                _ => "the recording failed and was stopped".to_string(),
            };
            let words = Words {
                message: None,
                error: Some(message.clone()),
                cue: None,
            };
            return (Err(message), Some(words));
        }
        _ => match (committed, listing) {
            (Err(error), Some(listing)) => {
                format!("{error}; it could not be added to Runs: {listing}")
            }
            (Err(error), None) => error,
            (Ok(path), Some(listing)) => {
                format!("capture saved at {path}, but it could not be added to Runs: {listing}")
            }
            (Ok(path), None) => {
                let (saved, export) = match driven.events {
                    Some(Ok(0)) => ("saved to Runs, no target events".to_string(), None),
                    Some(Ok(count)) => (format!("saved to Runs, {count} target events"), None),
                    Some(Err(error)) => (
                        "saved to Runs".to_string(),
                        Some(format!("target event export: {error}")),
                    ),
                    None => ("saved to Runs".to_string(), None),
                };
                let words = Words {
                    message: Some(if stop == StopReason::CaptureStop {
                        saved
                    } else {
                        format!("{saved}, USB monitoring keeps running")
                    }),
                    error: warnings(driven.warnings, export),
                    cue: Some(false),
                };
                return (Ok(Some(path)), Some(words));
            }
        },
    };
    let words = Words::failed(&error);
    (Err(error), Some(words))
}
