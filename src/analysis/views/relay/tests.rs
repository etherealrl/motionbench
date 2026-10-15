use super::*;

fn stream(reports: &[(u64, i32, i32)]) -> Stream {
    let mut stream = Stream::new(u64::MAX);
    for &(t, dx, dy) in reports {
        stream.push_decoded(t, Some((dx, dy)));
    }
    stream
}

fn relayed(boxed: &[(u64, i32, i32)], wire: &[(u64, i32, i32)]) -> Relay {
    let (boxed, wire) = (stream(boxed), stream(wire));
    relay(&wire.view(1_000_000, 1_400_000), &boxed, Some(1000.0))
}

/// every box report reaches the wire with its own counts a little later: everything pairs,
/// nothing is left over, and only the offset's spread around its median shows
#[test]
fn a_lossless_relay_conserves_counts_and_pairs_every_report() {
    let boxed: Vec<_> = (0..300)
        .map(|i| (1_000_000 + i * 1000, 1 + (i % 3) as i32, -1))
        .collect();
    let wire: Vec<_> = boxed
        .iter()
        .enumerate()
        .map(|(i, &(t, dx, dy))| (t + 200 + [0, 4, 8][i % 3], dx, dy))
        .collect();
    let relay = relayed(&boxed, &wire);
    assert_eq!(relay.box_counts, relay.wire_counts);
    assert_eq!(
        (relay.matched, relay.box_only, relay.wire_only),
        (300, 0, 0)
    );
    assert_eq!(relay.conserved.last().unwrap()[1..], [0.0, 0.0]);
    assert_eq!(relay.windows.len(), 4);
    assert!(relay.windows.iter().all(|w| *w == [0, 0]));
    assert_eq!(
        (relay.offset.p01_us, relay.offset.p99_us),
        (Some(-4.0), Some(4.0))
    );
    let middle = relay.offset.bins.len() / 2;
    assert_eq!(
        relay.offset.bins[middle - 4..=middle + 4],
        [100, 0, 0, 0, 100, 0, 0, 0, 100]
    );
}

/// a report that never reaches the wire leaves its counts behind from that moment on, in its
/// window, and stays alone
#[test]
fn a_dropped_report_shows_as_counts_left_behind() {
    let boxed: Vec<_> = (0..300).map(|i| (1_000_000 + i * 1000, 2, 0)).collect();
    let wire: Vec<_> = boxed
        .iter()
        .filter(|r| r.0 != 1_150_000)
        .map(|&(t, dx, dy)| (t + 300, dx, dy))
        .collect();
    let relay = relayed(&boxed, &wire);
    assert_eq!(
        (relay.matched, relay.box_only, relay.wire_only),
        (299, 1, 0)
    );
    assert_eq!(relay.box_counts[0] - relay.wire_counts[0], 2);
    assert_eq!(relay.windows[1], [2, 0]);
    assert_eq!(relay.conserved.last().unwrap()[1], 2.0);
    // before the drop the running difference stays at zero, the box's side moved by the median
    // offset so a count in flight is not counted as lost
    let before = relay.conserved.iter().find(|c| c[0] >= 0.1).unwrap();
    assert_eq!(before[1], 0.0);
}

/// two box reports merged into one wire report keep the counts but pair with nothing
#[test]
fn merged_reports_keep_the_counts_but_do_not_pair() {
    let relay = relayed(
        &[(1_000_000, 1, 0), (1_001_000, 1, 0), (1_002_000, 3, 0)],
        &[(1_001_200, 2, 0), (1_002_200, 3, 0)],
    );
    assert_eq!(relay.box_counts, relay.wire_counts);
    assert_eq!((relay.matched, relay.box_only, relay.wire_only), (1, 2, 1));
}

/// the box's clock moves onto the wire's by the fixed shift, and a row the shift would put
/// before the clock's start is left out
#[test]
fn the_box_input_moves_onto_the_wire_clock() {
    let input = stream(&[(500, 1, 0), (5_000, 2, 0), (6_000, 3, 0)]);
    let moved = aligned(&input, 1000);
    let rows: Vec<_> = moved.view(0, u64::MAX).motion().collect();
    assert_eq!(rows, [(4_000, 2, 0), (5_000, 3, 0)]);
    let back = aligned(&input, -1000);
    assert_eq!(back.first_us(), Some(1_500));
}

/// a lossless relay of equal counts, 300 µs late, cut by the range in the middle of the stream:
/// the box's reports count by where they land, so nothing at the edges is left alone, every
/// pair is the true one, and no column shows a count in flight
#[test]
fn a_range_cut_mid_stream_keeps_a_lossless_relay_at_zero() {
    let boxed: Vec<_> = (0..1000).map(|i| (900_000 + i * 1000, 2, 0)).collect();
    let wire: Vec<_> = boxed.iter().map(|&(t, dx, dy)| (t + 300, dx, dy)).collect();
    let relay = relayed(&boxed, &wire);
    assert_eq!(relay.box_counts, relay.wire_counts);
    assert_eq!((relay.box_only, relay.wire_only), (0, 0));
    assert_eq!(relay.matched, 400);
    assert!(relay.conserved.iter().all(|c| c[1] == 0.0 && c[2] == 0.0));
    assert!(relay.windows.iter().all(|w| *w == [0, 0]));
    assert_eq!(
        (relay.offset.p01_us, relay.offset.p99_us),
        (Some(0.0), Some(0.0))
    );
}

/// a pair needs the same counts on both axes
#[test]
fn a_pair_needs_both_axes_alike() {
    let relay = relayed(&[(1_100_000, 1, 1)], &[(1_100_100, 1, 2)]);
    assert_eq!((relay.matched, relay.box_only, relay.wire_only), (0, 1, 1));
}

/// a box report that lands in the next window across the 100 ms edge is counted there, beside
/// its wire report
#[test]
fn a_window_takes_the_box_report_where_it_landed() {
    let boxed: Vec<_> = (0..300)
        .map(|i| (999_500 + i * 1000, 1 + (i % 3) as i32, 0))
        .collect();
    let wire: Vec<_> = boxed.iter().map(|&(t, dx, dy)| (t + 700, dx, dy)).collect();
    let relay = relayed(&boxed, &wire);
    assert!(relay.windows.iter().all(|w| *w == [0, 0]));
}

/// at 8 kHz a pair is looked for within the shorter period: 40 µs apart pairs, 200 µs apart
/// is beyond it and pairs with nothing
#[test]
fn pairs_follow_the_period() {
    let boxed: Vec<_> = (0..2000)
        .map(|i| (1_000_000 + i * 125, 1 + (i % 3) as i32, 0))
        .collect();
    let b = stream(&boxed);
    let late = |by: u64| {
        let wire: Vec<_> = boxed.iter().map(|&(t, dx, dy)| (t + by, dx, dy)).collect();
        relay(&stream(&wire).view(1_000_000, 1_200_000), &b, Some(125.0))
    };
    let near = late(40);
    assert_eq!((near.box_only, near.wire_only), (0, 0));
    assert_eq!(
        (near.offset.p01_us, near.offset.p99_us),
        (Some(0.0), Some(0.0))
    );
    assert_eq!(late(200).matched, 0);
}

/// a pair that straddles a column's end is a count in flight, not a lost one; the column keeps
/// its smallest difference so the line stays at zero
#[test]
fn a_count_in_flight_at_a_column_end_does_not_show() {
    let boxed: Vec<_> = (0..300)
        .map(|i| (1_000_490 + i * 1000, 1 + (i % 3) as i32, 0))
        .collect();
    // half the wire reports on time and half 20 µs late: the median offset is 10 µs, so each
    // late one lands 10 µs after its box report, across the column end at every half millisecond
    let wire: Vec<_> = boxed
        .iter()
        .enumerate()
        .map(|(i, &(t, dx, dy))| (t + 20 * (i as u64 % 2), dx, dy))
        .collect();
    let relay = relayed(&boxed, &wire);
    assert_eq!((relay.box_only, relay.wire_only), (0, 0));
    assert!(relay.conserved.iter().all(|c| c[1] == 0.0 && c[2] == 0.0));
}
