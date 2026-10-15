use super::*;
use crate::analysis::views::plot::{Views, plot};
use crate::capture::session::Data;
use crate::signal::stats::{Key, Stream};

fn numbers_of(reports: &[(u64, Option<(i32, i32)>)]) -> Numbers {
    let key = Key {
        bus: 1,
        device: 2,
        endpoint: 0x81,
    };
    let mut stream = Stream::new(u64::MAX);
    for &(t, motion) in reports {
        stream.push_decoded(t, motion);
    }
    let mut data = Data::new(false);
    data.bus.streams.insert(key, stream);
    data.selected = Some(key);
    numbers(&plot(&data, None, None, false, Views::All))
}

/// a recording's figures come from the same views the tabs show: shares of its moving
/// intervals and of its ticks while moving, the counts after its gap, how alike one report is
/// to the next
#[test]
fn the_numbers_follow_the_views() {
    let dx = |t: u64| 1 + (t / 1000 % 4) as i32;
    let mut reports: Vec<_> = (0..40)
        .filter(|i| *i != 30)
        .map(|i| (i * 1000, Some((dx(i * 1000), 0))))
        .collect();
    // two bunched reports, each splitting a period into 400 and 600 µs
    reports.extend([(10_400, Some((1, 0))), (20_400, Some((1, 0)))]);
    reports.sort_by_key(|r| r.0);
    let n = numbers_of(&reports);
    assert_eq!(
        (n.rate_hz, n.period_us, n.jitter_us, n.moving),
        (Some(1000.0), Some(1000.0), Some(0.0), 40)
    );
    assert_eq!(
        (n.bunched, n.beyond_1x),
        (Some(4.0 / 40.0), Some(1.0 / 40.0))
    );
    assert_eq!(n.empty_ticks, Some(1.0 / 40.0));
    // after the 2 ms gap the report held 4 counts against neighbours moving 2 and 1 a period
    assert!((n.after_gap.unwrap() - 4.0 / 1.5).abs() < 1e-9);
    assert!(n.lag1_dx.is_some_and(|r| r.abs() < 1.0));
    assert_eq!(n.longest_moving_us, Some(2000.0));
}

/// without a decoder there is nothing moving to measure, every figure is unavailable
#[test]
fn without_a_decoder_every_figure_is_unavailable() {
    let n = numbers_of(&(0..40).map(|i| (i * 1000, None)).collect::<Vec<_>>());
    assert_eq!(n, Numbers::default());
}
