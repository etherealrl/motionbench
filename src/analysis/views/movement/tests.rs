use super::*;
use crate::signal::stats::Stream;

fn moving(reports: &[(u64, i32, i32)]) -> (Stream, Movement) {
    let mut stream = Stream::new(u64::MAX);
    for &(t, dx, dy) in reports {
        stream.push_decoded(t, Some((dx, dy)));
    }
    let view = stream.view(0, u64::MAX);
    let movement = movement(&view, &Moving::of(&view));
    (stream, movement)
}

/// reports that moved, binned by their counts on each axis, 0 to 10 and then more; a still
/// report is not counted
#[test]
fn counts_per_report_bin_each_axis() {
    let (_, movement) = moving(&[
        (0, 1, 0),
        (1000, -3, 2),
        (2000, 0, 0),
        (3000, 10, -11),
        (4000, 40, 0),
    ]);
    assert_eq!(movement.dx, [0, 1, 0, 1, 0, 0, 0, 0, 0, 0, 1, 1]);
    assert_eq!(movement.dy, [2, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
}

/// a smoothed stream repeats its counts from one report to the next, a quantized one flips
/// between two values; the lag 1 correlation tells them apart and needs both sides to vary
#[test]
fn the_lag_1_correlation_shows_smoothing() {
    let ramp: Vec<_> = (0..40).map(|i| (i * 1000, 1 + i as i32 / 4, 1)).collect();
    let (_, smooth) = moving(&ramp);
    assert!(smooth.lag1_dx.unwrap() > 0.9);
    assert_eq!(smooth.lag1_dy, None);
    let flip: Vec<_> = (0..40).map(|i| (i * 1000, 1 + (i % 2) as i32, 1)).collect();
    let (_, flipping) = moving(&flip);
    assert!((flipping.lag1_dx.unwrap() + 1.0).abs() < 1e-9);
}

/// an interval goes in the bin of its neighbours' speed: one count a period at 1 kHz is one
/// count a millisecond, and a 2 ms gap there shows as that bin's longest
#[test]
fn intervals_are_binned_by_the_neighbours_speed() {
    let mut reports: Vec<_> = (0..20).map(|i| (i * 1000, 1, 0)).collect();
    reports.push((21_000, 2, 0));
    reports.extend((22..30).map(|i| (i * 1000, 1, 0)));
    reports.extend((30..40).map(|i| (i * 1000, 4, 0)));
    let (_, movement) = moving(&reports);
    assert_eq!(movement.speed.len(), 9);
    let at = |from: f64| {
        movement
            .speed
            .iter()
            .find(|s| s.from == Some(from))
            .unwrap()
    };
    let one = at(1.0);
    assert_eq!(
        (one.to, one.median_us, one.longest_us),
        (Some(2.0), Some(1000.0), Some(2000))
    );
    let four = at(4.0);
    // under 20 intervals a speed shows no median or longest
    assert_eq!(
        (four.to, four.intervals, four.median_us, four.longest_us),
        (Some(8.0), 9, None, None)
    );
    assert_eq!(one.intervals + at(2.0).intervals + four.intervals, 38);
    assert_eq!(movement.speed[0].from, None);
    assert_eq!(movement.speed[8].to, None);
}

/// the counts line breaks only where no report came for over 50 ms, however long the view, and
/// its times count from the view's start
#[test]
fn the_preview_breaks_at_real_pauses_only() {
    let mut stream = Stream::new(u64::MAX);
    for i in 0..120_000u64 {
        stream.push_decoded(10_000_000 + i * 1000, Some((1, 0)));
    }
    stream.push_decoded(10_000_000 + 119_999_000 + 80_000, Some((1, 0)));
    let view = stream.view(10_000_000, u64::MAX);
    let movement = movement(&view, &Moving::of(&view));
    assert_eq!(movement.pauses, [[119.999, 120.079]]);
    assert_eq!(movement.motion[0][0], 0.0);
    assert!(movement.motion.len() > 1000);
}

/// a handful of pairs says nothing about smoothing, so the correlation waits for 20
#[test]
fn the_lag_1_correlation_needs_20_pairs() {
    let few: Vec<_> = (0..20)
        .map(|i| (i * 1000, 1 + i as i32, 1 + (i % 2) as i32))
        .collect();
    let (_, movement) = moving(&few);
    assert_eq!((movement.lag1_pairs, movement.lag1_dx), (19, None));
    let (_, enough) = moving(&[few.clone(), vec![(20_000, 21, 1)]].concat());
    assert_eq!(enough.lag1_pairs, 20);
    assert!(enough.lag1_dx.is_some());
}
