use super::*;

/// every number below was printed by numpy 1.26 and abcurves/judges.py on the same inputs
#[test]
fn seed_sequence_matches_numpy() {
    let seed = SeedSequence::new(&[7, 0, 0xC2A5]).generate_state(1)[0];
    assert_eq!(seed, 3044039776);
}

#[test]
fn pcg64_matches_numpy() {
    let mut rng = Pcg64::from_seed(3044039776);
    let raw = [
        rng.next_u64(),
        rng.next_u64(),
        rng.next_u64(),
        rng.next_u64(),
    ];
    assert_eq!(
        raw,
        [
            1619817036623076085,
            12791220459104529907,
            13414998761158716167,
            5480481911489289392
        ]
    );
}

#[test]
fn permutation_matches_numpy() {
    assert_eq!(
        Pcg64::from_seed(3044039776).permutation(16),
        vec![10, 9, 1, 11, 4, 13, 6, 15, 8, 2, 0, 7, 12, 14, 3, 5]
    );
    assert_eq!(
        Pcg64::from_seed(3044039776).permutation(8),
        vec![1, 2, 0, 7, 4, 6, 3, 5]
    );
}

#[test]
fn reductions_match_numpy() {
    // a bare sum of nothing is -0, which an export would print as -0.0
    assert_eq!(mean(&[]).to_bits(), 0.0f64.to_bits());
    assert_eq!(mean(&[1.0, 2.0, 6.0]), 3.0);
    assert_eq!(std(&[1.0, 2.0, 6.0]), (14.0f64 / 3.0).sqrt());
    assert_eq!(linear_quantile(&[1.0, 2.0, 4.0], 0.5), 2.0);
    assert_eq!(linear_quantile(&[1.0, 2.0, 4.0], 0.75), 3.0);
    assert_eq!(linear_quantile(&[], 0.5), 0.0);
    assert_eq!(sorted(&[3.0, -1.0, 2.0]), [-1.0, 2.0, 3.0]);
}
