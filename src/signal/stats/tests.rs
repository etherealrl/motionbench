#[test]
fn active_rate_excludes_stationary_reports_and_silent_bursts() {
    let mut stream = Stream::new(u64::MAX);
    stream.push_decoded(0, Some((0, 0)));
    for time in [1_000_000, 1_001_000, 1_002_000, 4_000_000, 4_001_000] {
        stream.push_decoded(time, Some((1, 0)));
    }
    stream.push_decoded(4_002_000, Some((0, 0)));
    stream.push_decoded(5_000_000, Some((0, 0)));
    assert_eq!(stream.view(0, 8_000_000).movement_rate_hz(None), 1000.0);
    assert_eq!(
        stream.view(1_000_000, 1_002_000).movement_rate_hz(None),
        1000.0
    );
    assert_eq!(
        stream.view(4_002_000, 8_000_000).movement_rate_hz(None),
        0.0
    );
    assert_eq!(all(&stream).gaps().max().unwrap(), 2_998_000);
}

/// slow movement leaves polls empty, so most intervals stay at the poll period while a few are
/// longer; the median keeps the rate at the mouse's, and the recent window forgets older bursts
#[test]
fn active_rate_follows_the_poll_period_and_the_latest_movement() {
    let mut stream = Stream::new(u64::MAX);
    // a second at 1 kHz with every fourth poll empty, as slow movement leaves it
    for i in 0..1000_u64 {
        if i % 4 != 3 {
            stream.push_decoded(1_000_000 + i * 1000, Some((1, 0)));
        }
    }
    assert_eq!(all(&stream).movement_rate_hz(None), 1000.0);
    // then an older burst at 500 Hz and a later one at 2 kHz, two seconds apart
    let mut bursts = Stream::new(u64::MAX);
    for i in 0..500_u64 {
        bursts.push_decoded(1_000_000 + i * 2000, Some((1, 0)));
    }
    for i in 0..2000_u64 {
        bursts.push_decoded(4_000_000 + i * 500, Some((1, 0)));
    }
    assert_eq!(all(&bursts).movement_rate_hz(Some(1_000_000)), 2000.0);
    // stopping holds the last movement's rate, it does not fall to nothing
    bursts.push_decoded(9_000_000, Some((0, 0)));
    assert_eq!(all(&bursts).movement_rate_hz(Some(1_000_000)), 2000.0);
    // over the whole view the longer burst decides
    assert_eq!(all(&bursts).movement_rate_hz(None), 2000.0);
}

#[test]
fn active_rate_keeps_missed_polls_and_requires_decoded_movement() {
    let mut stream = Stream::new(u64::MAX);
    for time in [0, 1000, 3000] {
        stream.push_decoded(time, Some((0, -1)));
    }
    assert!((all(&stream).movement_rate_hz(None) - 2e6 / 3000.0).abs() < 1e-9);
    assert_eq!(all(&stream).gaps().max().unwrap(), 2000);
    let mut unknown = Stream::new(u64::MAX);
    unknown.push_decoded(0, None);
    unknown.push_decoded(1000, None);
    assert_eq!(all(&unknown).movement_rate_hz(None), 0.0);
    assert_eq!(stream.view(1000, 1000).movement_rate_hz(None), 0.0);
}

use super::*;
use crate::capture::pcap::Report;

fn train(period_us: u64, n: u64) -> Stream {
    let mut s = Stream::default();
    for i in 0..n {
        s.push(i * period_us, &[0; 7]);
    }
    s
}

fn all(s: &Stream) -> View<'_> {
    s.view(0, u64::MAX)
}

#[test]
fn a_clean_1000hz_train_reads_as_1000hz() {
    let s = train(1000, 2000);

    assert!(all(&s).gaps().all(|g| g == 1000));
    assert_eq!(s.reports_last_second(), 1000);
}

#[test]
fn the_window_holds_seconds_not_a_sample_count() {
    let slow = train(1000, 20_000);
    let fast = train(125, 160_000);

    for s in [&slow, &fast] {
        let first = s.samples.front().unwrap().t_us;
        let last = s.last_us().unwrap();
        assert!(last - first <= HISTORY_US);
        assert!(last - first > HISTORY_US - 10_000);
    }
    assert_eq!(slow.total, 20_000);
    assert!(all(&fast).gaps().all(|g| g == 125));
}

#[test]
fn an_idle_device_reports_no_recent_traffic() {
    let mut s = train(1000, 500);
    s.push(10_000_000, &[0; 7]);

    assert_eq!(s.reports_last_second(), 1);
}

#[test]
fn on_time_range_ignores_skips_and_idle() {
    let mut s = Stream::default();
    let mut t = 0;
    for i in 0..400u64 {
        t += if i % 2 == 0 { 990 } else { 1010 };
        s.push(t, &[0; 7]);
    }
    s.push(t + 2000, &[0; 7]);
    s.push(t + 2000 + 171_000, &[0; 7]);

    assert_eq!(all(&s).on_time_range(1000), Some((990, 1010)));
}

#[test]
fn silence_leaves_its_columns_empty() {
    let mut s = train(1000, 100);
    s.push(99_000 + 100_000, &[0; 7]);

    let (_, _, columns) = s.view(0, 199_000).timeline(20);

    assert_eq!(columns.len(), 20);
    assert!(columns[0].is_some());
    assert!(columns[10].is_none());
    assert_eq!(columns[19].unwrap()[1], 100_000);
}

/// a column holds its median gap and whether a report in it moved, so the timeline can draw
/// a median line and shade where movement was observed
#[test]
fn a_column_holds_its_median_and_whether_it_moved() {
    let mut s = Stream::new(u64::MAX);
    for (t, moved) in [
        (0, true),
        (1000, true),
        (2000, true),
        (5000, true),
        (10_000, false),
    ] {
        s.push_decoded(t, Some((i32::from(moved), 0)));
    }
    s.push_decoded(13_000, Some((0, 0)));
    let (_, _, columns) = s.view(0, 20_000).timeline(2);
    assert_eq!(columns[0], Some([1000, 3000, 1000, 1]));
    assert_eq!(columns[1], Some([3000, 5000, 4000, 0]));
}

/// the window has to keep sliding after the reports stop, or current silence is invisible
#[test]
fn a_window_past_the_last_report_is_all_silence() {
    let s = train(1000, 100);

    let (_, _, columns) = s.view(200_000, 300_000).timeline(10);

    assert!(columns.iter().all(|c| c.is_none()));
}

/// a live view slides a little every frame; each report must stay in the column it was in, or
/// the drawn spikes jump and vanish as columns regroup
#[test]
fn a_sliding_view_keeps_every_report_in_its_column() {
    let mut s = train(1000, 100);
    s.push(99_000 + 30_000, &[0; 7]);
    let column_of = |from: u64| {
        let (start, step, columns) = s.view(from, from + 100_000).timeline(10);
        assert_eq!(step, 10_000);
        let i = columns
            .iter()
            .position(|c| c.is_some_and(|c| c[1] == 30_000))
            .unwrap();
        start + i as u64 * step
    };
    // the long interval ends at 129 ms, its column starts at 120 ms wherever the view begins
    for from in [40_000, 43_217, 48_999, 55_001] {
        assert_eq!(column_of(from), 120_000, "{from}");
    }
}

#[test]
fn a_report_counts_as_still_only_when_nothing_past_the_buttons_is_set() {
    let mut s = Stream::default();
    s.push(0, &[0x00, 0, 0, 0, 0, 0, 0]);
    s.push(1000, &[0x01, 0, 0, 0, 0, 0, 0]);
    s.push(2000, &[0x00, 0x01, 0, 0, 0, 0, 0]);

    assert_eq!(all(&s).reports(), 3);
    assert_eq!(all(&s).still(), 2);
}

/// the picker scopes the numbers as well as the plot, so both describe the same seconds
#[test]
fn a_view_only_sees_its_own_slice() {
    let mut s = train(1000, 100);
    s.push(99_000 + 40_000, &[0; 7]);

    let recent = s.view(90_000, 139_000);

    assert_eq!(recent.reports(), 11);
    assert_eq!(recent.gaps().max().unwrap(), 40_000);
    assert_eq!(all(&s).reports(), 101);
    assert_eq!(s.view(0, 50_000).gaps().max().unwrap(), 1000);
}

#[test]
fn a_replayed_bus_keeps_everything() {
    let mut live = Bus::live();
    let mut whole = Bus::whole();
    for i in 0..20_000u64 {
        live.push(&report(31, 0x81, i * 1000));
        whole.push(&report(31, 0x81, i * 1000));
    }

    let key = live.busiest().unwrap();
    assert_eq!(live.streams[&key].view(0, u64::MAX).reports(), 8001);
    assert_eq!(whole.streams[&key].view(0, u64::MAX).reports(), 20_000);
    assert_eq!(whole.span(), Some((0, 19_999_000)));
}

#[test]
fn busiest_endpoint_wins() {
    let mut bus = Bus::default();
    for i in 0..500u64 {
        bus.push(&report(31, 0x81, i * 1000));
        if i % 8 == 0 {
            bus.push(&report(32, 0x81, i * 1000));
        }
    }

    assert_eq!(bus.busiest().unwrap().device, 31);
}

fn report(device: u16, endpoint: u8, t_us: u64) -> Report {
    Report {
        t_us,
        bus: 1,
        device,
        endpoint,
        len: 7,
        data: vec![0; 7],
    }
}

#[test]
fn a_sample_stamped_before_the_latest_is_counted_and_left_out() {
    let mut stream = Stream::new(u64::MAX);
    for t in [2000, 3000, 1000, 4000, 4000] {
        stream.push_decoded(t, Some((1, 1)));
    }
    assert_eq!(stream.regressions, 1);
    // an equal stamp is not a step back
    assert_eq!(stream.total, 4);
}

/// a mouse that keeps moving but whose moving reports never sit side by side has no rate to
/// show, and an older burst must not stand in for it
#[test]
fn an_older_burst_never_stands_in_for_the_latest_movement() {
    let mut stream = Stream::new(u64::MAX);
    for i in 0..1000_u64 {
        stream.push_decoded(1_000_000 + i * 1000, Some((1, 0)));
    }
    // two seconds of moving reports with an empty poll between each
    for i in 0..2000_u64 {
        let moved = if i % 2 == 0 { (1, 0) } else { (0, 0) };
        stream.push_decoded(3_000_000 + i * 1000, Some(moved));
    }
    assert_eq!(all(&stream).movement_rate_hz(Some(1_000_000)), 0.0);
    assert_eq!(all(&stream).movement_rate_hz(None), 1000.0);
}

#[test]
fn a_paused_view_keeps_its_samples_past_the_live_history() {
    let report = |t_us| crate::capture::pcap::Report {
        t_us,
        bus: 1,
        device: 2,
        endpoint: 0x81,
        len: 1,
        data: vec![0],
    };
    let key = Key {
        bus: 1,
        device: 2,
        endpoint: 0x81,
    };
    let mut bus = Bus::live();
    bus.push(&report(0));
    bus.pin(Some(1_000_000));
    // a range picked inside the paused one keeps the wider one
    bus.pin(Some(3_000_000));
    for t in (1_000_000..=20_000_000).step_by(1_000_000) {
        bus.push(&report(t));
    }
    assert_eq!(bus.streams[&key].first_us(), Some(1_000_000));
    // resumed, the history slides again
    bus.pin(None);
    bus.push(&report(21_000_000));
    assert_eq!(bus.streams[&key].first_us(), Some(21_000_000 - HISTORY_US));
}
