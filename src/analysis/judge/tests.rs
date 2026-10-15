use super::*;

/// what `SeedSequence([7, 0, 0xC2A5])` gives, the reference's first repeat
const REPEAT_SEED: u32 = 3044039776;

fn unpaired_names(real: usize, fake: usize) -> Vec<String> {
    (0..real)
        .map(|i| format!("__real_sample_{i}"))
        .chain((0..fake).map(|i| format!("__fake_sample_{i}")))
        .collect()
}

#[test]
fn folds_match_the_reference_split() {
    let labels: Vec<bool> = (0..16).map(|i| i >= 8).collect();

    assert_eq!(
        folds(&unpaired_names(8, 8), &labels, 5, REPEAT_SEED),
        vec![
            vec![1, 14, 15],
            vec![4, 5, 9, 13],
            vec![0, 3, 8],
            vec![2, 6, 10],
            vec![7, 11, 12]
        ]
    );
}

/// groups=range(8): window i and its twin i+8 share a group and land in one fold together
#[test]
fn paired_folds_match_the_reference_split() {
    let labels: Vec<bool> = (0..16).map(|i| i >= 8).collect();
    let names: Vec<String> = (0..8).chain(0..8).map(|i| format!("int:{i}")).collect();

    assert_eq!(
        folds(&names, &labels, 5, REPEAT_SEED),
        vec![
            vec![2, 3, 10, 11],
            vec![0, 5, 8, 13],
            vec![6, 7, 14, 15],
            vec![1, 9],
            vec![4, 12]
        ]
    );
}

#[test]
fn ranks_ties_by_their_average() {
    assert_eq!(
        auc(&[0.1, 0.9, 0.5, 0.5], &[false, true, false, true]),
        0.875
    );
    assert_eq!(auc(&[1.0, 2.0, 3.0], &[false, false, true]), 1.0);
    assert_eq!(auc(&[1.0, 1.0], &[false, true]), 0.5);
}

#[test]
fn the_solver_inverts_a_known_system() {
    let n: usize = 20;
    let a: Vec<Vec<f64>> = (0..n)
        .map(|i| {
            (0..n)
                .map(|k| match i.abs_diff(k) {
                    0 => 2.0,
                    1 => 1.0,
                    _ => 0.0,
                })
                .collect()
        })
        .collect();
    let b: Vec<f64> = (0..n).map(|i| i as f64).collect();
    let x = solve(a.clone(), b.clone());
    for i in 0..n {
        let back: f64 = (0..n).map(|k| a[i][k] * x[k]).sum();
        assert!((back - b[i]).abs() < 1e-9);
    }
}

/// the rank auc is the share of (fake, real) pairs the fake side wins, a tie counting half,
/// counted here pair by pair
#[test]
fn the_rank_auc_is_the_share_of_pairs_won() {
    let mut rng = numpy::Pcg64::from_seed(11);
    for size in [2usize, 7, 40] {
        // coarse scores so ties happen
        let scores: Vec<f64> = (0..size).map(|_| rng.interval(5) as f64).collect();
        let labels: Vec<bool> = (0..size).map(|i| i % 3 == 0).collect();
        let (mut won, mut pairs) = (0.0, 0.0);
        for (s, _) in scores.iter().zip(&labels).filter(|(_, l)| **l) {
            for (r, _) in scores.iter().zip(&labels).filter(|(_, l)| !**l) {
                pairs += 1.0;
                won += if s > r {
                    1.0
                } else if s == r {
                    0.5
                } else {
                    0.0
                };
            }
        }
        assert!(
            (auc(&scores, &labels) - won / pairs).abs() < 1e-12,
            "{size}"
        );
    }
}

/// the fitted judge sits where the penalised loss is flat: the mean of (p - y) times each
/// standardised input, plus 2 lambda theta, is zero. theta is read back from the scores of
/// probe rows at the training mean and one spread above it on each axis
#[test]
fn the_judge_stops_where_its_loss_is_flat() {
    let mut rng = numpy::Pcg64::from_seed(5);
    let mut noise = || rng.interval(1000) as f64 / 1000.0;
    let train: Vec<Vec<f64>> = (0..30)
        .map(|i| vec![(i % 2) as f64 + noise(), 3.0 * noise() - 1.0])
        .collect();
    let labels: Vec<bool> = (0..30).map(|i| i % 2 == 1).collect();
    let dim = train[0].len();
    let column = |j: usize| train.iter().map(|r| r[j]).collect::<Vec<_>>();
    let (means, spreads): (Vec<f64>, Vec<f64>) = (0..dim)
        .map(|j| (mean(&column(j)), std(&column(j))))
        .unzip();
    let probes: Vec<Vec<f64>> = std::iter::once(means.clone())
        .chain((0..dim).map(|j| {
            let mut probe = means.clone();
            probe[j] += spreads[j];
            probe
        }))
        .collect();
    let scores = logistic(&train, &labels, &probes);
    let bias = scores[0];
    let weights: Vec<f64> = scores[1..].iter().map(|s| s - bias).collect();
    let mut gradient = vec![0.0; dim + 1];
    for (row, label) in train.iter().zip(&labels) {
        let x: Vec<f64> = (0..dim).map(|j| (row[j] - means[j]) / spreads[j]).collect();
        let z = bias + x.iter().zip(&weights).map(|(a, b)| a * b).sum::<f64>();
        let residual = 1.0 / (1.0 + (-z).exp()) - f64::from(u8::from(*label));
        for j in 0..dim {
            gradient[j] += residual * x[j] / train.len() as f64;
        }
        gradient[dim] += residual / train.len() as f64;
    }
    let theta: Vec<f64> = weights.iter().copied().chain([bias]).collect();
    for (g, t) in gradient.iter().zip(&theta) {
        assert!((g + 2.0 * 1e-3 * t).abs() < 1e-9, "{g} {t}");
    }
}
