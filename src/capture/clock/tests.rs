use super::*;
#[test]
fn anchor_brackets_and_large_epochs_keep_integer_precision() {
    let anchor = Anchor {
        qpc_before: 10_000,
        qpc_after: 10_020,
        utc_us: 1_790_897_533_325_891,
    };
    assert_eq!(
        anchor.epoch_us(10_110, 1_000_000),
        Some(anchor.utc_us + 100)
    );
    assert_eq!(anchor.uncertainty_us(1_000_000), 11.0);
    assert_eq!(anchor.epoch_us(9910, 1_000_000), Some(anchor.utc_us - 100));
}
#[test]
fn mapping_detects_forward_backward_steps_and_accepts_bracket_uncertainty() {
    for step in [-5000i64, 5000] {
        let mut monitor = Monitor::default();
        let mut a = Anchor {
            qpc_before: 1_000_000,
            qpc_after: 1_000_002,
            utc_us: 10_000_000,
        };
        monitor.observe(&a, 1_000_000).unwrap();
        a.qpc_before += 1_000_000;
        a.qpc_after += 1_000_000;
        a.utc_us = (a.utc_us as i64 + 1_000_000 + step) as u64;
        assert!(monitor.observe(&a, 1_000_000).is_err());
        assert_eq!(monitor.summary.discontinuities, 1);
    }
    let mut monitor = Monitor::default();
    monitor
        .observe(
            &Anchor {
                qpc_before: 0,
                qpc_after: 2000,
                utc_us: 100_000,
            },
            1_000_000,
        )
        .unwrap();
    monitor
        .observe(
            &Anchor {
                qpc_before: 100_000,
                qpc_after: 102_000,
                utc_us: 202_000,
            },
            1_000_000,
        )
        .unwrap();
}
#[test]
fn native_clock_is_monotonic_and_uses_a_positive_frequency() {
    let a = Anchor::take();
    let b = Anchor::take();
    assert!(frequency() > 0);
    assert!(b.qpc_before >= a.qpc_after);
    assert!(elapsed_us(a.midpoint(), b.midpoint()).is_some());
}
