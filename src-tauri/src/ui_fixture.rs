//! the ui's fixture, made by the snapshot and plot builders the app uses, so the preview and the ui
//! tests see what the native side really sends. `bun run fixture` writes it again, and the test
//! here fails once the builders make something of another shape than the fixture

use crate::{app::state::*, commands::runs::*, recording};
use motion_bench::sync::LockExt;

use crate::library::Library;
use motion_bench::{
    analysis::views::{numbers::numbers, plot::Views},
    capture::{
        pcap,
        session::{self, Capture},
    },
    evidence::reader::recording::open_recording,
    hardware::medius,
};
use serde_json::{Value, json};
use std::path::Path;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/fixture.ts");
/// a real mouse at 1 kHz, its timing as it came
const CAPTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../tests/fixtures/mouse-1000hz.pcap"
);

/// `raw` played `copies` times back to back, the last copy starting at `last_us`, so a live
/// view of it is full. the device is described once, at the start, as on a real wire
fn looped(raw: &[u8], copies: u64, last_us: u64) -> Vec<u8> {
    let word = |at: usize| u32::from_le_bytes(raw[at..at + 4].try_into().unwrap()) as u64;
    let mut records = Vec::new();
    let mut at = 24;
    while at + 16 <= raw.len() {
        let end = at + 16 + word(at + 8) as usize;
        records.push((word(at) * 1_000_000 + word(at + 4), &raw[at..end]));
        at = end;
    }
    let mut first_report = None;
    let mut index = 0;
    pcap::read_pcap(raw, |record| {
        if let pcap::Record::Packet { event, .. } = record {
            if matches!(event, Some(pcap::Event::Report(_))) {
                first_report.get_or_insert(index);
            }
            index += 1;
        }
    })
    .unwrap();
    let (first, last) = (records[0].0, records.last().unwrap().0);
    let period = last - first + 1000;
    let mut out = raw[..24].to_vec();
    for copy in 0..copies {
        let skip = if copy == 0 { 0 } else { first_report.unwrap() };
        let shift = last_us - (copies - 1 - copy) * period - first;
        for (t, record) in &records[skip..] {
            let moved = t + shift;
            out.extend(((moved / 1_000_000) as u32).to_le_bytes());
            out.extend(((moved % 1_000_000) as u32).to_le_bytes());
            out.extend(&record[8..]);
        }
    }
    out
}

/// the box as the snapshot shows it with the crate's mock Medius connected
fn connected_box() -> Value {
    let folder = tempfile::tempdir().unwrap();
    let (controller, _) = recording::hotkey::Controller::for_test(folder.path());
    let mut bench = Bench::new(controller);
    let (device, _poller) = anybox::Device::mock_medius_manual(
        anybox::medius::MockBox::new(),
        anybox::ClickTiming::Fixed(std::time::Duration::from_millis(5)),
    )
    .unwrap();
    bench.link = medius::Link::with_device(device);
    let started = std::time::Instant::now();
    // connected comes before the box is asked who it clones
    while bench.link.snapshot().clone.is_empty() && started.elapsed().as_secs() < 8 {
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    bench.snapshot(0, Views::Readouts).box_
}

/// a live capture of a mouse with the recorded fixture in the library: the snapshot, then a
/// recording of the capture with the box's input beside it: its plot and its numbers, and the
/// box connected
fn made() -> [Value; 4] {
    let folder = tempfile::tempdir().unwrap();
    for file in ["mouse-1000hz.pcap", "mouse-1000hz.json"] {
        let from = Path::new(CAPTURE).with_file_name(file);
        std::fs::copy(from, folder.path().join(file)).unwrap();
    }
    let recorded = tempfile::tempdir().unwrap();
    let (controller, _) = recording::hotkey::Controller::for_test(folder.path());
    let mut bench = Bench::new(controller.clone());
    bench.library_loaded = true;
    let state = Shared::new(controller, bench).in_folder(folder.path());
    // on the wall clock, which the box's input is lined up with
    let recording_from = session::wall_us();
    let capture = looped(&std::fs::read(CAPTURE).unwrap(), 10, recording_from);
    let decoder = open_recording(Path::new(CAPTURE), None).unwrap().streams[0]
        .decoder
        .clone()
        .unwrap();
    let anchor = motion_bench::capture::clock::Anchor::take();
    let frequency = motion_bench::capture::clock::frequency();
    let mut data = session::Data::new(true);
    let mut reports = 0;
    pcap::read_pcap(&capture[..], |record| {
        data.ingest(&record);
        let pcap::Record::Packet {
            event: Some(pcap::Event::Report(report)),
            ..
        } = &record
        else {
            return;
        };
        if data.selected.is_none() {
            let id = format!("{}:{}:{}", report.bus, report.device, report.endpoint);
            data.select(&id, Some(decoder.clone())).unwrap();
            data.trust_selection();
        }
        // only the last copy is recorded, it runs on the wall clock the box's input needs
        if report.t_us < recording_from {
            return;
        }
        reports += 1;
        if data.recorder.is_none() {
            data.begin("USBPcap1", recorded.path(), None).unwrap();
            return;
        }
        // the box's view of the same report, 300 µs later on its clock, with one left out
        if reports == 200 {
            return;
        }
        let Some((dx, dy)) = decoder.decode(report.bytes()) else {
            return;
        };
        let arrived_us = report.t_us + 600;
        let since = (arrived_us as i64 - anchor.utc_us as i64) * frequency / 1_000_000;
        data.input(&medius::Motion {
            arrived_us,
            arrived_qpc: anchor.midpoint() + since,
            box_us: report.t_us + 300,
            dx: dx as i16,
            dy: dy as i16,
            dz: 0,
        });
    })
    .unwrap();
    data.finish().unwrap();
    // read all at once, the capture would look seconds late, a live one is read as it comes
    for delivery in data.delivery.values_mut() {
        *delivery = session::Delivery {
            delay_us: 0,
            since_us: session::wall_us(),
        };
    }
    let last = data.bus.streams.values().filter_map(|s| s.last_us()).max();
    state.bench.locked().capture = Some(Capture::replayed(data, "USBPcap1"));
    // the live view follows the wall clock, and the reports were stamped ahead of it
    let last = last.unwrap();
    while session::wall_us() < last {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(state.library().restore().is_empty());
    let mut snapshot = serde_json::to_value(state.bench.locked().snapshot(0, Views::All)).unwrap();
    snapshot["runs"] = serde_json::to_value(state.library().listed()).unwrap()["runs"].clone();
    let paired = Library::new(recorded.path().to_path_buf());
    assert!(paired.restore().is_empty());
    let listed = serde_json::to_value(paired.listed()).unwrap();
    let run = paired
        .run(listed["runs"][0]["id"].as_str().unwrap())
        .unwrap();
    let plot = wire_plot(&run, None, None, true).unwrap();
    assert!(plot.tabs.relay.is_some(), "the recording has no relay");
    let figures = numbers(&wire_plot(&run, None, None, false).unwrap());
    // the temporary folder would put this machine's paths in the repository
    let text = serde_json::to_string(&json!([snapshot, plot, figures, connected_box()])).unwrap();
    let text = [folder.path(), recorded.path()]
        .iter()
        .fold(text, |text, local| {
            let local = serde_json::to_string(&local.display().to_string()).unwrap();
            text.replace(local.trim_matches('"'), "recordings")
        });
    // a path the replace missed, written another way, would still name this machine
    assert!(
        !text.contains(":\\\\"),
        "a local path is left in the fixture"
    );
    serde_json::from_str(&text).unwrap()
}

const NAMES: [(&str, &str); 4] = [
    ("snapshot", "Snapshot"),
    ("runPlot", "Plot"),
    ("numbers", "Numbers"),
    ("connectedBox", "BoxState"),
];

fn written(values: &[Value; 4]) -> String {
    let mut text = String::from(
        "// made by the app's own snapshot and plot builders, `bun run fixture` writes it again\n\
         import type { BoxState, Numbers, Plot, Snapshot } from \"./types\";\n",
    );
    for ((name, kind), value) in NAMES.iter().zip(values) {
        let value = serde_json::to_string_pretty(value).unwrap();
        text.push_str(&format!("\nexport const {name}: {kind} = {value};\n"));
    }
    text
}

fn read() -> [Value; 4] {
    let text = std::fs::read_to_string(FIXTURE)
        .unwrap()
        .replace("\r\n", "\n");
    NAMES.map(|(name, kind)| {
        let head = format!("export const {name}: {kind} = ");
        let rest = &text[text.find(&head).unwrap() + head.len()..];
        // the value's closing brace is the only one at the start of a line
        serde_json::from_str(&rest[..rest.find("\n};").unwrap() + 2]).unwrap()
    })
}

/// a value with every number, string and bool replaced by its kind, and every list by its first
/// item, so two fixtures made at different times compare
fn shape(value: &Value) -> Value {
    match value {
        Value::Object(fields) => fields
            .iter()
            .map(|(key, value)| (key.clone(), shape(value)))
            .collect(),
        Value::Array(items) => Value::Array(items.first().map(shape).into_iter().collect()),
        Value::Number(_) => "number".into(),
        Value::String(_) => "string".into(),
        Value::Bool(_) => "bool".into(),
        Value::Null => Value::Null,
    }
}

#[test]
fn the_ui_fixture_has_the_shape_the_builders_make() {
    let (made, read) = (made(), read());
    for ((name, _), (made, read)) in NAMES.iter().zip(made.iter().zip(&read)) {
        assert_eq!(shape(made), shape(read), "{name}, run `bun run fixture`");
    }
}

#[test]
#[ignore = "writes ui/fixture.ts, run by `bun run fixture`"]
fn write_the_ui_fixture() {
    std::fs::write(FIXTURE, written(&made())).unwrap();
}
