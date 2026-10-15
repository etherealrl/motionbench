use super::*;
#[test]
fn independent_permutations_rebuild_repeated_folds_for_five_and_seven_sessions() {
    for candidates in [5, 7] {
        let population = |prefix: &str, groups: usize, shift: f64| {
            let mut rows = Vec::new();
            let mut identities = Vec::new();
            for group in 0..groups {
                for i in 0..(5 + group) {
                    rows.push(vec![
                        group as f64 * 0.2 + i as f64 * 0.1 + shift,
                        (i % 3) as f64,
                    ]);
                    identities.push(format!("{prefix}-{group}"));
                }
            }
            (rows, identities)
        };
        let (real, rg) = population("native", 5, 0.0);
        let (fake, fg) = population("candidate", candidates, 0.1);
        let mut config = Config {
            permutations: 100,
            bootstrap: 0,
            ..Config::default()
        };
        let report = run(
            &real,
            &fake,
            &rg,
            &fg,
            &config,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert_eq!(report.permutation_auc.len(), 100);
        assert!(report.permutation_error.is_none());
        assert!(report.permutation_method.contains("rebuilt"));
        config.permutations = 0;
        let descriptive = run(
            &real,
            &fake,
            &rg,
            &fg,
            &config,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert_eq!(report.auc, descriptive.auc);
    }
}
#[test]
fn identical_constant_bags_and_zero_permutations() {
    let rows = vec![vec![0.0; 49]; 128];
    let groups = (0..128)
        .map(|i| format!("int:{}", i / 8))
        .collect::<Vec<_>>();
    let config = Config {
        repeats: 2,
        bootstrap: 10,
        permutations: 2,
        ..Config::default()
    };
    let report = run(
        &rows,
        &rows,
        &groups,
        &groups,
        &config,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(report.auc, 0.5);
    assert_eq!(report.auc_ci, Some([0.5, 0.5]));
    assert_eq!(report.p_value, Some(1.0));
}
#[test]
fn one_recording_per_population_cannot_claim_independence() {
    assert!(
        run(
            &vec![vec![0.0]; 2],
            &vec![vec![1.0]; 2],
            &vec!["a".into(); 2],
            &vec!["b".into(); 2],
            &Config::default(),
            &AtomicBool::new(false),
            |_| {}
        )
        .is_err()
    );
}
#[test]
fn unequal_bags_and_correlated_groups_use_grouped_folds() {
    let real = vec![vec![1.0; 3]; 65];
    let fake = vec![vec![1.0; 3]; 128];
    let rg = (0..65)
        .map(|i| format!("trial-{}", i / 8))
        .collect::<Vec<_>>();
    let fg = (0..128)
        .map(|i| format!("trial-{}", i / 8))
        .collect::<Vec<_>>();
    let config = Config {
        repeats: 1,
        bootstrap: 3,
        permutations: 0,
        ..Config::default()
    };
    let report = run(
        &real,
        &fake,
        &rg,
        &fg,
        &config,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(report.auc, 0.5);
    assert_eq!(report.n_groups, 16);
    assert!(report.permutation_auc.is_empty());
    assert!(report.p_value.is_none());
    assert_eq!(
        run(
            &real,
            &fake,
            &rg,
            &fg,
            &config,
            &AtomicBool::new(true),
            |_| {}
        )
        .err()
        .unwrap(),
        crate::analysis::error::Error::Cancelled
    );
}

#[test]
fn a_cancel_asked_for_during_the_permutations_stops_them() {
    let real = vec![vec![1.0; 3]; 64];
    let fake = vec![vec![2.0; 3]; 64];
    let groups = (0..64)
        .map(|i| format!("trial-{}", i / 8))
        .collect::<Vec<_>>();
    let config = Config {
        repeats: 1,
        bootstrap: 0,
        permutations: 50,
        ..Config::default()
    };
    // the progress callback only runs once the permutations have started, so the first one
    // asks to stop and the next check has to see it
    let stop = AtomicBool::new(false);
    let result = run(&real, &fake, &groups, &groups, &config, &stop, |_| {
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    assert_eq!(result.err(), Some(crate::analysis::error::Error::Cancelled));
}

/// every number a report carries, on the fixture's windows, pinned so reorganising the numerics
/// can't move one unnoticed
#[test]
fn the_fixture_reports_stay_put() {
    use crate::analysis::texture::features;
    let ticks = crate::testkit::fixtures::fixture_ticks();
    let rows: Vec<Vec<f64>> = ticks
        .chunks_exact(25)
        .map(|w| features(w).to_vec())
        .collect();
    let half = rows.len() / 2;
    let (real, fake) = (&rows[..half], &rows[half..half * 2]);
    let config = Config {
        repeats: 2,
        bootstrap: 40,
        permutations: 12,
        ..Config::default()
    };
    let report = |real_groups: &[String], fake_groups: &[String]| {
        let report = run(
            real,
            fake,
            real_groups,
            fake_groups,
            &config,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        crate::testkit::fixtures::digest(&serde_json::to_string(&report).unwrap())
    };
    let named = |side: &str, every: usize| -> Vec<String> {
        (0..half).map(|i| format!("{side}-{}", i / every)).collect()
    };
    let paired: Vec<String> = (0..half).map(|i| format!("int:{i}")).collect();
    assert_eq!(
        [
            report(&named("real", 1), &named("fake", 1)),
            report(&paired, &paired),
            report(&named("real", 4), &named("fake", 4)),
        ],
        ["c40cbf62dfb0d841", "b6126ca4860c0d0c", "25e483ef2ea5be26"]
    );
}

/// one repeat, no bootstrap: the production judge on the reference's own c2st_auc cases
fn reference_case(
    real: &[Vec<f64>],
    fake: &[Vec<f64>],
    groups: (&[String], &[String]),
    permutations: usize,
) -> Report {
    let config = Config {
        repeats: 1,
        bootstrap: 0,
        permutations,
        ..Config::default()
    };
    run(
        real,
        fake,
        groups.0,
        groups.1,
        &config,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap()
}

fn windows(width: usize) -> Vec<Vec<f64>> {
    crate::testkit::fixtures::fixture_ticks()
        .chunks_exact(width)
        .map(|w| crate::analysis::texture::features(w).to_vec())
        .collect()
}

fn every_window_alone(side: &str, count: usize) -> Vec<String> {
    (0..count).map(|i| format!("__{side}_sample_{i}")).collect()
}

/// c2st_auc(real, fake, folds=5, seed=7) over the fixture's 50 tick windows, 8 a side, then
/// 100 tick ones, 4 a side, every window its own group
#[test]
fn unpaired_windows_match_the_reference_auc() {
    let rows = windows(50);
    let groups = (
        &every_window_alone("real", 8)[..],
        &every_window_alone("fake", 8)[..],
    );
    let report = reference_case(&rows[..8], &rows[8..16], groups, 0);
    assert_eq!(report.fold_auc, vec![vec![0.0, 0.75, 1.0, 1.0, 1.0]]);
    assert!((report.auc - 0.75).abs() < 1e-12);

    let rows = windows(100);
    let groups = (
        &every_window_alone("real", 4)[..],
        &every_window_alone("fake", 4)[..],
    );
    let report = reference_case(&rows[..4], &rows[4..8], groups, 0);
    assert_eq!(report.fold_auc[0].len(), 4);
    assert!(report.auc.abs() < 1e-12);
}

/// groups=range(8): window i of each side is one event seen twice and stays in one fold. a bag
/// against itself is a coin flip in every fold, and no shuffle can do worse
#[test]
fn paired_windows_match_the_reference_and_read_a_twin_as_a_coin_flip() {
    let rows = windows(50);
    let pairs: Vec<String> = (0..8).map(|i| format!("int:{i}")).collect();
    let report = reference_case(&rows[..8], &rows[8..16], (&pairs, &pairs), 0);
    assert_eq!(report.fold_auc, vec![vec![0.5, 1.0, 0.0, 1.0, 1.0]]);
    assert!((report.auc - 0.571428571429).abs() < 1e-9);

    let same = reference_case(&rows[..8], &rows[..8], (&pairs, &pairs), 20);
    assert_eq!(same.fold_auc, vec![vec![0.5; 5]]);
    assert_eq!(same.auc, 0.5);
    assert_eq!(same.p_value, Some(1.0));
}

/// two recordings a side: six ways to split them, the real one and its swap as far from chance as
/// each other, so p can't go under a third. each fold holds one recording a side, so the bootstrap
/// has nothing to draw and there's no interval
#[test]
fn two_recordings_a_side_say_how_low_p_can_go_and_give_no_interval() {
    let rows = |offset: f64| -> Vec<Vec<f64>> {
        (0..40)
            .map(|i| vec![offset + (i % 7) as f64, (i % 3) as f64])
            .collect()
    };
    let groups =
        |side: &str| -> Vec<String> { (0..40).map(|i| format!("{side}-{}", i / 20)).collect() };
    let config = Config {
        repeats: 1,
        bootstrap: 20,
        permutations: 30,
        ..Config::default()
    };
    let report = run(
        &rows(0.0),
        &rows(5.0),
        &groups("real"),
        &groups("fake"),
        &config,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert_eq!(report.n_groups, 4);
    assert!((report.smallest_p.unwrap() - 1.0 / 3.0).abs() < 1e-12);
    assert_eq!(report.auc_ci, None);
}

#[test]
fn a_value_too_big_for_f32_is_refused_rather_than_fitted_as_infinity() {
    let mut rows = vec![vec![1.0; 19]; 16];
    rows[3][2] = 1e39;
    let groups = (0..16).map(|i| format!("r{}", i / 4)).collect::<Vec<_>>();
    let refused = run(
        &rows,
        &rows,
        &groups,
        &groups,
        &Config::default(),
        &AtomicBool::new(false),
        |_| {},
    );
    assert!(refused.is_err());
}
