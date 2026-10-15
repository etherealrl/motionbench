use super::status::{Degradation, Metadata, Status};
use crate::evidence::reason::{Reason, StopReason};
use std::fmt;

/// evidence saved beside the usb capture that an analysis reads
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stream {
    /// the box's own record of the mouse's movement, `.input.csv`
    PhysicalInput,
}
impl Stream {
    pub fn describe(self) -> &'static str {
        match self {
            Stream::PhysicalInput => "it has no physical input, the box didn't record the mouse",
        }
    }
}

/// what an analysis needs from a recording before it will read it
#[derive(Clone, Copy, Debug)]
pub struct Requirements {
    /// only a recording that finished, not one cut short
    pub finished: bool,
    /// degradations that make this analysis meaningless, others it can live with
    pub refuses: &'static [Degradation],
    /// evidence it reads besides the usb capture
    pub streams: &'static [Stream],
}

/// texture and event comparisons read the usb evidence, which pairing does not touch
pub const COMPARISON: Requirements = Requirements {
    finished: false,
    refuses: &[],
    streams: &[],
};

/// paired comparisons line the box's clock up with the wire's
pub const PAIRED: Requirements = Requirements {
    finished: false,
    refuses: &[Degradation::PairingInvalid],
    streams: &[Stream::PhysicalInput],
};

/// target calibration and event reconstruction only work on a recording that finished, with
/// its screen recording whole
pub const TARGETS: Requirements = Requirements {
    finished: true,
    refuses: &[Degradation::TargetsFailed],
    streams: &[],
};

/// why an analysis will not read a recording. it says why the comparison refuses, the recording's
/// own failure code, when there is one, rides along
#[derive(Clone, Debug, PartialEq)]
pub enum Refusal {
    Failed {
        reason: Option<Reason>,
        message: String,
    },
    /// only the marker is left, nothing was saved to say how far it got
    Unfinished,
    /// cut short with its files saved, but this needs a recording that finished
    Interrupted(StopReason),
    CorruptMetadata(String),
    UnsupportedMetadata(String),
    Degraded(Degradation),
    Missing(Stream),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Failed { message, .. } => write!(f, "it failed: {message}"),
            Refusal::Unfinished => f.write_str(
                "it never finished, the app or the computer stopped while it was recording",
            ),
            Refusal::Interrupted(stop) => write!(
                f,
                "{}, and this needs a recording that finished",
                stop.describe()
            ),
            Refusal::CorruptMetadata(why) => {
                write!(f, "its metadata cannot be read ({why})")
            }
            Refusal::UnsupportedMetadata(schema) => {
                write!(f, "its metadata is from a newer version ({schema})")
            }
            Refusal::Degraded(degradation) => f.write_str(degradation.describe()),
            Refusal::Missing(stream) => f.write_str(stream.describe()),
        }
    }
}

/// the one decision every consumer uses. metadata that cannot be read or comes from a newer
/// version is refused before anything else, a missing sidecar with a marker is not the same thing.
/// how the recording ended comes before what was saved beside it, a failed one says it failed
pub fn eligibility(
    metadata: &Metadata,
    status: &Status,
    saved: &[Stream],
    needs: Requirements,
) -> Result<(), Refusal> {
    match metadata {
        Metadata::Corrupt(why) => return Err(Refusal::CorruptMetadata(why.clone())),
        Metadata::Unsupported(schema) => return Err(Refusal::UnsupportedMetadata(schema.clone())),
        Metadata::Missing | Metadata::Valid(_) => {}
    }
    match status {
        Status::Failed { reason, message } => Err(Refusal::Failed {
            reason: *reason,
            message: message.clone(),
        }),
        Status::Interrupted { stop: None, .. } => Err(Refusal::Unfinished),
        Status::Interrupted {
            stop: Some(stop), ..
        } if needs.finished => Err(Refusal::Interrupted(*stop)),
        Status::Degraded(found)
        | Status::Interrupted {
            degraded: found, ..
        } => match found.iter().find(|d| needs.refuses.contains(d)) {
            Some(degradation) => Err(Refusal::Degraded(*degradation)),
            None => Ok(()),
        },
        Status::Complete => Ok(()),
    }?;
    match needs.streams.iter().find(|stream| !saved.contains(stream)) {
        Some(stream) => Err(Refusal::Missing(*stream)),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests;
