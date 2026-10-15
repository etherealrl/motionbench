//! the snapshot pump and what it names each endpoint by
use crate::{app::state::Shared, recording};
use motion_bench::sync::LockExt;

use crate::capture::connected_hid;

use motion_bench::{
    capture::{
        pcap::Endpoint,
        session::{self, Device},
    },
    evidence::commit::participant::OBSERVERS,
    hardware::medius,
    signal::stats::Key,
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::atomic::Ordering,
};

/// the ids and interface of every endpoint on the wire, sorted: the set the Windows names are read
/// for, so the pump only walks every HID device again when one comes or goes
pub(crate) fn name_keys(
    endpoints: &BTreeMap<Key, Endpoint>,
    names: &BTreeMap<(u16, u16), (u16, u16)>,
) -> Vec<(u16, u16, u8)> {
    let mut keys: Vec<(u16, u16, u8)> = endpoints
        .iter()
        .filter_map(|(key, endpoint)| {
            let (vid, pid) = *names.get(&(key.bus, key.device))?;
            Some((vid, pid, endpoint.interface))
        })
        .collect();
    keys.sort_unstable();
    keys.dedup();
    keys
}

/// where a device row sits in the capture
pub(crate) fn key_of(device: &Device) -> motion_bench::signal::stats::Key {
    motion_bench::signal::stats::Key {
        bus: device.bus,
        device: device.address,
        endpoint: device.endpoint,
    }
}

/// what Windows calls this endpoint, when it listed the same ids and interface
pub(crate) fn windows_name(
    device: &Device,
    interface: Option<u8>,
    names: &HashMap<(u16, u16, u8), String>,
) -> Option<String> {
    let (vid, pid, interface) = (device.vid?, device.pid?, interface?);
    names.get(&(vid, pid, interface)).cloned()
}

/// the connected box that clones this endpoint, by the name its owner gave it, when it is the clone
pub(crate) fn cloned_by(device: &Device, firmware: &medius::Snapshot) -> Option<String> {
    let (vid, pid) = (firmware.clone_vid?, firmware.clone_pid?);
    if !firmware.connected || device.vid != Some(vid) || device.pid != Some(pid) {
        return None;
    }
    box_label(firmware)
}

/// how far apart a press on the box and the same press on the wire can be. the box sends it
/// over its serial port and passes it on over USB within a few milliseconds
pub(crate) const PRESS_WITHIN_US: u64 = 50_000;

/// the endpoint the box's mouse is, found from a press the box reported that one endpoint's
/// reports show at the same moment. kept until the box goes away or connects again
pub(crate) fn box_endpoint(
    known: Option<(Key, u64)>,
    data: &session::Data,
    firmware: &medius::Snapshot,
    pressed_us: Option<u64>,
) -> Option<(Key, u64)> {
    if !firmware.connected {
        return None;
    }
    pressed_us
        .and_then(|at| data.pressed_at(at, PRESS_WITHIN_US))
        .map(|key| (key, firmware.generation))
        .or(known.filter(|(_, generation)| *generation == firmware.generation))
}

/// the box as its badge reads: its kind and its own name, once when the name starts with the kind
pub(crate) fn box_label(firmware: &medius::Snapshot) -> Option<String> {
    let (family, name) = (firmware.family.trim(), firmware.name.trim());
    let label = match (family.is_empty(), name.is_empty()) {
        (_, true) => family.to_string(),
        (true, false) => name.to_string(),
        _ if name.starts_with(family) => name.to_string(),
        _ => format!("{family} - {name}"),
    };
    (!label.is_empty()).then_some(label)
}

/// the box's own name for the device it clones, for when Windows named nothing
pub(crate) fn clone_product(firmware: &medius::Snapshot) -> Option<String> {
    let name = firmware.clone_product.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// one round of the snapshot pump: the box's input and the failures it sees go to the
/// capture and the coordinator, and the next snapshot goes to the page once it took the last
pub(crate) fn pump(worker: &Shared) {
    let mut frame = None;
    let mut names_for = None;
    {
        let mut bench = worker.bench.locked();
        let firmware = bench.link.snapshot();
        // with no capture there is no endpoint to be the box's
        let mut endpoint = None;
        if let Some(capture) = &bench.capture {
            let mut data = capture.data.locked();
            let keys = name_keys(&data.endpoints, &data.names);
            if keys != bench.names_for {
                names_for = Some(keys);
            }
            recording::box_input(
                &mut data,
                bench.link.motion.try_iter(),
                &firmware,
                (bench.seen_serial, bench.seen_discontinuities),
                "Medius reports were lost or its session reset during recording",
            );
            if let Some(error) = bench.aim_error() {
                data.fail_injection(format!("injection experiment failed: {error}"));
            }
            if let Some(error) = bench.observers.as_ref().and_then(|o| o.snapshot().error) {
                data.degrade(
                    OBSERVERS,
                    format!("input observer evidence failed: {error}"),
                );
            }
            endpoint = box_endpoint(
                bench.box_endpoint,
                &data,
                &firmware,
                bench.buttons.pressed_us(),
            );
            recording::watch(worker, &data);
            // the box's input is written from here too, so it never waits in memory for the
            // next usb report
            if let Some(writes) = data.writes() {
                writes.after(data);
            }
        } else {
            for _ in bench.link.motion.try_iter() {}
        }
        bench.box_endpoint = endpoint;
        bench.seen_serial = recording::box_losses(&firmware);
        bench.seen_discontinuities = firmware.discontinuities;
        let in_use = bench
            .capture
            .as_ref()
            .is_some_and(|capture| capture.data.locked().recorder.is_some())
            || bench.buttons.aim().armed;
        bench.link.hold_still(in_use);
        if firmware.remember.is_some() && firmware.remember != bench.settings().get().capture.box_ {
            let _ = bench
                .settings()
                .update(|s| s.capture.box_ = firmware.remember.clone());
        }
        if let Some(channel) = bench.channel.clone() {
            let pending = worker.sequence.load(Ordering::Relaxed);
            if worker.ack.load(Ordering::Relaxed) >= pending {
                let seq = pending + 1;
                worker.sequence.store(seq, Ordering::Relaxed);
                let (snapshot, unplotted) = bench.snapshot_unplotted(seq);
                frame = Some((snapshot, unplotted, bench.live.views(), channel));
            }
        }
    }
    if let Some(keys) = names_for {
        // Windows is asked outside the bench lock: the enumeration walks every HID device
        let names = connected_hid::names();
        let layouts = connected_hid::layouts(&keys);
        let mut bench = worker.bench.locked();
        if let Some(capture) = &bench.capture {
            let mut data = capture.data.locked();
            let found: Vec<_> = data
                .endpoints
                .iter()
                .filter_map(|(key, endpoint)| {
                    let (vid, pid) = *data.names.get(&(key.bus, key.device))?;
                    Some((*key, layouts.get(&(vid, pid, endpoint.interface))?.clone()))
                })
                .collect();
            for (key, layout) in found {
                data.remember_windows_layout(key, layout);
            }
        }
        bench.device_names = names;
        bench.names_for = keys;
    }
    if let Some((mut snapshot, unplotted, views, channel)) = frame {
        snapshot.plot = unplotted.plot(views);
        let seq = snapshot.seq;
        worker
            .bench
            .locked()
            .live
            .settle(&snapshot.plot, unplotted.resets);
        if channel.send(snapshot).is_err() {
            let mut bench = worker.bench.locked();
            // a page that subscribed again meanwhile keeps its channel
            if bench
                .channel
                .as_ref()
                .is_some_and(|c| c.id() == channel.id())
            {
                bench.channel = None;
                worker.ack.store(seq, Ordering::Relaxed);
            }
        }
    }
}
