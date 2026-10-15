use super::*;
#[test]
fn lower_envelopes_preserve_drift_without_claiming_known_transport_delay() {
    let mut arrivals = Arrivals {
        frequency: 1_000_000,
        ..Default::default()
    };
    for second in 0..20u64 {
        for delay in [1000, 5000, 20_000] {
            let time = second * 1_000_000;
            arrivals
                .push(time, (time + second * 20 + delay) as i64)
                .unwrap();
        }
    }
    let estimate = arrivals.estimate().unwrap();
    assert!((estimate.observed_rate_ppm - 20.0).abs() < 1e-6);
    assert!(!estimate.absolute_delay_known && !estimate.applied_to_scores);
}
#[test]
fn insufficient_evidence_and_resets_are_explicit_and_storage_is_bounded() {
    let mut arrivals = Arrivals {
        frequency: 1_000_000,
        ..Default::default()
    };
    arrivals.push(100, 200).unwrap();
    assert!(arrivals.estimate().is_none());
    assert!(arrivals.push(99, 201).is_err());
    for i in 1..1000 {
        arrivals
            .push(i * 1_000_000, i as i64 * 1_000_000 + 200)
            .unwrap();
    }
    assert_eq!(arrivals.envelopes.len(), 256);
}
