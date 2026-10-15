use super::layout::Located;
use crate::evidence::commit::{
    RECORDING_SCHEMA, RECORDING_VERSION,
    participant::{MEDIUS_INPUT, VISION},
};
use crate::evidence::reason::{Reason, StopReason};
use std::path::Path;

/// the newest timing schema this build writes and understands
const TIMING_SCHEMA: u32 = 1;

/// what the sidecar turned out to be. corrupt and unsupported metadata are read errors of their
/// own, they never pass for a recording that simply has none
#[derive(Clone, Debug, PartialEq)]
pub enum Metadata {
    Missing,
    /// unreadable, or not the metadata this version writes
    Corrupt(String),
    /// written by a newer version, so its fields cannot be trusted to mean what they mean here
    Unsupported(String),
    Valid(serde_json::Value),
}

impl Metadata {
    pub fn read(path: &Path) -> Metadata {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Metadata::Missing,
            Err(e) => return Metadata::Corrupt(e.to_string()),
        };
        let value: serde_json::Value = match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(e) => return Metadata::Corrupt(e.to_string()),
        };
        if !value.is_object() {
            return Metadata::Corrupt("the metadata is not a json object".into());
        }
        for (schema, prefix, newest) in [
            (
                &value["timing"]["schema"],
                "motion-bench.timing.v",
                TIMING_SCHEMA,
            ),
            (
                &value["schema"],
                "motion-bench.recording.v",
                RECORDING_VERSION,
            ),
        ] {
            let schema = schema.as_str().unwrap_or("");
            // a version this build cannot even parse, like `v3.0`, is no older than it
            if let Some(version) = schema.strip_prefix(prefix)
                && version.parse::<u32>().ok().is_none_or(|v| v > newest)
            {
                return Metadata::Unsupported(schema.into());
            }
        }
        if value["schema"] != RECORDING_SCHEMA {
            return Metadata::Corrupt("it is not the metadata this version writes".into());
        }
        if !matches!(
            value["outcome"].as_str(),
            Some("complete" | "degraded" | "failed" | "interrupted")
        ) {
            return Metadata::Corrupt(format!(
                "its outcome {} is not one this version knows",
                value["outcome"]
            ));
        }
        Metadata::Valid(value)
    }

    /// the parsed sidecar, only when it can be trusted
    pub fn valid(&self) -> Option<&serde_json::Value> {
        match self {
            Metadata::Valid(value) => Some(value),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Complete,
    /// the usb evidence is whole but something next to it is not
    Degraded(Vec<Degradation>),
    /// `reason` is the stable code when the recording wrote one
    Failed {
        reason: Option<Reason>,
        message: String,
    },
    /// cut short. with the stop the metadata recorded, the files were saved and it can be
    /// compared; with none, only the marker is left, what a crash leaves, and it is view only.
    /// what was already wrong with it stays, so a paired analysis still sees lost pairing
    Interrupted {
        stop: Option<StopReason>,
        degraded: Vec<Degradation>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Degradation {
    /// the box's clock could not be lined up with the wire's, so paired analyses cannot run
    PairingInvalid,
    /// something that ran beside the capture failed, the usb evidence itself is whole
    ParticipantFailed,
    /// the screen recording beside it failed, so its target events cannot be trusted
    TargetsFailed,
}

impl Degradation {
    pub fn describe(self) -> &'static str {
        match self {
            Degradation::PairingInvalid => {
                "the box's clock could not be lined up with the wire's for this recording"
            }
            Degradation::ParticipantFailed => "something recorded beside the usb capture failed",
            Degradation::TargetsFailed => "the screen recording beside it failed",
        }
    }
}

/// what a person needs to see about how a recording ended: one word for the kind, the stable
/// code when the recording wrote one, and the reason in words
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Summary {
    pub kind: &'static str,
    pub reason: Option<Reason>,
    pub message: Option<String>,
}

pub fn summary(status: &Status) -> Summary {
    let (kind, reason, message) = match status {
        Status::Complete => ("complete", None, None),
        Status::Degraded(found) => (
            "degraded",
            None,
            Some(
                found
                    .iter()
                    .map(|d| d.describe())
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        ),
        Status::Failed { reason, message } => ("failed", *reason, Some(message.clone())),
        Status::Interrupted { stop, degraded } => (
            "interrupted",
            None,
            Some(
                std::iter::once(match stop {
                    Some(stop) => stop.describe(),
                    None => {
                        "it never finished, the app or the computer stopped while it was recording"
                    }
                })
                .chain(degraded.iter().map(|d| d.describe()))
                .collect::<Vec<_>>()
                .join(", "),
            ),
        ),
    };
    Summary {
        kind,
        reason,
        message,
    }
}

/// something that ran beside the usb capture and wrote into the sidecar how it went
#[derive(Clone, Debug, PartialEq)]
pub struct Participant {
    pub name: String,
    pub failure: Option<String>,
}

/// the participants a sidecar lists, each with its failure if it had one. one that did not
/// complete failed even when it gives no words for it
pub fn participants(sidecar: Option<&serde_json::Value>) -> Vec<Participant> {
    let Some(listed) = sidecar.and_then(|s| s["participants"].as_array()) else {
        return Vec::new();
    };
    listed
        .iter()
        .map(|p| {
            let name = p["id"].as_str().unwrap_or("participant").to_string();
            let failure = p["error"].as_str().map(str::to_string).or_else(|| {
                (p["outcome"] != "complete").then(|| format!("{name} did not complete"))
            });
            Participant { name, failure }
        })
        .collect()
}

/// the metadata decides, a marker left beside it only lost the race to be removed. without
/// metadata, the marker the writer leaves when publishing failed says how, and the one a
/// recording keeps while it runs says it never finished. `read_error` is what reading the pcap
/// itself ran into
pub fn classify(
    pcap: &Path,
    metadata: &Metadata,
    read_error: Option<(Option<Reason>, String)>,
) -> Status {
    let s = match metadata {
        Metadata::Valid(s) => s,
        Metadata::Corrupt(why) => {
            return Status::Failed {
                reason: None,
                message: format!("its metadata cannot be read ({why})"),
            };
        }
        Metadata::Unsupported(schema) => {
            return Status::Failed {
                reason: None,
                message: format!("its metadata is from a newer version ({schema})"),
            };
        }
        Metadata::Missing => return unpublished(pcap),
    };
    if s["outcome"] == "failed" {
        return Status::Failed {
            reason: serde_json::from_value(s["reason"].clone()).ok(),
            message: s["health"]["error"]
                .as_str()
                .unwrap_or("the recording failed")
                .into(),
        };
    }
    if let Some((reason, message)) = read_error {
        return Status::Failed { reason, message };
    }
    let mut degraded = degradations(s);
    if s["outcome"] == "interrupted" {
        return Status::Interrupted {
            // only a stop that cuts a recording short makes it comparable, any other is treated
            // like a crash leftover
            stop: serde_json::from_value::<StopReason>(s["stop_reason"].clone())
                .ok()
                .filter(|stop| stop.interrupts()),
            degraded,
        };
    }
    // a recording that said it was degraded stays so, even when it did not say what failed
    if degraded.is_empty() && s["outcome"] == "degraded" {
        degraded.push(Degradation::ParticipantFailed);
    }
    if degraded.is_empty() {
        Status::Complete
    } else {
        Status::Degraded(degraded)
    }
}

/// a recording whose metadata never appeared
fn unpublished(pcap: &Path) -> Status {
    match std::fs::read(pcap.with_extension("incomplete.json")) {
        Ok(bytes) => {
            let record: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
            Status::Failed {
                reason: serde_json::from_value(record["reason"].clone()).ok(),
                message: record["error"]
                    .as_str()
                    .unwrap_or("the recording did not finish")
                    .into(),
            }
        }
        // the file is there, only unreadable, and it still says the recording did not finish
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Status::Failed {
            reason: None,
            message: format!("the recording did not finish, its marker cannot be read ({e})"),
        },
        Err(_) if pcap.with_extension("active.json").exists() => Status::Interrupted {
            stop: None,
            degraded: Vec::new(),
        },
        Err(_) => Status::Failed {
            reason: None,
            message: "nothing beside the capture says how it ended".into(),
        },
    }
}

/// what is wrong beside whole usb evidence
fn degradations(s: &serde_json::Value) -> Vec<Degradation> {
    let failed = participants(Some(s))
        .into_iter()
        .filter(|p| p.failure.is_some())
        .collect::<Vec<_>>();
    [
        (s["health"]["pairing_valid"] == false).then_some(Degradation::PairingInvalid),
        // the box's input failing is already said by the pairing
        failed
            .iter()
            .any(|p| p.name != VISION && p.name != MEDIUS_INPUT)
            .then_some(Degradation::ParticipantFailed),
        failed
            .iter()
            .any(|p| p.name == VISION)
            .then_some(Degradation::TargetsFailed),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// the metadata and status from the files beside the pcap alone, for consumers that never read
/// the capture itself. a failure only the pcap shows is left to whoever reads it
pub fn assess(located: &Located) -> (Metadata, Status) {
    let metadata = Metadata::read(&located.sidecar);
    let status = classify(&located.pcap, &metadata, None);
    (metadata, status)
}

#[cfg(test)]
mod tests;
