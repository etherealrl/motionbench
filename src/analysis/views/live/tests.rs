use super::*;

fn plot(moving: usize, decoded: bool, band: [u32; 2]) -> Plot {
    let mut plot = Plot {
        reports: (moving + 1).max(30),
        movement_available: decoded,
        scale: Scale {
            band,
            jitter_us: 2.0,
        },
        ..Plot::default()
    };
    plot.readouts.moving = moving;
    plot
}

/// every frame computes the tab on show
#[test]
fn a_frame_computes_the_shown_tab() {
    let mut live = Live::default();
    assert_eq!(live.views(), Views::Tab(Tab::Timing));
    live.show(Tab::Movement);
    assert_eq!(live.views(), Views::Tab(Tab::Movement));
}

/// the axes hold still only once enough moving intervals fixed them, so a quiet start cannot
/// leave the jitter on a span from a handful of intervals; without a decoder the band is fixed
/// from the reports
#[test]
fn the_axes_settle_on_enough_moving_intervals() {
    let mut live = Live::default();
    live.settle(&plot(SETTLED_INTERVALS - 1, true, [1, 2]), 0);
    assert_eq!(live.scale(), None);
    live.settle(&plot(SETTLED_INTERVALS, true, [900, 1100]), 0);
    assert_eq!(live.scale().unwrap().band, [900, 1100]);
    live.settle(&plot(5000, true, [1, 2]), 0);
    assert_eq!(live.scale().unwrap().band, [900, 1100]);
    live.reset();
    assert_eq!(live.scale(), None);
    live.settle(&plot(0, false, [990, 1010]), live.resets());
    assert_eq!(live.scale().unwrap().band, [990, 1010]);
}

/// a plot computed before the view started again, for another range or mouse, never fixes the
/// new view's axes
#[test]
fn a_plot_from_before_a_reset_fixes_nothing() {
    let mut live = Live::default();
    let computed = live.resets();
    live.reset();
    live.settle(&plot(SETTLED_INTERVALS, true, [900, 1100]), computed);
    assert_eq!(live.scale(), None);
}
