use super::*;
use crate::evidence::reader::recording::open_recording;

#[test]
fn keyboard_traffic_cannot_reanchor_the_selected_mouse_plot() {
    use crate::{capture::pcap, signal::stats::Key};
    let mut data = Data::new(true);
    let mouse = Key {
        bus: 1,
        device: 2,
        endpoint: 0x81,
    };
    data.selected = Some(mouse);
    data.decoders
        .insert(mouse, crate::testkit::traffic::mouse_decoder());
    let ingest = |data: &mut Data, device, endpoint, time| {
        data.ingest(&pcap::Record::Packet {
            header: &[0; 16],
            packet: &[],
            bus: 1,
            device,
            event: Some(pcap::Event::Report(pcap::Report {
                t_us: time,
                bus: 1,
                device,
                endpoint,
                len: 5,
                data: vec![0, 1, 0, 0, 0],
            })),
        });
    };
    ingest(&mut data, 2, 0x81, 10_000_000);
    ingest(&mut data, 2, 0x81, 10_001_000);
    let delivered = |at: u64, t_us: u64| crate::capture::session::Delivery {
        delay_us: at as i64 - t_us as i64,
        since_us: at,
    };
    data.delivery
        .insert(mouse, delivered(20_000_000, 10_001_000));
    let before = plot_at(&data, None, None, true, Views::All, 20_200_000);
    assert_eq!(before.to_us, 10_201_000);
    assert_eq!(before.readouts.rate_hz, 1000.0);
    for (device, endpoint) in [(2, 0x82), (3, 0x81)] {
        ingest(&mut data, device, endpoint, 10_200_000);
    }
    let after = plot_at(&data, None, None, true, Views::All, 20_200_000);
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(after).unwrap()
    );
    assert_eq!(
        plot_at(&data, None, None, true, Views::All, 20_300_000).to_us,
        10_301_000
    );
    ingest(&mut data, 2, 0x81, 10_301_000);
    data.delivery
        .insert(mouse, delivered(20_300_000, 10_301_000));
    assert_eq!(
        plot_at(&data, None, None, true, Views::All, 20_400_000).to_us,
        10_401_000
    );
}

/// a scale handed in is the one used, so a baseline and a candidate share their axes
#[test]
fn a_shared_scale_and_the_selected_range_are_used() {
    let path = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/mouse-1000hz.pcap"
    ));
    let data = open_recording(path, None).unwrap().into_data();
    let all = plot(&data, None, None, false, Views::All);
    let selected = plot(
        &data,
        Some(&Selection {
            from_us: all.from_us,
            to_us: all.from_us + 100_000,
        }),
        Some(Scale {
            band: [900, 1100],
            jitter_us: 30.0,
        }),
        false,
        Views::All,
    );
    assert!(selected.reports < all.reports);
    assert_eq!(selected.scale.band, [900, 1100]);
    let jitter = selected.tabs.timing.unwrap().jitter;
    assert_eq!(jitter.span_us, 30.0);
    assert_eq!(jitter.bins.len(), 61);
}
/// the fixture mouse polls at 1 kHz, and the readout says so even though slow movement leaves
/// many of its polls empty (the mean of the moving intervals said 635 Hz)
#[test]
fn the_movement_readout_of_the_1khz_fixture() {
    let path = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/mouse-1000hz.pcap"
    ));
    let data = open_recording(path, None).unwrap().into_data();
    let rate_hz = plot(&data, None, None, false, Views::All).readouts.rate_hz;
    assert_eq!(rate_hz, 1000.0);
}

/// live and unselected, the readout follows the latest second of movement; a saved view or a
/// selected range takes everything in it
#[test]
fn the_live_readout_follows_the_latest_movement() {
    use crate::{capture::pcap, signal::stats::Key};
    let mut data = Data::new(true);
    let mouse = Key {
        bus: 1,
        device: 2,
        endpoint: 0x81,
    };
    data.selected = Some(mouse);
    data.decoders
        .insert(mouse, crate::testkit::traffic::mouse_decoder());
    let mut ingest = |time| {
        data.ingest(&pcap::Record::Packet {
            header: &[0; 16],
            packet: &[],
            bus: 1,
            device: 2,
            event: Some(pcap::Event::Report(pcap::Report {
                t_us: time,
                bus: 1,
                device: 2,
                endpoint: 0x81,
                len: 5,
                data: vec![0, 1, 0, 0, 0],
            })),
        });
    };
    // three seconds at 500 Hz, then one at 1 kHz
    for i in 0..1500_u64 {
        ingest(1_000_000 + i * 2000);
    }
    for i in 0..1000_u64 {
        ingest(5_000_000 + i * 1000);
    }
    data.delivery.insert(
        mouse,
        crate::capture::session::Delivery {
            delay_us: 0,
            since_us: 0,
        },
    );
    assert_eq!(
        plot_at(&data, None, None, true, Views::All, 6_000_000)
            .readouts
            .rate_hz,
        1000.0
    );
    assert_eq!(
        plot_at(&data, None, None, false, Views::All, 6_000_000)
            .readouts
            .rate_hz,
        500.0
    );
    let whole = Selection {
        from_us: 0,
        to_us: 6_000_000,
    };
    assert_eq!(
        plot_at(&data, Some(&whole), None, true, Views::All, 6_000_000)
            .readouts
            .rate_hz,
        500.0
    );
}

/// movement shows when the stream holds decoded axes, whatever decoder the wire had: the box's
/// own input is decoded before it gets here
#[test]
fn movement_shows_when_the_stream_holds_decoded_axes() {
    let mouse = crate::signal::stats::Key {
        bus: 1,
        device: 2,
        endpoint: 0x81,
    };
    let plotted = |motion: Option<(i32, i32)>| {
        let mut data = Data::new(false);
        let mut stream = crate::signal::stats::Stream::new(u64::MAX);
        for t in 0..100 {
            stream.push_decoded(1_000_000 + t * 1000, motion);
        }
        data.bus.streams.insert(mouse, stream);
        data.selected = Some(mouse);
        plot(&data, None, None, false, Views::All).movement_available
    };
    assert!(plotted(Some((1, -1))));
    assert!(plotted(Some((0, 0))));
    assert!(!plotted(None));
}

/// live, a frame computes only what it is asked for, and the burst navigator waits for a
/// paused range; a saved recording computes every tab
#[test]
fn a_plot_computes_the_tabs_it_is_asked_for() {
    let mouse = crate::signal::stats::Key {
        bus: 1,
        device: 2,
        endpoint: 0x81,
    };
    let mut data = Data::new(false);
    let mut stream = crate::signal::stats::Stream::new(u64::MAX);
    for t in 0..100 {
        stream.push_decoded(1_000_000 + t * 1000, Some((1, 0)));
    }
    data.bus.streams.insert(mouse, stream);
    data.selected = Some(mouse);
    let at = |range: Option<&Selection>, live, views| {
        plot_at(&data, range, None, live, views, 1_100_000)
    };
    let readouts = at(None, true, Views::Readouts);
    assert!(readouts.tabs.timing.is_none() && readouts.tabs.movement.is_none());
    assert_eq!(readouts.readouts.period_us, Some(1000.0));
    assert_eq!(readouts.readouts.moving, 99);
    let live = at(None, true, Views::Tab(Tab::Timing));
    assert!(live.tabs.timing.unwrap().bursts.is_none() && live.tabs.movement.is_none());
    let paused = Selection {
        from_us: 1_000_000,
        to_us: 1_099_000,
    };
    let paused = at(Some(&paused), true, Views::Tab(Tab::Timing));
    assert_eq!(paused.tabs.timing.unwrap().bursts.unwrap().rows.len(), 1);
    let saved = at(None, false, Views::All);
    assert!(saved.tabs.timing.is_some() && saved.tabs.movement.is_some());
}

/// without a decoder nothing moves: the timeline keeps every gap, the moving views are empty
#[test]
fn without_a_decoder_the_moving_views_are_empty() {
    let path = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/mouse-1000hz.pcap"
    ));
    let data = open_recording(path, Some(crate::capture::hid::Decoder::Unavailable))
        .unwrap()
        .into_data();
    let plot = plot(&data, None, None, false, Views::All);
    assert!(plot.reports > 0 && !plot.movement_available);
    assert_eq!(plot.readouts.period_us, None);
    assert_eq!(plot.readouts.moving, 0);
    assert!(plot.timeline.iter().flatten().all(|c| c[3] == 0));
    assert!(plot.tabs.movement.is_none());
    assert_eq!(plot.tabs.timing.unwrap().groups, [0; 5]);
}

/// a live plot computed from the copy taken under the capture's lock is the plot of the capture
/// itself, live and for a paused range
#[test]
fn a_plot_of_the_detached_copy_is_the_live_plot() {
    let mouse = crate::signal::stats::Key {
        bus: 1,
        device: 2,
        endpoint: 0x81,
    };
    let mut data = Data::new(true);
    let mut stream = crate::signal::stats::Stream::default();
    for i in 0..20_000u64 {
        stream.push_decoded(1_000_000 + i * 1000, Some((1 + (i % 3) as i32, 0)));
    }
    data.bus.streams.insert(mouse, stream);
    data.selected = Some(mouse);
    let same = |range: Option<&Selection>| {
        let copy = detach(&data, range);
        assert_eq!(
            serde_json::to_value(plot(&copy, range, None, true, Views::Tab(Tab::Timing))).unwrap(),
            serde_json::to_value(plot(&data, range, None, true, Views::Tab(Tab::Timing))).unwrap()
        );
        copy.bus.streams[&mouse].view(0, u64::MAX).reports()
    };
    // live, the 8 seconds the capture keeps
    assert_eq!(same(None), 8001);
    let paused = Selection {
        from_us: 15_000_000,
        to_us: 16_000_000,
    };
    // the paused second and nothing past it
    assert_eq!(same(Some(&paused)), 1001);
}

/// what the live copy costs under the capture's lock at 8 kHz, for a range picked from the live
/// view. the live view itself copies at most twice that span
#[test]
#[ignore = "a measurement, run by hand with --nocapture"]
fn measure_the_live_copy() {
    use crate::testkit::traffic::{Descriptors, Packet, Traffic, mouse_payload, ticks};
    let mut traffic = Traffic::default();
    traffic.describe(0, 1, 2, &Descriptors::mouse(0x373b, 0x11d9));
    traffic.extend(
        ticks(8000, 1_000_000, 8000 * 20)
            .map(|t| (t, Packet::report(1, 2, 0x81, &mouse_payload(1, -1, 7)))),
    );
    let mut data = Data::new(true);
    crate::capture::pcap::read_pcap(traffic.pcap().bytes(), |record| {
        data.ingest(&record);
        if data.selected.is_none() && data.health.reports > 0 {
            data.select("1:2:129", Some(crate::testkit::traffic::mouse_decoder()))
                .unwrap();
        }
    })
    .unwrap();
    let last = data
        .bus
        .streams
        .values()
        .map(|s| s.last_us().unwrap())
        .max()
        .unwrap();
    let picked = Selection {
        from_us: last - 8_000_000,
        to_us: last,
    };
    let frames = 200;
    let started = std::time::Instant::now();
    let mut span = 0;
    for _ in 0..frames {
        let copy = detach(&data, Some(&picked));
        span = copy
            .bus
            .streams
            .values()
            .map(|s| s.last_us().unwrap_or(0) - s.first_us().unwrap_or(0))
            .sum::<u64>();
    }
    println!(
        "{span} us of 8 kHz reports, {:?} a frame",
        started.elapsed() / frames
    );
}
