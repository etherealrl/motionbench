use super::*;
use crate::signal::stats::Stream;

/// moving reports at these times, each with `counts` along x
fn moving(reports: &[(u64, i32)]) -> Moving {
    let mut stream = Stream::new(u64::MAX);
    for &(t, counts) in reports {
        stream.push_decoded(t, Some((counts, 0)));
    }
    Moving::of(&stream.view(0, u64::MAX))
}

fn steady(gaps: impl IntoIterator<Item = u64>) -> Moving {
    let mut t = 1_000_000;
    let mut reports = vec![(t, 1)];
    for gap in gaps {
        t += gap;
        reports.push((t, 1));
    }
    moving(&reports)
}

/// 1x intervals 10 µs either side of a 1 ms period: the deviation is binned in whole
/// microseconds with the period in the middle bin, its median distance is 10 µs, and the 2x
/// interval is left out of it
#[test]
fn jitter_bins_the_1x_deviation_from_the_period() {
    let moving = steady((0..300).map(|i| [990, 1000, 1010][i % 3]).chain([2000]));
    assert_eq!(moving.period_us, Some(1000.0));
    let deviations = deviations(&moving);
    assert_eq!(jitter_span(&deviations), 10.0);
    let jitter = timing(&moving, 10.0, false).jitter;
    assert_eq!(jitter.bins.len(), 21);
    assert_eq!(
        (jitter.bins[0], jitter.bins[10], jitter.bins[20]),
        (100, 100, 100)
    );
    assert_eq!(jitter.outside, 0);
    assert_eq!(jitter.mad_us, Some(10.0));
    assert_eq!((jitter.p01_us, jitter.p99_us), (Some(-10.0), Some(10.0)));
    assert_eq!((jitter.intervals, jitter.left_out), (300, 1));
    // a narrower shared span puts the rest outside
    let narrow = timing(&moving, 5.0, false).jitter;
    assert_eq!(
        (narrow.bins.iter().sum::<u32>(), narrow.outside),
        (100, 200)
    );
}

/// early and late land on their own sides of the period
#[test]
fn jitter_keeps_the_sign_of_a_deviation() {
    let moving = steady((0..301).map(|i| match i % 6 {
        0 => 990,
        1 | 2 => 1003,
        _ => 1000,
    }));
    assert_eq!(moving.period_us, Some(1000.0));
    let jitter = timing(&moving, 10.0, false).jitter;
    assert_eq!(
        (jitter.bins[0], jitter.bins[10], jitter.bins[13]),
        (51, 150, 100)
    );
    assert_eq!((jitter.p01_us, jitter.p99_us), (Some(-10.0), Some(3.0)));
}

/// a wide spread gets wider bins, still whole microseconds
#[test]
fn a_wide_jitter_span_keeps_whole_microsecond_bins() {
    assert_eq!(jitter_span(&[-61.0, 0.0, 44.0]), 63.0);
    assert_eq!(bin_width(63.0), 3.0);
    assert_eq!(jitter_span(&[0.5]), 2.0);
    assert_eq!(jitter_span(&[]), 2.0);
}

/// a bunched report counts as bunched, not as 1x, and the long tail is a share of the moving
/// intervals
#[test]
fn groups_and_the_long_tail_count_moving_intervals() {
    let moving = steady([1000, 1000, 1000, 400, 1000, 2000, 3000, 9000]);
    let timing = timing(&moving, 10.0, false);
    assert_eq!(timing.groups, [1, 4, 1, 1, 1]);
    let longer = |threshold: f64| {
        timing
            .longer
            .iter()
            .find(|p| p[0] >= threshold)
            .map(|p| p[1])
            .unwrap()
    };
    let (first, last) = (timing.longer[0], timing.longer[60]);
    assert!((first[0] - 50.0).abs() < 1e-9 && (last[0] - 50_000.0).abs() < 1e-6);
    assert_eq!(first[1], 1.0);
    assert_eq!(last[1], 0.0);
    assert_eq!(longer(1100.0), 3.0 / 8.0);
    assert_eq!(longer(9100.0), 0.0);
}

/// gaps where the neighbours move at different paces: the readout is the counts held over the
/// counts their neighbours predict, summed over every gap, and the axis spans the p99 pace
#[test]
fn counts_after_gaps_at_two_paces_add_up() {
    let mut reports: Vec<(u64, i32)> = (0..40).map(|i| (i * 1000, 1)).collect();
    reports.push((41_000, 2));
    reports.extend((42..60).map(|i| (i * 1000, 1)));
    reports.extend((60..70).map(|i| (i * 1000, 2)));
    reports.push((72_000, 3 * 2));
    reports.extend((73..80).map(|i| (i * 1000, 2)));
    let density = timing(&moving(&reports), 10.0, false).density;
    let after = density.after_gap.unwrap();
    // 2 counts against a pace of 1 after 2 periods, 6 against 2 after 3 periods
    assert_eq!(
        (after.periods, after.ratio, after.intervals),
        (2.5, 8.0 / 3.0, 2)
    );
    assert_eq!(density.x_max, 2.0);
    assert_eq!((density.intervals, density.alone), (76, 0));
}

/// after a 2 ms gap the report holds two counts where its neighbours move one per period:
/// twice what one report predicts, over a gap of two periods
#[test]
fn counts_after_a_gap_are_set_against_the_neighbours_pace() {
    let mut reports: Vec<(u64, i32)> = (0..20).map(|i| (i * 1000, 1)).collect();
    reports.push((21_000, 2));
    reports.extend((22..40).map(|i| (i * 1000, 1)));
    let density = timing(&moving(&reports), 10.0, false).density;
    let after = density.after_gap.unwrap();
    assert_eq!((after.periods, after.ratio, after.intervals), (2.0, 2.0, 1));
    assert_eq!(density.x_max, 1.0);
    assert_eq!(density.cells.len(), 64 * 48);
    // one count per period lands in the last column, at one and at two periods
    assert_eq!(
        density.cells[12 * 64 + 63] + 1,
        density.cells.iter().sum::<u32>()
    );
    assert_eq!(density.cells[24 * 64 + 63], 1);
    assert_eq!(density.outside, 0);
}

/// the judge sums reports into 1 ms ticks; while moving, a tick can hold none, one or more
#[test]
fn ticks_while_moving_count_the_reports_each_holds() {
    let moving = moving(&[
        (0, 1),
        (1000, 1),
        (2000, 1),
        (3000, 1),
        (3500, 1),
        (5000, 1),
        (6000, 1),
        (6200, 1),
        (6400, 1),
        (6600, 1),
        // a pause, its ticks are not while moving
        (80_000, 1),
    ]);
    assert_eq!(timing(&moving, 10.0, false).ticks, [1, 4, 1, 1]);
}

/// a pause of exactly 50 ms keeps a burst whole; 20 intervals show a rate, 19 do not; past
/// 500 bursts the rest are counted
#[test]
fn bursts_keep_50_ms_need_20_intervals_and_list_500() {
    let navigate = |reports: &[u64]| {
        let mut stream = Stream::new(u64::MAX);
        for &t in reports {
            stream.push_decoded(t, Some((1, 0)));
        }
        timing(&Moving::of(&stream.view(0, u64::MAX)), 10.0, true)
            .bursts
            .unwrap()
    };
    let twenty: Vec<u64> = (0..21).map(|i| 1_000_000 + i * 1000).collect();
    let nineteen: Vec<u64> = (0..20).map(|i| 2_000_000 + i * 1000).collect();
    let bursts = navigate(&[twenty, nineteen, vec![3_000_000, 3_050_000]].concat());
    let rates: Vec<_> = bursts
        .rows
        .iter()
        .map(|b| (b.intervals, b.rate_hz))
        .collect();
    assert_eq!(rates, [(20, Some(1000.0)), (19, None), (1, None)]);
    let many: Vec<u64> = (0..502)
        .flat_map(|i| [i * 100_000, i * 100_000 + 1000])
        .collect();
    let bursts = navigate(&many);
    assert_eq!((bursts.rows.len(), bursts.more), (500, 2));
}

/// bursts split at pauses over 50 ms, hold only their own intervals, and show a rate only
/// from 20 intervals; a lone moving report is no burst
#[test]
fn bursts_split_at_pauses_and_hide_a_rate_from_few_intervals() {
    let mut stream = Stream::new(u64::MAX);
    for i in 0..30 {
        stream.push_decoded(100_000 + i * 1000, Some((1, 0)));
    }
    stream.push_decoded(200_000, Some((1, 0)));
    stream.push_decoded(201_000, Some((0, 0)));
    stream.push_decoded(202_000, Some((0, 1)));
    stream.push_decoded(203_500, Some((0, 1)));
    stream.push_decoded(400_000, Some((1, 0)));
    let moving = Moving::of(&stream.view(0, u64::MAX));
    assert!(timing(&moving, 10.0, false).bursts.is_none());
    let bursts = timing(&moving, 10.0, true).bursts.unwrap();
    assert_eq!(bursts.more, 0);
    let [a, b] = &bursts.rows[..] else {
        panic!("two bursts")
    };
    assert_eq!(
        (a.from_us, a.to_us, a.reports, a.intervals),
        (100_000, 129_000, 30, 29)
    );
    assert_eq!(
        (a.median_us, a.longest_us, a.rate_hz),
        (Some(1000.0), 1000, Some(1000.0))
    );
    assert_eq!(
        (b.from_us, b.to_us, b.reports, b.intervals),
        (200_000, 203_500, 4, 1)
    );
    assert_eq!(
        (b.median_us, b.longest_us, b.rate_hz),
        (Some(1500.0), 1500, None)
    );
}
