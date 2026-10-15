//! inputs shared by tests in more than one module

use crate::signal::{stats::View, ticks::tick};

// the busiest stream of the real 1 kHz fixture as 1 ms ticks, the input the parity tests share
pub fn fixture_ticks() -> Vec<[i32; 2]> {
    use crate::capture::pcap::{Event, Record, read_pcap};
    use crate::signal::stats::Bus;

    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/mouse-1000hz.pcap"
    );
    let raw = std::fs::read(path).unwrap();
    let mut bus = Bus::whole();
    read_pcap(&raw[..], |record| {
        if let Record::Packet {
            event: Some(Event::Report(r)),
            ..
        } = record
        {
            bus.push(&r);
        }
    })
    .unwrap();
    let key = bus.busiest().unwrap();
    ticks(&bus.streams[&key].view(0, u64::MAX))
}

/// one (dx, dy) per millisecond tick from the first report in the view. reports landing in the
/// same tick are summed, a tick nothing landed in is zero. the same rule the parity script uses,
/// and the same rule abcurves' own loader uses, floor from the first report. one row per report
/// would be wrong: it invents a uniform cadence the wire does not have, and would erase exactly
/// the gap structure being measured
pub fn ticks(view: &View<'_>) -> Vec<[i32; 2]> {
    let mut out: Vec<[i32; 2]> = Vec::new();
    let mut first = None;
    for (t_us, dx, dy) in view.motion() {
        let Some(i) = tick(*first.get_or_insert(t_us), t_us) else {
            continue;
        };
        if i >= out.len() {
            out.resize(i + 1, [0, 0]);
        }
        out[i][0] += dx;
        out[i][1] += dy;
    }
    out
}

/// ticks from an origin the caller fixed, exactly `count` of them, so two views windowed on the
/// same instant produce two tick streams that line up index for index
pub fn ticks_from(view: &View<'_>, origin: u64, count: usize) -> Vec<[i32; 2]> {
    let mut out = vec![[0i32; 2]; count];
    for (t_us, dx, dy) in view.motion() {
        if let Some(i) = tick(origin, t_us).filter(|i| *i < count) {
            out[i][0] += dx;
            out[i][1] += dy;
        }
    }
    out
}

/// the metadata a finished recording of the test traffic saves
pub fn sidecar(outcome: &str) -> serde_json::Value {
    serde_json::json!({
        "schema": crate::evidence::commit::RECORDING_SCHEMA,
        "outcome": outcome,
        "reason": null,
        "stop_reason": "record_stop",
        "health": {"error": null, "pairing_valid": true},
        "participants": [],
        "decoder": crate::testkit::traffic::mouse_decoder(),
    })
}

/// a copy of a fixture capture in `folder`, with `sidecar` beside it as its metadata
pub fn recorded(
    folder: &std::path::Path,
    capture: &str,
    sidecar: &serde_json::Value,
) -> std::path::PathBuf {
    std::fs::create_dir_all(folder).unwrap();
    let path = folder.join(capture);
    std::fs::write(&path, crate::testkit::traffic::fixture(capture)).unwrap();
    std::fs::write(path.with_extension("json"), sidecar.to_string()).unwrap();
    path
}

/// the target capture evidence a recording writes, for the given scenario
pub fn vision_evidence(scenario: &str) -> String {
    use crate::targets::vision::{Config, Evidence};
    serde_json::to_string(&Evidence {
        schema: "motion-bench.vision.v1".into(),
        config: Config {
            scenario: scenario.into(),
            ..Default::default()
        },
        model_sha256: "x".into(),
        provider: "CUDA".into(),
        pipeline: "synthetic".into(),
        inference: None,
        complete: true,
        error: None,
        clock: Some(screen_clock()),
    })
    .unwrap()
}

/// the qpc a fixture frame was taken at
const FRAME_QPC: i64 = 10_000_000;

/// the clock fixture frames follow from
pub fn screen_clock() -> crate::targets::vision::Clock {
    crate::targets::vision::Clock {
        initial_bracket_us: 0.0,
        qpc_origin: FRAME_QPC,
        qpc_frequency: 10_000_000,
        epoch_origin_us: 0,
    }
}

/// a screen frame the way the vision worker writes it, on the fixture clock
pub fn frame(
    source_us: u64,
    detections: Vec<crate::targets::vision::Detection>,
) -> crate::targets::vision::Frame {
    let anchor = crate::capture::clock::Anchor {
        qpc_before: FRAME_QPC,
        qpc_after: FRAME_QPC,
        utc_us: source_us,
    };
    crate::targets::vision::Frame {
        clock_anchor: anchor,
        source_qpc: FRAME_QPC,
        received_qpc: FRAME_QPC,
        completed_qpc: FRAME_QPC,
        source_us,
        received_us: source_us + 1000,
        completed_us: source_us + 2000,
        preprocess_ms: 1.0,
        pipeline_ms: 3.0,
        inference_ms: 2.0,
        skipped: 0,
        detections,
    }
}

/// fnv-1a of the text, the same from one build to the next, which the std hasher isn't, so a
/// test can pin numbers it would take pages to write out
pub fn digest(text: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
