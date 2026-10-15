//! abcurves' classifier two-sample test, ported from abcurves/judges.py: its grouped stratified
//! folds, its logistic judge and its rank auc. the folds come off numpy's own generator, because
//! a different split is a different number

use crate::analysis::numpy::{self, mean, std};

/// mann-whitney auc with average ranks for ties, as _auc does it
pub fn auc(scores: &[f64], labels: &[bool]) -> f64 {
    let n = scores.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|a, b| scores[*a].total_cmp(&scores[*b]));
    let mut ranks = vec![0.0f64; n];
    let mut start = 0;
    while start < n {
        let mut stop = start + 1;
        while stop < n && scores[order[stop]] == scores[order[start]] {
            stop += 1;
        }
        let rank = 0.5 * ((start + 1) as f64 + stop as f64);
        for &i in &order[start..stop] {
            ranks[i] = rank;
        }
        start = stop;
    }
    let positives = labels.iter().filter(|l| **l).count();
    let negatives = n - positives;
    if positives == 0 || negatives == 0 {
        return 0.5;
    }
    let rank_sum: f64 = (0..n).filter(|i| labels[*i]).map(|i| ranks[i]).sum();
    (rank_sum - positives as f64 * (positives as f64 + 1.0) / 2.0)
        / (positives as f64 * negatives as f64)
}

/// _stratified_group_folds: every group goes whole into one test fold, a greedy balance picked
/// from 64 seeded restarts. `names[row]` is the group's canonical name as judges.py spells it,
/// because numpy's unique() sorts those strings and the shuffle runs over that order.
/// returns the test rows of each fold
pub fn folds(names: &[String], labels: &[bool], folds: usize, seed: u32) -> Vec<Vec<usize>> {
    let n = labels.len();
    let mut unique: Vec<String> = names.to_vec();
    unique.sort();
    unique.dedup();
    let n_groups = unique.len();
    let inverse: Vec<usize> = names
        .iter()
        .map(|name| unique.binary_search(name).expect("its own name"))
        .collect();
    let mut counts = vec![[0.0f64; 2]; n_groups];
    for (row, &fake) in labels.iter().enumerate() {
        counts[inverse[row]][usize::from(fake)] += 1.0;
    }
    let sizes: Vec<f64> = counts.iter().map(|c| c[0] + c[1]).collect();
    let totals = [
        counts.iter().map(|c| c[0]).sum::<f64>(),
        counts.iter().map(|c| c[1]).sum::<f64>(),
    ];
    let groups_per_class = [
        counts.iter().filter(|c| c[0] > 0.0).count(),
        counts.iter().filter(|c| c[1] > 0.0).count(),
    ];
    let n_folds = folds
        .min(n_groups)
        .min(groups_per_class[0].min(groups_per_class[1]));
    if n_folds < 2 {
        return Vec::new();
    }
    let total_size = n as f64;

    let mut rng = numpy::Pcg64::from_seed(seed);
    let mut best: Option<(f64, Vec<usize>)> = None;
    for _ in 0..64 {
        // shuffled, then the biggest groups first, stable, as order[argsort(-sizes)] does it
        let mut order = rng.permutation(n_groups);
        order.sort_by_key(|g| std::cmp::Reverse(sizes[*g] as u64));
        let mut assignment = vec![usize::MAX; n_groups];
        let mut fold_counts = vec![[0.0f64; 2]; n_folds];
        let mut fold_groups = vec![0usize; n_folds];

        for (step, &group) in order.iter().enumerate() {
            let empty: Vec<usize> = (0..n_folds).filter(|f| fold_groups[*f] == 0).collect();
            let candidates: Vec<usize> = if step < n_folds && !empty.is_empty() {
                empty
            } else {
                (0..n_folds).collect()
            };
            let mut chosen = candidates[0];
            let mut best_score = f64::INFINITY;
            for &fold in &candidates {
                let mut trial = fold_counts.clone();
                trial[fold][0] += counts[group][0];
                trial[fold][1] += counts[group][1];
                let label_balance = mean(&[
                    std(&trial.iter().map(|t| t[0] / totals[0]).collect::<Vec<_>>()),
                    std(&trial.iter().map(|t| t[1] / totals[1]).collect::<Vec<_>>()),
                ]);
                let size_balance = std(&trial
                    .iter()
                    .map(|t| (t[0] + t[1]) / total_size.max(1.0))
                    .collect::<Vec<_>>());
                let group_balance = std(&(0..n_folds)
                    .map(|f| {
                        (fold_groups[f] as f64 + if f == fold { 1.0 } else { 0.0 })
                            / (n_groups as f64).max(1.0)
                    })
                    .collect::<Vec<_>>());
                let score = label_balance + 0.2 * size_balance + 0.05 * group_balance;
                if score < best_score {
                    best_score = score;
                    chosen = fold;
                }
            }
            assignment[group] = chosen;
            fold_counts[chosen][0] += counts[group][0];
            fold_counts[chosen][1] += counts[group][1];
            fold_groups[chosen] += 1;
        }

        let mut valid = fold_counts.iter().all(|c| c[0] > 0.0 && c[1] > 0.0);
        for c in &fold_counts {
            valid = valid && totals[0] - c[0] > 0.0 && totals[1] - c[1] > 0.0;
        }
        let score = mean(&[
            std(&fold_counts
                .iter()
                .map(|c| c[0] / totals[0])
                .collect::<Vec<_>>()),
            std(&fold_counts
                .iter()
                .map(|c| c[1] / totals[1])
                .collect::<Vec<_>>()),
        ]) + 0.2
            * std(&fold_counts
                .iter()
                .map(|c| (c[0] + c[1]) / total_size.max(1.0))
                .collect::<Vec<_>>());
        if valid && best.as_ref().is_none_or(|(b, _)| score < *b) {
            best = Some((score, assignment));
        }
    }

    let Some((_, assignment)) = best else {
        return Vec::new();
    };
    (0..n_folds)
        .map(|fold| {
            (0..n)
                .filter(|row| assignment[inverse[*row]] == fold)
                .collect()
        })
        .collect()
}

/// the judge in _descriptor_cv: standardise on the training rows, then a zero-started logistic
/// regression with an l2 penalty of 1e-3 on weights and bias. torch fits it with lbfgs; the loss
/// is convex so newton lands on the same optimum, to more digits
pub fn logistic(train: &[Vec<f64>], labels: &[bool], test: &[Vec<f64>]) -> Vec<f64> {
    const LAMBDA: f64 = 1e-3;
    let dim = train[0].len() + 1;
    let n = train.len() as f64;
    let mut mean = vec![0.0f64; dim - 1];
    let mut std = vec![0.0f64; dim - 1];
    for j in 0..dim - 1 {
        let column = train.iter().map(|r| r[j]).collect::<Vec<_>>();
        mean[j] = numpy::mean(&column);
        std[j] = numpy::std(&column);
        if std[j] < 1e-6 {
            std[j] = 1.0;
        }
    }
    let standardise = |rows: &[Vec<f64>]| -> Vec<Vec<f64>> {
        rows.iter()
            .map(|r| {
                let mut x = vec![1.0f64; dim];
                for j in 0..dim - 1 {
                    x[j] = (r[j] - mean[j]) / std[j];
                }
                x
            })
            .collect()
    };
    let x = standardise(train);
    let y: Vec<f64> = labels.iter().map(|l| if *l { 1.0 } else { 0.0 }).collect();

    let loss = |theta: &[f64]| -> f64 {
        let data: f64 = x
            .iter()
            .zip(&y)
            .map(|(row, yi)| {
                let z: f64 = row.iter().zip(theta).map(|(a, b)| a * b).sum();
                // binary cross entropy with logits, the stable form torch uses
                z.max(0.0) - z * yi + (-z.abs()).exp().ln_1p()
            })
            .sum::<f64>()
            / n;
        data + LAMBDA * theta.iter().map(|t| t * t).sum::<f64>()
    };

    let mut theta = vec![0.0f64; dim];
    for _ in 0..100 {
        let mut gradient = vec![0.0f64; dim];
        let mut hessian = vec![vec![0.0f64; dim]; dim];
        for (row, yi) in x.iter().zip(&y) {
            let z: f64 = row.iter().zip(&theta).map(|(a, b)| a * b).sum();
            let p = 1.0 / (1.0 + (-z).exp());
            let w = p * (1.0 - p);
            for a in 0..dim {
                gradient[a] += (p - yi) * row[a] / n;
                for b in 0..dim {
                    hessian[a][b] += w * row[a] * row[b] / n;
                }
            }
        }
        for a in 0..dim {
            gradient[a] += 2.0 * LAMBDA * theta[a];
            hessian[a][a] += 2.0 * LAMBDA;
        }
        let direction = solve(hessian, gradient);
        // damped newton: halve the step until the loss drops, which it always does eventually
        // on a strongly convex loss. plain newton can overshoot on separable data
        let before = loss(&theta);
        let mut scale = 1.0;
        let mut next = theta.clone();
        loop {
            for a in 0..dim {
                next[a] = theta[a] - scale * direction[a];
            }
            if loss(&next) <= before || scale < 1e-6 {
                break;
            }
            scale *= 0.5;
        }
        let moved: f64 = direction
            .iter()
            .map(|d| (d * scale).abs())
            .fold(0.0, f64::max);
        theta = next;
        if moved < 1e-12 {
            break;
        }
    }

    standardise(test)
        .iter()
        .map(|row| row.iter().zip(&theta).map(|(a, b)| a * b).sum())
        .collect()
}

/// gaussian elimination with partial pivoting, for the symmetric positive definite newton step
fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Vec<f64> {
    let n = b.len();
    for col in 0..n {
        let pivot = (col..n)
            .max_by(|i, j| a[*i][col].abs().total_cmp(&a[*j][col].abs()))
            .unwrap();
        a.swap(col, pivot);
        b.swap(col, pivot);
        let pivot_row = a[col].clone();
        for row in col + 1..n {
            let factor = a[row][col] / pivot_row[col];
            for k in col..n {
                a[row][k] -= factor * pivot_row[k];
            }
            b[row] -= factor * b[col];
        }
    }
    let mut x = vec![0.0; n];
    for row in (0..n).rev() {
        x[row] = (b[row] - (row + 1..n).map(|k| a[row][k] * x[k]).sum::<f64>()) / a[row][row];
    }
    x
}

#[cfg(test)]
mod tests;
