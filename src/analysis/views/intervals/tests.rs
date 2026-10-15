use super::*;
use crate::signal::stats::Stream;

fn stream(reports: &[(u64, Option<(i32, i32)>)]) -> Stream {
    let mut stream = Stream::new(u64::MAX);
    for &(t, motion) in reports {
        stream.push_decoded(t, motion);
    }
    stream
}

/// a moving interval needs movement on both sides and at most 50 ms between them; a still
/// report, an undecoded one and a pause each break it, and every other gap is left out
#[test]
fn a_moving_interval_is_two_back_to_back_moving_reports() {
    let s = stream(&[
        (0, Some((1, 0))),
        (1000, Some((0, -1))),
        (2000, Some((0, 0))),
        (3000, Some((1, 1))),
        (4000, None),
        (5000, Some((1, 0))),
        (6000, Some((2, 0))),
        (56_000, Some((1, 0))),
        (106_001, Some((1, 0))),
    ]);
    let moving = Moving::of(&s.view(0, u64::MAX));
    assert_eq!(moving.gaps().collect::<Vec<_>>(), [1000, 1000, 50_000]);
    assert_eq!(moving.left_out(), 5);
    assert_eq!(moving.period_us, Some(1000.0));
}

#[test]
fn nothing_moves_without_decoded_axes() {
    let s = stream(&[(0, None), (1000, None), (2000, None)]);
    let moving = Moving::of(&s.view(0, u64::MAX));
    assert!(moving.ends.is_empty());
    assert_eq!(moving.period_us, None);
    assert_eq!(moving.left_out(), 2);
}

#[test]
fn groups_split_at_three_quarters_and_the_halves() {
    let at = |ratio: f64| group(ratio * 1000.0, 1000.0);
    assert_eq!(
        [0.0, 0.74, 0.75, 1.49, 1.5, 2.49, 2.5, 3.49, 3.5, 40.0].map(at),
        [0, 0, 1, 1, 2, 2, 3, 3, 4, 4]
    );
}

#[test]
fn the_median_of_an_even_count_is_the_mean_of_the_middle_two() {
    assert_eq!(median(&mut []), None);
    assert_eq!(median(&mut [3.0, 1.0, 2.0]), Some(2.0));
    assert_eq!(median(&mut [4.0, 1.0, 3.0, 2.0]), Some(2.5));
    assert_eq!(
        nearest_rank(&mut [5.0, 1.0, 4.0, 2.0, 3.0], 0.01),
        Some(1.0)
    );
    assert_eq!(
        nearest_rank(&mut [5.0, 1.0, 4.0, 2.0, 3.0], 0.99),
        Some(5.0)
    );
    assert_eq!(nearest_rank(&mut [5.0, 1.0, 4.0, 2.0, 3.0], 0.5), Some(3.0));
}

/// a moving interval's pace comes from its neighbours, each over its own interval, never from
/// its own report; with no moving neighbour it has none
#[test]
fn the_pace_comes_from_the_neighbours_only() {
    let s = stream(&[
        (0, Some((2, 0))),
        (1000, Some((8, 0))),
        (3000, Some((4, 0))),
        (4000, Some((6, 0))),
        (100_000, Some((1, 0))),
        (101_000, Some((3, 0))),
    ]);
    let moving = Moving::of(&s.view(0, u64::MAX));
    assert_eq!(moving.period_us, Some(1000.0));
    // the first interval only has the next neighbour, 4 counts over 2 periods; the second has 8
    // over 1 and 6 over 1; the third has 4 over 2 alone; the last stands alone
    assert_eq!(moving.paces(), [Some(2.0), Some(7.0), Some(2.0), None]);
}
