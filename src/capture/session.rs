use crate::sync::LockExt;
use crate::{
    capture::{
        hid::{Decoder, Layout},
        pcap::{self, Endpoint, Event, Record},
    },
    evidence::{
        commit::{
            Participant, RECORDING_SCHEMA,
            participant::{INJECTION, MEDIUS_INPUT},
        },
        reason::{Failure, Reason, Scope, StopReason},
        recorder::{self, Recorder, Tee},
    },
    signal::stats::{Bus, Key},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::BufReader,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread::JoinHandle,
};

pub fn wall_us() -> u64 {
    crate::capture::clock::utc_us()
}

/// reports the selected device sends, after it is selected or something made it suspect,
/// before a recording of it can start. no report period is assumed
pub const VALIDATION_REPORTS: u32 = 32;

/// live, `error` is usb capture's own failure, which stops everything until capture restarts,
/// and `notice` the latest problem with the selected device or a recording, which does not.
/// in a recording's metadata, `error` is that recording's failure and `pairing_valid` its pairing
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Health {
    pub records: u64,
    pub reports: u64,
    pub timestamp_regressions: u64,
    pub serial_dropped: u64,
    pub pairing_valid: bool,
    pub ended: bool,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<Reason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
}

/// how far behind the wall clock a device's reports arrive. the live view trails the clock by
/// the longest delay of the last two seconds, so it moves at the clock's pace instead of jumping
/// with every batch the driver hands over
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Delivery {
    pub delay_us: i64,
    pub since_us: u64,
}

impl Delivery {
    // ponytail: holds the largest delay for two seconds, then takes the current one, so the view
    // can step by the jitter at most every two seconds; a sliding maximum if that ever shows
    pub fn observe(&mut self, delay_us: i64, now_us: u64) {
        if delay_us >= self.delay_us || now_us.saturating_sub(self.since_us) > 2_000_000 {
            *self = Delivery {
                delay_us,
                since_us: now_us,
            };
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub bus: u16,
    pub address: u16,
    pub endpoint: u8,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
    pub kind: String,
    pub interval: Option<u8>,
    pub decoder: String,
    pub reports: u64,
    /// what Windows calls the device behind this endpoint, when it said
    pub name: Option<String>,
    /// the connected box that clones this endpoint, by name, when this endpoint is its clone
    pub box_name: Option<String>,
}

/// `bus:address:endpoint`, as [`Data::devices`] writes an id
fn parse_id(id: &str) -> Option<(u16, u16, u8)> {
    let mut parts = id.split(':');
    let bus = parts.next()?.parse().ok()?;
    let address = parts.next()?.parse().ok()?;
    let endpoint = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((bus, address, endpoint))
}

pub struct Data {
    arrivals: crate::signal::alignment::Arrivals,
    pub bus: Bus,
    pub names: BTreeMap<(u16, u16), (u16, u16)>,
    pub endpoints: BTreeMap<Key, Endpoint>,
    pub decoders: BTreeMap<Key, Decoder>,
    captured_decoders: BTreeMap<Key, Decoder>,
    overrides: BTreeMap<Key, Decoder>,
    /// each endpoint's buttons: whether one is down, and when one last went down
    presses: BTreeMap<Key, (bool, u64)>,
    pub selected: Option<Key>,
    pub health: Health,
    pub recorder: Option<Recorder>,
    pub tee: Tee,
    pub box_offset: Option<i64>,
    pub input_last: Option<u64>,
    pub delivery: BTreeMap<Key, Delivery>,
    /// reports of the selected device that passed the checks since it was selected or last
    /// failed one
    validated: u32,
    /// the notice is about the selected device, and goes once it passes its checks again
    source_notice: bool,
    /// what took part in the running recording beside the usb capture, with its first error
    participants: BTreeMap<&'static str, Option<String>>,
    /// the box's clock went back during this recording, its later input is on another clock
    input_reset: bool,
    /// the counters when the running recording started, so its metadata counts only its own
    at_install: Health,
}

/// a recording whose input has ended, taken out of the live state so its files can be closed
/// and its metadata published without holding the lock
pub struct Closing {
    recorder: Recorder,
    health: Health,
    decoder: Decoder,
    shift: Option<i64>,
    receipt_alignment: Option<crate::signal::alignment::Estimate>,
    participants: BTreeMap<&'static str, Option<String>>,
}

impl Closing {
    pub fn path(&self, kind: &str) -> PathBuf {
        self.recorder.path(kind)
    }

    /// a failure once input had ended, so the metadata still says so
    pub fn fail(&mut self, reason: Reason, message: impl ToString) {
        self.recorder
            .trouble
            .get_or_insert(Failure::new(reason, message.to_string()));
    }

    /// closes the files and publishes the metadata, once. `participants` are what other parts of
    /// the app hand in, such as the observers, and the
    /// ones that took part during the recording are added with their errors. the outcome is
    /// failed when anything failed, else interrupted when the stop cut it short, else degraded
    /// when a participant failed, else complete
    pub fn commit(
        self,
        stop: StopReason,
        mut participants: Vec<Participant>,
    ) -> Result<PathBuf, Failure> {
        let mut closing = self;
        // a recording ended because it could not start whole, or because it failed, never
        // finished, whatever else recorded a failure first
        match stop {
            StopReason::StartFailed => closing.fail(
                Reason::ParticipantFailed,
                "the recording could not be started whole",
            ),
            StopReason::RecordingFailed => closing.fail(
                Reason::ParticipantFailed,
                "the recording failed while it was running",
            ),
            _ => {}
        }
        let Closing {
            recorder,
            mut health,
            decoder,
            shift,
            receipt_alignment,
            participants: mut took_part,
        } = closing;
        for participant in &mut participants {
            if let Some(error) = took_part.remove(participant.id).flatten() {
                participant.error.get_or_insert(error);
            }
        }
        participants.extend(took_part.into_iter().map(|(id, error)| Participant {
            id,
            error,
            section: serde_json::Value::Null,
        }));
        let closed = recorder.close(shift);
        // this recording's own health, whatever capture went through before it
        health.error = closed.failure.as_ref().map(|f| f.message.clone());
        health.reason = closed.failure.as_ref().map(|f| f.reason);
        health.notice = None;
        if closed.failure.is_some() {
            health.pairing_valid = false;
        }
        let mut manifest = closed.manifest.clone();
        manifest["health"] = serde_json::to_value(health).unwrap();
        manifest["decoder"] = serde_json::to_value(decoder).unwrap();
        manifest["paired"]["receipt_alignment"] = serde_json::to_value(receipt_alignment).unwrap();
        manifest["paired"]["absolute_transport_delay_known"] = false.into();
        let outcome = if closed.failure.is_some() {
            "failed"
        } else if stop.interrupts() {
            "interrupted"
        } else if participants.iter().any(|p| p.error.is_some()) {
            "degraded"
        } else {
            "complete"
        };
        manifest["schema"] = RECORDING_SCHEMA.into();
        manifest["outcome"] = outcome.into();
        manifest["reason"] =
            serde_json::to_value(closed.failure.as_ref().map(|f| f.reason)).unwrap();
        manifest["stop_reason"] = serde_json::to_value(stop).unwrap();
        manifest["participants"] = participants
            .iter()
            .map(|p| {
                serde_json::json!({
                    "id": p.id,
                    "outcome": if p.error.is_some() { "failed" } else { "complete" },
                    "error": p.error,
                })
            })
            .collect();
        for participant in participants {
            if !participant.section.is_null() {
                manifest[participant.id] = participant.section;
            }
        }
        closed.publish(&manifest)
    }
}

impl Data {
    pub fn new(live: bool) -> Self {
        Self {
            arrivals: crate::signal::alignment::Arrivals::default(),
            bus: if live { Bus::live() } else { Bus::whole() },
            names: BTreeMap::new(),
            endpoints: BTreeMap::new(),
            decoders: BTreeMap::new(),
            captured_decoders: BTreeMap::new(),
            presses: BTreeMap::new(),
            overrides: BTreeMap::new(),
            selected: None,
            health: Health {
                pairing_valid: true,
                ..Health::default()
            },
            recorder: None,
            tee: Tee::default(),
            box_offset: None,
            input_last: None,
            delivery: BTreeMap::new(),
            validated: 0,
            source_notice: false,
            participants: BTreeMap::new(),
            input_reset: false,
            at_install: Health::default(),
        }
    }

    /// a record and its writes in one go, for a caller that holds the state alone
    pub fn ingest(&mut self, record: &Record<'_>) {
        self.take_in(record);
        if let Some(writes) = self.writes() {
            writes.after(());
        }
    }

    /// what the recording has to write, written by `Writes::after` once the lock is let go
    pub fn writes(&mut self) -> Option<crate::evidence::recorder::Writes> {
        self.recorder.as_mut()?.writes()
    }

    /// takes a record in. what it adds to the recording's files waits in memory, see `writes`
    /// the one endpoint whose button went down within `within_us` of `at_us`, none when no
    /// endpoint or more than one did
    pub fn pressed_at(&self, at_us: u64, within_us: u64) -> Option<Key> {
        let mut near = self
            .presses
            .iter()
            .filter(|(_, (_, t))| *t != 0 && t.abs_diff(at_us) <= within_us)
            .map(|(key, _)| *key);
        let first = near.next()?;
        near.next().is_none().then_some(first)
    }

    pub fn take_in(&mut self, record: &Record<'_>) {
        if let Some(recorder) = &mut self.recorder {
            recorder.observe_clock(false);
        }
        self.tee.offer(self.recorder.as_mut(), record);
        if let Some(failure) = self.tee.error.clone()
            && self.health.error.is_none()
        {
            self.fail(failure.reason, failure.message);
        }
        if self.health.notice.is_none()
            && let Some(failure) = self.recorder.as_ref().and_then(Recorder::failure)
        {
            let notice = format!("the recording failed: {}", failure.message);
            self.notify(notice, false);
        }
        if let Record::Packet { event, .. } = record {
            self.health.records += 1;
            match event {
                Some(Event::Disconnected { bus, address }) => {
                    if self
                        .selected
                        .is_some_and(|k| k.bus == *bus && k.device == *address)
                    {
                        self.selected = None;
                        self.fail(
                            Reason::DeviceLost,
                            "the selected device was unplugged, select it again",
                        );
                    }
                    self.bus
                        .streams
                        .retain(|k, _| k.bus != *bus || k.device != *address);
                    self.delivery
                        .retain(|k, _| k.bus != *bus || k.device != *address);
                    self.decoders
                        .retain(|k, _| k.bus != *bus || k.device != *address);
                    self.captured_decoders
                        .retain(|k, _| k.bus != *bus || k.device != *address);
                    self.presses
                        .retain(|k, _| k.bus != *bus || k.device != *address);
                    self.overrides
                        .retain(|k, _| k.bus != *bus || k.device != *address);
                    self.endpoints
                        .retain(|k, _| k.bus != *bus || k.device != *address);
                }
                Some(Event::Aborted {
                    bus,
                    address,
                    endpoint,
                }) => {
                    let key = Key {
                        bus: *bus,
                        device: *address,
                        endpoint: *endpoint,
                    };
                    if self.selected == Some(key) {
                        self.selected = None;
                        self.fail(
                            Reason::DeviceLost,
                            "the selected device was unplugged, select it again",
                        );
                    }
                    // the endpoint is listed again if it sends again, its descriptors stay
                    self.bus.streams.remove(&key);
                    self.delivery.remove(&key);
                }
                Some(Event::Report(report)) => {
                    let key = Key {
                        bus: report.bus,
                        device: report.device,
                        endpoint: report.endpoint,
                    };
                    let decoder = self.decoders.entry(key).or_default();
                    let motion = decoder.decode(report.bytes());
                    // a layout Windows gave for an endpoint nobody picked still shows its presses
                    let down = decoder.left_button(report.bytes()).or_else(|| {
                        self.captured_decoders
                            .get(&key)?
                            .left_button(report.bytes())
                    });
                    if let Some(down) = down {
                        let press = self.presses.entry(key).or_insert((false, 0));
                        if down && !press.0 {
                            press.1 = report.t_us;
                        }
                        press.0 = down;
                    }
                    let layout = !matches!(decoder, Decoder::Unavailable);
                    let retain = if self.bus.retain_live() {
                        crate::signal::stats::HISTORY_US
                    } else {
                        u64::MAX
                    };
                    let stream = self
                        .bus
                        .streams
                        .entry(key)
                        .or_insert_with(|| crate::signal::stats::Stream::new(retain));
                    // another device's clock going back says nothing about the selected one
                    if stream.last_us().is_some_and(|last| report.t_us < last) {
                        stream.regressions += 1;
                        if self.selected == Some(key) {
                            self.health.timestamp_regressions += 1;
                            self.fail(
                                Reason::TimestampRegressed,
                                "the selected device's USBPcap timestamp went back",
                            );
                        }
                        return;
                    }
                    stream.payload_len = report.len;
                    stream.push_decoded(report.t_us, motion);
                    self.health.reports += 1;
                    // a report its layout cannot decode is skipped, not held against the device,
                    // which can send other reports on the same endpoint
                    if self.selected == Some(key) && (motion.is_some() || !layout) {
                        self.validated = self.validated.saturating_add(1);
                        if self.validated == VALIDATION_REPORTS && self.source_notice {
                            self.health.notice = None;
                            self.source_notice = false;
                        }
                    }
                    let now = wall_us();
                    let delay = now as i64 - report.t_us as i64;
                    self.delivery
                        .entry(key)
                        .and_modify(|d| d.observe(delay, now))
                        .or_insert(Delivery {
                            delay_us: delay,
                            since_us: now,
                        });
                }
                Some(Event::Device {
                    bus,
                    address,
                    vid,
                    pid,
                }) => {
                    let identity = (*vid, *pid);
                    if self.bus.retain_live()
                        && self.names.contains_key(&(*bus, *address))
                        && self
                            .selected
                            .is_some_and(|key| key.bus == *bus && key.device == *address)
                    {
                        self.selected = None;
                        self.fail(
                            Reason::DeviceReenumerated,
                            "the selected device was enumerated again, select it again",
                        );
                    }
                    if self
                        .names
                        .get(&(*bus, *address))
                        .is_some_and(|old| *old != identity)
                    {
                        if self
                            .selected
                            .is_some_and(|key| key.bus == *bus && key.device == *address)
                        {
                            self.selected = None;
                            self.fail(
                                Reason::AddressReused,
                                "selected USB address was reused, select the device again",
                            );
                        }
                        self.bus
                            .streams
                            .retain(|key, _| key.bus != *bus || key.device != *address);
                        self.delivery
                            .retain(|key, _| key.bus != *bus || key.device != *address);
                        self.decoders
                            .retain(|key, _| key.bus != *bus || key.device != *address);
                    }
                    self.names.insert((*bus, *address), identity);
                }
                Some(Event::Config {
                    bus,
                    address,
                    endpoints,
                }) => {
                    for endpoint in endpoints {
                        self.endpoints.insert(
                            Key {
                                bus: *bus,
                                device: *address,
                                endpoint: endpoint.address,
                            },
                            *endpoint,
                        );
                    }
                }
                Some(Event::Hid {
                    bus,
                    address,
                    interface,
                    descriptor,
                }) => {
                    if let Ok(layout) = Layout::parse(descriptor) {
                        for (key, endpoint) in &self.endpoints {
                            if key.bus == *bus
                                && key.device == *address
                                && endpoint.interface == *interface
                            {
                                let decoder = Decoder::Descriptor(layout.clone());
                                self.captured_decoders.insert(*key, decoder.clone());
                                if !self.overrides.contains_key(key) {
                                    self.decoders.insert(*key, decoder);
                                }
                            }
                        }
                    }
                }
                None => {}
            }
        }
    }

    /// fails what the reason belongs to and the running recording with it. only a failure of
    /// usb capture itself outlives the recording; a source failure is only ever reported for the
    /// selected device, which then has to pass its checks again. the first failure is the one
    /// kept, its code and its words together
    pub fn fail(&mut self, reason: Reason, message: impl ToString) {
        let message = message.to_string();
        match reason.scope() {
            Scope::Pipeline => {
                if self.health.error.is_none() {
                    self.health.reason = Some(reason);
                }
                self.health.error.get_or_insert(message.clone());
            }
            Scope::Source => {
                self.validated = 0;
                self.notify(message.clone(), true);
            }
            Scope::Recording | Scope::Participant => {
                self.notify(format!("the recording failed: {message}"), false);
            }
        }
        if let Some(recorder) = &mut self.recorder {
            recorder
                .trouble
                .get_or_insert(Failure::new(reason, message));
        }
    }

    fn notify(&mut self, notice: String, source: bool) {
        self.health.notice = Some(notice);
        self.source_notice = source;
    }

    /// marks an optional participant as part of the running recording
    pub fn take_part(&mut self, id: &'static str) {
        if self.recorder.is_some() {
            self.participants.entry(id).or_default();
        }
    }

    pub fn took_part(&self, id: &str) -> bool {
        self.participants.contains_key(id)
    }

    /// an optional participant failed during the running recording. the usb evidence stays
    /// whole and the recording reads as degraded
    pub fn degrade(&mut self, id: &'static str, message: impl ToString) {
        if self.recorder.is_none() {
            return;
        }
        let message = message.to_string();
        if self.health.notice.is_none() {
            self.notify(
                format!("{message}; the recording keeps its usb evidence"),
                false,
            );
        }
        self.participants
            .entry(id)
            .or_default()
            .get_or_insert(message);
    }

    /// the box's input of the running recording can no longer be lined up with the wire. only
    /// a recording the box's input took part in has a pairing to lose
    pub fn degrade_pairing(&mut self, message: impl ToString) {
        if self.took_part(MEDIUS_INPUT) {
            self.health.pairing_valid = false;
            self.degrade(MEDIUS_INPUT, message);
        }
    }

    /// the injection drives the condition being recorded, so a recording it took part in
    /// cannot stand without it
    pub fn fail_injection(&mut self, message: impl ToString) {
        if self.took_part(INJECTION) {
            let message = message.to_string();
            self.degrade(INJECTION, &message);
            self.fail(Reason::ParticipantFailed, message);
        }
    }

    /// a recording that has failed gains nothing from going on, it is ended at once
    pub fn recording_failed(&self) -> bool {
        self.trouble().is_some()
    }

    /// why the running recording failed
    pub fn trouble(&self) -> Option<&Failure> {
        self.recorder.as_ref()?.failure()
    }

    /// why usb capture itself ended, once it has; it is always capture's own failure
    pub fn capture_failure(&self) -> Option<Failure> {
        let error = self.health.error.clone().filter(|_| self.health.ended)?;
        // capture ends through `fail`, which always gives a pipeline code
        Some(Failure::new(
            self.health.reason.unwrap_or(Reason::TransportEnded),
            error,
        ))
    }

    /// whether a recording of the selected device can start, or the next thing to do first
    pub fn readiness(&self) -> Result<Key, &'static str> {
        if self.recorder.is_some() {
            return Err("already recording");
        }
        if self.capture_failed() {
            return Err("USB capture failed; restart capture before recording");
        }
        let key = self
            .selected
            .ok_or("select the mouse endpoint before recording")?;
        if self.validated < VALIDATION_REPORTS {
            return Err(
                "move the selected mouse first, its reports are checked before recording starts",
            );
        }
        Ok(key)
    }

    /// whether the selected device's first reports have been checked, none without a selection
    pub fn checked(&self) -> Option<bool> {
        self.selected.map(|_| self.validated >= VALIDATION_REPORTS)
    }

    /// what USBPcap said after capture already ended, added to its failure. a capture stopped on
    /// purpose has none and stays without one
    pub fn add_driver_words(&mut self, words: &str) {
        let words: String = words.trim().chars().take(2000).collect();
        let stream = matches!(
            self.health.reason,
            Some(Reason::TransportEnded | Reason::TransportFailed)
        );
        if let Some(error) = &mut self.health.error
            && stream
            && !words.is_empty()
        {
            error.push_str(&format!(", and USBPcap said: {words}"));
        }
    }

    /// whether usb capture itself ended or failed, so nothing can record until it restarts
    pub fn capture_failed(&self) -> bool {
        self.health.ended || self.health.error.is_some()
    }

    /// takes the selection as checked, for a replay of traffic that was checked when it was
    /// captured. the app never calls it
    #[doc(hidden)]
    pub fn trust_selection(&mut self) {
        self.validated = VALIDATION_REPORTS;
    }

    /// what makes a saved capture unreadable: usb capture's own failure, or the selected
    /// device's clock going back
    pub fn read_error(&self) -> Option<(Option<Reason>, String)> {
        if let Some(error) = &self.health.error {
            return Some((self.health.reason, error.clone()));
        }
        let key = self.selected?;
        (self.bus.streams.get(&key)?.regressions > 0).then(|| {
            (
                Some(Reason::TimestampRegressed),
                "USBPcap timestamp regressed".to_string(),
            )
        })
    }

    pub fn devices(&self) -> Vec<Device> {
        self.bus
            .streams
            .iter()
            .map(|(key, stream)| {
                let identity = self.names.get(&(key.bus, key.device));
                let endpoint = self.endpoints.get(key);
                Device {
                    id: format!("{}:{}:{}", key.bus, key.device, key.endpoint),
                    bus: key.bus,
                    address: key.device,
                    endpoint: key.endpoint,
                    vid: identity.map(|i| i.0),
                    pid: identity.map(|i| i.1),
                    kind: endpoint.map_or("unknown", Endpoint::kind).into(),
                    interval: endpoint.map(|e| e.interval),
                    decoder: self
                        .decoders
                        .get(key)
                        .unwrap_or(&Decoder::Unavailable)
                        .name()
                        .into(),
                    reports: stream.total,
                    // the desktop fills these in: neither is something the capture can know
                    name: None,
                    box_name: None,
                }
            })
            .collect()
    }

    pub fn remember_windows_layout(&mut self, key: Key, layout: Layout) {
        self.captured_decoders
            .entry(key)
            .or_insert(Decoder::WindowsDescriptor(layout));
    }

    /// the device a pick means. the id names one row of [`devices`](Self::devices), and when that
    /// row is gone because the device was enumerated again since the list was drawn, the identity
    /// the id was drawn from still says which one was meant. two rows that could both be it are
    /// none: a pick that cannot tell them apart must not guess
    pub fn device(&self, id: &str) -> Option<Device> {
        let devices = self.devices();
        if let Some(exact) = devices.iter().find(|d| d.id == id) {
            return Some(exact.clone());
        }
        let (bus, address, endpoint) = parse_id(id)?;
        let (vid, pid) = *self.names.get(&(bus, address))?;
        let mut same = devices.iter().filter(|d| {
            d.bus == bus && d.endpoint == endpoint && d.vid == Some(vid) && d.pid == Some(pid)
        });
        let found = same.next()?;
        same.next().is_none().then(|| found.clone())
    }

    pub fn select(&mut self, id: &str, decoder: Option<Decoder>) -> Result<(), String> {
        if self.recorder.is_some() {
            return Err("stop recording before changing the device or decoder".into());
        }
        let device = self.device(id).ok_or("device is no longer available")?;
        let key = Key {
            bus: device.bus,
            device: device.address,
            endpoint: device.endpoint,
        };
        let automatic = decoder.is_none();
        if let Some(decoder) = decoder {
            self.overrides.insert(key, decoder.clone());
            self.decoders.insert(key, decoder);
            if self.bus.retain_live() {
                self.bus
                    .streams
                    .insert(key, crate::signal::stats::Stream::default());
            }
        }
        if self.bus.retain_live() && automatic {
            self.overrides.remove(&key);
            self.decoders.insert(
                key,
                self.captured_decoders
                    .get(&key)
                    .cloned()
                    .unwrap_or_default(),
            );
            self.bus
                .streams
                .insert(key, crate::signal::stats::Stream::default());
        }
        self.selected = Some(key);
        self.validated = 0;
        self.health.notice = None;
        self.source_notice = false;
        Ok(())
    }

    /// what a recording of the selected device needs from the live state, checked under the
    /// lock. its files are made outside it by `Recorder::begin`, and `install` hands the
    /// recorder back, which is where the recording starts
    pub fn plan(
        &self,
        root: &str,
        firmware: Option<serde_json::Value>,
    ) -> Result<(recorder::Device, Decoder), String> {
        let key = self.readiness()?;
        let stream = self
            .bus
            .streams
            .get(&key)
            .ok_or("selected device has no reports")?;
        let (vid, pid) = self
            .names
            .get(&(key.bus, key.device))
            .copied()
            .unwrap_or((0, 0));
        let device = recorder::Device {
            root: root.into(),
            bus: key.bus,
            address: key.device,
            endpoint: key.endpoint,
            vid,
            pid,
            payload_len: stream.payload_len,
            advertised: self.endpoints.get(&key).copied(),
            box_state: firmware,
        };
        Ok((device, self.decoders.get(&key).cloned().unwrap_or_default()))
    }

    /// starts the recording `plan` was for. the device can go, or capture fail, while the files
    /// were being made, then the recorder comes back to be abandoned outside the lock
    pub fn install(&mut self, mut recorder: Recorder) -> Result<String, Refused> {
        let planned = self
            .selected
            .is_some_and(|key| recorder.device_key() == (key.bus, key.device, key.endpoint));
        if self.readiness().is_err() || !planned {
            return Err(Refused(Box::new(recorder)));
        }
        self.tee.initialize(&mut recorder);
        let stem = recorder.stem().to_string();
        self.box_offset = None;
        self.arrivals = crate::signal::alignment::Arrivals::default();
        self.input_last = None;
        self.input_reset = false;
        self.participants.clear();
        self.health.pairing_valid = true;
        self.health.notice = None;
        self.at_install = self.health.clone();
        self.recorder = Some(recorder);
        Ok(stem)
    }

    /// plan, begin and install in one go, for a caller that holds the state alone
    pub fn begin(
        &mut self,
        root: &str,
        folder: &Path,
        firmware: Option<serde_json::Value>,
    ) -> Result<String, String> {
        let (device, decoder) = self.plan(root, firmware)?;
        let mut recorder = Recorder::begin(folder, device).map_err(|e| e.to_string())?;
        recorder.decoder = decoder;
        self.install(recorder).map_err(Refused::abandon)
    }

    /// ends recording input here, between two records. closing the files and publishing the
    /// metadata is `Closing::commit`, which runs outside the lock
    pub fn close(&mut self) -> Result<Closing, String> {
        let recorder = self.recorder.take().ok_or("not recording")?;
        let shift = if self.health.pairing_valid {
            self.box_offset.and_then(i64::checked_neg)
        } else {
            None
        };
        // the counters are capture's, the metadata keeps what this recording added to them
        let mut health = self.health.clone();
        let at = &self.at_install;
        health.records = health.records.saturating_sub(at.records);
        health.reports = health.reports.saturating_sub(at.reports);
        health.timestamp_regressions = health
            .timestamp_regressions
            .saturating_sub(at.timestamp_regressions);
        health.serial_dropped = health.serial_dropped.saturating_sub(at.serial_dropped);
        Ok(Closing {
            decoder: recorder.decoder.clone(),
            recorder,
            health,
            shift,
            receipt_alignment: self.arrivals.estimate(),
            participants: std::mem::take(&mut self.participants),
        })
    }

    /// close and commit in one go, for a caller that holds the state alone. a failure lands in
    /// health with the code the metadata has
    pub fn finish(&mut self) -> Result<PathBuf, String> {
        let closing = self.close()?;
        closing
            .commit(StopReason::RecordStop, Vec::new())
            .map_err(|failure| {
                self.fail(failure.reason, &failure.message);
                failure.message
            })
    }

    pub fn input(&mut self, motion: &crate::hardware::medius::Motion) {
        if self
            .recorder
            .as_ref()
            .is_some_and(|r| motion.arrived_us < r.started_us())
        {
            return;
        }
        self.take_part(MEDIUS_INPUT);
        if self.input_last.is_some_and(|last| motion.box_us < last) {
            self.input_reset = true;
            self.degrade_pairing("Medius input clock reset");
        }
        self.input_last = Some(motion.box_us);
        // rows after a reset would sit on another clock than the ones before them
        if self.input_reset {
            return;
        }
        let offset = motion.arrived_us as i64 - motion.box_us as i64;
        self.box_offset = Some(self.box_offset.map_or(offset, |old| old.min(offset)));
        if self.recorder.is_some()
            && let Err(error) = self.arrivals.push(motion.box_us, motion.arrived_qpc)
        {
            self.degrade_pairing(format!("Medius input could not be lined up: {error}"));
        }
        if let Some(recorder) = &mut self.recorder {
            recorder.receipt(motion);
            recorder.input(motion.box_us, motion.dx, motion.dy, motion.dz);
        }
    }
}

/// a recorder `install` refused, its files still there for `abandon` to take away once the
/// lock is let go
#[must_use = "a refused recorder's files stay until it is abandoned"]
pub struct Refused(Box<Recorder>);
impl Refused {
    /// its empty files and marker go, and why it was refused comes back
    pub fn abandon(self) -> String {
        self.0.abandon();
        "capture or the selected device changed before recording started".into()
    }
}

pub struct Capture {
    stopping: Arc<std::sync::atomic::AtomicBool>,
    /// none for a capture replayed in tests, which has no USBPcapCMD behind it
    child: Option<Child>,
    pub data: Arc<Mutex<Data>>,
    worker: Option<JoinHandle<()>>,
    stderr: Option<JoinHandle<()>>,
    pub root: String,
}

impl Capture {
    pub fn start(root: &str) -> Result<Self, String> {
        let exe =
            pcap::usbpcapcmd().ok_or("USBPcapCMD.exe was not found; install USBPcap first")?;
        let mut child = Command::new(exe)
            .args([
                "-d",
                root,
                "-o",
                "-",
                "-b",
                &pcap::BUFFER_BYTES.to_string(),
                "--capture-from-all-devices",
                "--inject-descriptors",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let data = Arc::new(Mutex::new(Data::new(true)));
        let stopping = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_flag = stopping.clone();
        let state = data.clone();
        // USBPcap's stderr usually closes a moment after its stdout, so its words come over
        // this and the stdout side decides once, or the plain stream end would win
        let (said, heard) = std::sync::mpsc::channel::<String>();
        let worker = std::thread::spawn(move || stream(stdout, &heard, &stop_flag, &state));
        let state = data.clone();
        let stderr = std::thread::spawn(move || {
            use std::io::Read;
            let mut text = Vec::new();
            let mut reader = BufReader::new(stderr);
            let mut block = [0u8; 4096];
            let mut error = None;
            loop {
                match reader.read(&mut block) {
                    Ok(0) => break,
                    Ok(count) => {
                        let remaining = (64 * 1024usize).saturating_sub(text.len());
                        text.extend_from_slice(&block[..count.min(remaining)]);
                    }
                    Err(e) => {
                        error = Some(format!("its stderr could not be read, {e}"));
                        break;
                    }
                }
            }
            let text = String::from_utf8_lossy(&text).trim().to_string();
            let words = [Some(text), error]
                .into_iter()
                .flatten()
                .filter(|w| !w.is_empty());
            let words = words.collect::<Vec<_>>().join(", ");
            // under the lock the stdout side decides under, so the words are either heard there
            // or added to what it decided
            let mut state = state.locked();
            if state.health.ended {
                state.add_driver_words(&words);
            } else {
                let _ = said.send(words);
            }
        });
        Ok(Self {
            stopping,
            child: Some(child),
            data,
            worker: Some(worker),
            stderr: Some(stderr),
            root: root.into(),
        })
    }

    /// `data` as a running capture on `root`, for tests that drive it by hand
    #[cfg(any(test, feature = "testkit"))]
    pub fn replayed(data: Data, root: &str) -> Self {
        Self {
            stopping: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            child: None,
            data: Arc::new(Mutex::new(data)),
            worker: None,
            stderr: None,
            root: root.into(),
        }
    }

    pub fn stop(&mut self) {
        self.stopping
            .store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(stderr) = self.stderr.take() {
            let _ = stderr.join();
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop();
    }
}

/// a record into the live state the app shares. the recording's files are written after the
/// lock is let go, so the live view and the commands never wait on the disk
pub fn ingest_shared(data: &Mutex<Data>, record: &Record<'_>) {
    let mut data = data.locked();
    data.take_in(record);
    if let Some(writes) = data.writes() {
        writes.after(data);
    }
}

/// reads capture's stream into `data` until it ends, then fails capture with how it ended. a
/// panic while reading ends it like a broken stream, so capture never just stops without a word
pub(crate) fn stream(
    stdout: impl std::io::Read,
    heard: &std::sync::mpsc::Receiver<String>,
    stopping: &std::sync::atomic::AtomicBool,
    data: &Mutex<Data>,
) {
    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        pcap::read_pcap(BufReader::new(stdout), |record| {
            ingest_shared(data, &record)
        })
        .map_err(|e| e.to_string())
    }))
    .unwrap_or_else(|_| Err("capture's reader panicked".into()));
    // read now, a stop asked for while it waits for stderr below came after the stream ended
    let stopping = stopping.load(std::sync::atomic::Ordering::Relaxed);
    let waited = heard.recv_timeout(std::time::Duration::from_secs(1)).ok();
    let mut data = data.locked();
    let said = waited.or_else(|| heard.try_recv().ok()).unwrap_or_default();
    if let Some(failure) = ending(read, stopping, &said) {
        data.fail(failure.reason, failure.message);
    }
    data.health.ended = true;
}

/// how capture ended, from what reading its stream gave and what USBPcap said on stderr. a
/// stop that was asked for is no failure, whatever the reader saw while the process was killed
pub(crate) fn ending(read: Result<(), String>, stopping: bool, said: &str) -> Option<Failure> {
    // it lands in the sidecar and every snapshot, a page of it is plenty
    let said: String = said.trim().chars().take(2000).collect();
    let said = said.as_str();
    match read {
        _ if stopping => None,
        Err(error) if said.is_empty() => Some(Failure::new(Reason::TransportFailed, error)),
        Err(error) => Some(Failure::new(
            Reason::TransportFailed,
            format!("{error}, and USBPcap said: {said}"),
        )),
        Ok(()) if said.is_empty() => Some(Failure::new(
            Reason::TransportEnded,
            "USBPcap stream ended unexpectedly",
        )),
        Ok(()) => Some(Failure::new(Reason::DriverReported, said)),
    }
}

#[cfg(test)]
mod tests;
