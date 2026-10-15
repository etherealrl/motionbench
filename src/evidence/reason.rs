use serde::{Deserialize, Serialize};

/// why capture or a recording failed, written next to the readable message so a reader can tell
/// failures apart without parsing the wording. the codes are spelled out one by one, a renamed
/// variant must not change what lands on disk
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reason {
    #[serde(rename = "writer_failed")]
    WriterFailed,
    #[serde(rename = "descriptor_cache_full")]
    DescriptorCacheFull,
    #[serde(rename = "device_lost")]
    DeviceLost,
    #[serde(rename = "timestamp_regressed")]
    TimestampRegressed,
    #[serde(rename = "device_reenumerated")]
    DeviceReenumerated,
    #[serde(rename = "address_reused")]
    AddressReused,
    #[serde(rename = "metadata_failed")]
    MetadataFailed,
    #[serde(rename = "transport_failed")]
    TransportFailed,
    #[serde(rename = "transport_ended")]
    TransportEnded,
    #[serde(rename = "driver_reported")]
    DriverReported,
    #[serde(rename = "participant_failed")]
    ParticipantFailed,
    #[serde(rename = "clock_failed")]
    ClockFailed,
    /// a code written by a newer version
    #[serde(other, rename = "unknown")]
    Unknown,
}

/// what a failure belongs to, which decides what it stops. the pipeline is usb capture itself,
/// a source is one device in it, a recording is one recording's files, and a participant is
/// something that ran beside the usb capture
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// capture cannot go on, nothing records until it restarts
    Pipeline,
    /// the selected device can no longer be trusted until it is selected or checked again
    Source,
    /// this recording failed, the next one starts clean
    Recording,
    /// something beside the usb capture failed during this recording
    Participant,
}

impl Reason {
    // spelled out without a wildcard, so a new code has to say what it stops
    pub fn scope(self) -> Scope {
        match self {
            Reason::TransportFailed
            | Reason::TransportEnded
            | Reason::DriverReported
            | Reason::DescriptorCacheFull => Scope::Pipeline,
            // a code from a newer version stops the most
            Reason::Unknown => Scope::Pipeline,
            Reason::DeviceLost
            | Reason::TimestampRegressed
            | Reason::DeviceReenumerated
            | Reason::AddressReused => Scope::Source,
            Reason::WriterFailed | Reason::MetadataFailed | Reason::ClockFailed => Scope::Recording,
            Reason::ParticipantFailed => Scope::Participant,
        }
    }
}

/// why a recording stopped, apart from whether it failed. a stop that ended capture or the app
/// interrupts the recording, even when every file was saved. the codes are spelled out one by
/// one like `Reason`'s
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    #[serde(rename = "record_stop")]
    RecordStop,
    #[serde(rename = "hotkey_stop")]
    HotkeyStop,
    /// the recording was ended because it could not be started whole
    #[serde(rename = "start_failed")]
    StartFailed,
    #[serde(rename = "capture_stop")]
    CaptureStop,
    #[serde(rename = "app_exit")]
    AppExit,
    /// something failed while it was recording, and a failed recording is ended at once
    #[serde(rename = "recording_failed")]
    RecordingFailed,
    /// a code written by a newer version
    #[serde(other, rename = "unknown")]
    Unknown,
}

impl StopReason {
    /// whether this stop cut the recording short rather than ending it as asked
    pub fn interrupts(self) -> bool {
        matches!(self, StopReason::CaptureStop | StopReason::AppExit)
    }

    pub fn describe(self) -> &'static str {
        match self {
            StopReason::RecordStop => "it was stopped",
            StopReason::HotkeyStop => "it was stopped with the recording button",
            StopReason::StartFailed => "it could not be started whole",
            StopReason::CaptureStop => "usb capture was stopped while it was recording",
            StopReason::AppExit => "the app closed while it was recording",
            StopReason::RecordingFailed => "it failed while it was recording",
            StopReason::Unknown => "it stopped for a reason this version does not know",
        }
    }
}

/// the first thing that went wrong, as a code and as words
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub reason: Reason,
    pub message: String,
}

impl Failure {
    pub fn new(reason: Reason, message: impl ToString) -> Self {
        Failure {
            reason,
            message: message.to_string(),
        }
    }
}

#[cfg(test)]
mod tests;
