use super::{
    layout::{Located, locate},
    status::{Metadata, Participant, Status, classify, participants},
};
use crate::capture::{
    hid::Decoder,
    pcap::{self, Event, Record},
    session::Data,
};
use crate::signal::stats::Key;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufReader, Read},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

/// a movement series an analysis can pick out of the recording
#[derive(Clone, Debug)]
pub struct Stream {
    /// the stream's name inside this recording, what a result refers to
    pub id: &'static str,
    pub role: Role,
    pub clock: Clock,
    pub artifact: std::path::PathBuf,
    /// the usb endpoint and the decoder its reports went through, for the wire stream only
    pub endpoint: Option<Key>,
    pub decoder: Option<Decoder>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// the selected endpoint as USBPcap saw it on the wire
    SelectedOutput,
    /// the hand's motion as the Medius box saw it, before anything the box does to it
    PhysicalInput,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clock {
    UsbpcapUtc,
    MediusBox,
}

/// a saved recording read once: where its files are, what its metadata says, how it ended and
/// what can be selected from it
pub struct Recording {
    pub located: Located,
    pub metadata: Metadata,
    pub status: Status,
    pub streams: Vec<Stream>,
    pub participants: Vec<Participant>,
    data: Data,
    /// a saved selection that does not name a device here
    pub selection_error: Option<String>,
    /// why the capture file ends early, when it does. what came before the cut is still read
    pub cut_short: Option<String>,
    /// when each device first went away, by bus and address. the reports before it stay
    pub disconnected: BTreeMap<(u16, u16), u64>,
    /// addresses another device took during the recording, whose reports aren't one device's
    pub reused: BTreeSet<(u16, u16)>,
}

impl Recording {
    pub fn data(&self) -> &Data {
        &self.data
    }

    /// the live state, to select or plot from. a failed recording keeps what it captured
    pub fn into_data(self) -> Data {
        self.data
    }

    /// the device to read, by its id, the same way Runs picks it
    pub fn select(&mut self, id: &str) -> Result<(), String> {
        self.data.select(id, None)
    }
}

/// a file read that fails once `cancel` is set, so a long read stops part way
pub(crate) struct Cancellable<'a, R> {
    pub reader: R,
    pub cancel: &'a AtomicBool,
}

impl<R: Read> Read for Cancellable<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("cancelled"));
        }
        self.reader.read(buffer)
    }
}

/// the one way to open a saved recording. it fails only when it is not a recording this app
/// made or the pcap cannot be read at all. a capture cut short keeps what came before the cut,
/// and everything else lands in `status`
pub fn open_recording(path: &Path, override_decoder: Option<Decoder>) -> Result<Recording, String> {
    open_recording_until(path, override_decoder, &AtomicBool::new(false))
}

/// `open_recording` that stops reading once `cancel` is set. the recording it gives then reads as
/// cut short, so the caller checks `cancel` before using it
pub fn open_recording_until(
    path: &Path,
    override_decoder: Option<Decoder>,
    cancel: &AtomicBool,
) -> Result<Recording, String> {
    let located = locate(path)?;
    let open = || -> Result<_, String> {
        let file = std::fs::File::open(&located.pcap).map_err(|e| e.to_string())?;
        Ok(BufReader::new(Cancellable {
            reader: file,
            cancel,
        }))
    };
    let metadata = Metadata::read(&located.sidecar);
    let saved_decoder = metadata
        .valid()
        .and_then(|s| s.get("decoder"))
        .and_then(|d| serde_json::from_value::<Decoder>(d.clone()).ok());
    let decoder = override_decoder.or(saved_decoder);
    let mut data = Data::new(false);
    let mut started = false;
    // descriptors first, so a report that came before its device's descriptors still decodes
    let first = pcap::read_pcap(open()?, |record| {
        started = true;
        if matches!(
            &record,
            Record::Header(_)
                | Record::Packet {
                    event: Some(Event::Device { .. } | Event::Config { .. } | Event::Hid { .. }),
                    ..
                }
        ) {
            data.ingest(&record);
        }
    });
    let mut pcap_error = match first {
        Ok(()) => None,
        Err(error) if started => Some(error.to_string()),
        Err(error) => return Err(error.to_string()),
    };
    data.health.records = 0;
    let mut disconnected = BTreeMap::new();
    let mut identities = BTreeMap::new();
    let mut reused = BTreeSet::new();
    let second = pcap::read_pcap(open()?, |record| {
        match &record {
            Record::Packet {
                header,
                event:
                    Some(Event::Disconnected { bus, address } | Event::Aborted { bus, address, .. }),
                ..
            } => {
                disconnected
                    .entry((*bus, *address))
                    .or_insert(pcap::record_time(header));
            }
            Record::Packet {
                event:
                    Some(Event::Device {
                        bus,
                        address,
                        vid,
                        pid,
                    }),
                ..
            } if identities
                .insert((*bus, *address), (*vid, *pid))
                .is_some_and(|old| old != (*vid, *pid)) =>
            {
                reused.insert((*bus, *address));
            }
            _ => {}
        }
        if let (
            Some(decoder),
            Record::Packet {
                event: Some(Event::Report(report)),
                ..
            },
        ) = (&decoder, &record)
        {
            data.decoders.insert(
                Key {
                    bus: report.bus,
                    device: report.device,
                    endpoint: report.endpoint,
                },
                decoder.clone(),
            );
        }
        // the live session forgets a device that went away, a saved recording keeps what it sent
        // before, that is the evidence of how it failed
        if !matches!(
            record,
            Record::Packet {
                event: Some(Event::Disconnected { .. } | Event::Aborted { .. }),
                ..
            }
        ) {
            data.ingest(&record);
        }
    });
    if let Err(error) = second {
        pcap_error.get_or_insert(error.to_string());
    }
    // the chosen layout wins over one the capture saw after the last report too
    if let Some(decoder) = &decoder {
        for key in data.bus.streams.keys() {
            data.decoders.insert(*key, decoder.clone());
        }
    }
    let cut_short = pcap_error.clone();
    let selection_error = select(&mut data, metadata.valid());
    let read_error = match &pcap_error {
        Some(error) => Some((None, format!("the capture file is cut short ({error})"))),
        None => data.read_error(),
    };
    let status = classify(&located.pcap, &metadata, read_error);
    let mut streams = Vec::new();
    if let Some(key) = data.selected {
        streams.push(Stream {
            id: "usb",
            role: Role::SelectedOutput,
            clock: Clock::UsbpcapUtc,
            artifact: located.pcap.clone(),
            endpoint: Some(key),
            decoder: data.decoders.get(&key).cloned(),
        });
    }
    if located.input.is_file() {
        streams.push(Stream {
            id: "medius_input",
            role: Role::PhysicalInput,
            clock: Clock::MediusBox,
            artifact: located.input.clone(),
            endpoint: None,
            decoder: None,
        });
    }
    let participants = participants(metadata.valid());
    Ok(Recording {
        located,
        metadata,
        status,
        streams,
        participants,
        data,
        selection_error,
        cut_short,
        disconnected,
        reused,
    })
}

/// the single device when there is only one, then the saved selection. the first one that does
/// not work stops it
fn select(data: &mut Data, sidecar: Option<&serde_json::Value>) -> Option<String> {
    let devices = data.devices();
    if devices.len() == 1
        && let Err(error) = data.select(&devices[0].id, None)
    {
        return Some(error);
    }
    let selection = sidecar.and_then(|s| s.get("selection"))?;
    let id = format!(
        "{}:{}:{}",
        selection["bus"], selection["address"], selection["endpoint"]
    );
    data.select(&id, None).err()
}

#[cfg(test)]
mod tests;
