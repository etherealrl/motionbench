use crate::analysis::error::{Error, check};
use crate::analysis::judge;
use crate::analysis::numpy::{Pcg64, SeedSequence, linear_quantile, mean, sorted};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::atomic::AtomicBool};

pub const REVISION: &str = "e197441f8d343d3b4a4069c7cae20764d0953d15";

#[derive(Clone, Serialize, Deserialize)]
pub struct Config {
    pub seed: u32,
    pub folds: usize,
    pub repeats: usize,
    pub bootstrap: usize,
    pub permutations: usize,
    pub width_ms: usize,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            seed: 7,
            folds: 5,
            repeats: 5,
            bootstrap: 200,
            permutations: 2000,
            width_ms: 160,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Report {
    pub auc: f64,
    pub auc_ci: Option<[f64; 2]>,
    pub repeat_auc: Vec<f64>,
    pub fold_auc: Vec<Vec<f64>>,
    pub permutation_auc: Vec<f64>,
    pub p_value: Option<f64>,
    /// the smallest p the shuffles could give with this many groups a side, however far apart
    /// the populations are. none when the labels aren't whole groups
    pub smallest_p: Option<f64>,
    pub null_p95: Option<f64>,
    pub n_real: usize,
    pub n_fake: usize,
    pub n_groups: usize,
    pub permutation_error: Option<String>,
    pub permutation_method: String,
}

struct Fold {
    scores: Vec<f64>,
    labels: Vec<bool>,
    groups: Vec<String>,
    auc: f64,
    weight: f64,
}

fn seed(words: &[u32]) -> u32 {
    SeedSequence::new(words).generate_state(1)[0]
}
fn aggregate(folds: &[Fold]) -> f64 {
    let total = folds.iter().map(|f| f.weight).sum::<f64>();
    folds.iter().map(|f| f.auc * f.weight).sum::<f64>() / total.max(1.0)
}

fn cv(
    rows: &[Vec<f64>],
    labels: &[bool],
    groups: &[String],
    plans: &[Vec<Vec<usize>>],
    cancel: &AtomicBool,
) -> Result<Vec<Vec<Fold>>, Error> {
    plans
        .iter()
        .map(|plan| {
            plan.iter()
                .map(|test| {
                    check(cancel)?;
                    let mut included = vec![false; rows.len()];
                    for &i in test {
                        included[i] = true;
                    }
                    let train = (0..rows.len())
                        .filter(|i| !included[*i])
                        .collect::<Vec<_>>();
                    let train_rows = train.iter().map(|i| rows[*i].clone()).collect::<Vec<_>>();
                    let train_labels = train.iter().map(|i| labels[*i]).collect::<Vec<_>>();
                    let test_rows = test.iter().map(|i| rows[*i].clone()).collect::<Vec<_>>();
                    let test_labels = test.iter().map(|i| labels[*i]).collect::<Vec<_>>();
                    let scores = judge::logistic(&train_rows, &train_labels, &test_rows);
                    let positives = test_labels.iter().filter(|v| **v).count();
                    Ok(Fold {
                        auc: judge::auc(&scores, &test_labels),
                        weight: (positives * (test_labels.len() - positives)) as f64,
                        scores,
                        labels: test_labels,
                        groups: test.iter().map(|i| groups[*i].clone()).collect(),
                    })
                })
                .collect()
        })
        .collect()
}

fn clusters(groups: &[String]) -> Vec<Vec<usize>> {
    let mut map = BTreeMap::<&str, Vec<usize>>::new();
    for (i, group) in groups.iter().enumerate() {
        map.entry(group).or_default().push(i);
    }
    map.into_values().collect()
}

fn permute(labels: &[bool], groups: &[Vec<usize>], rng: &mut Pcg64) -> Vec<bool> {
    let pure = groups
        .iter()
        .all(|indices| indices.iter().all(|i| labels[*i] == labels[indices[0]]));
    let mut out = labels.to_vec();
    if pure {
        let order = rng.permutation(groups.len());
        for (cluster, j) in groups.iter().zip(order) {
            for &i in cluster {
                out[i] = labels[groups[j][0]];
            }
        }
    } else {
        for indices in groups {
            let positives = indices.iter().filter(|i| labels[**i]).count();
            if positives * 2 == indices.len() {
                if rng.integer(2) != 0 {
                    for &i in indices {
                        out[i] = !labels[i];
                    }
                }
            } else {
                let order = rng.permutation(indices.len());
                for (&i, j) in indices.iter().zip(order) {
                    out[i] = labels[indices[j]];
                }
            }
        }
    }
    out
}

pub fn run(
    real: &[Vec<f64>],
    fake: &[Vec<f64>],
    real_groups: &[String],
    fake_groups: &[String],
    config: &Config,
    cancel: &AtomicBool,
    progress: impl Fn(f64),
) -> Result<Report, Error> {
    if real.len() < 2 || fake.len() < 2 {
        return Err("each population needs at least two rows".into());
    }
    if real.len() != real_groups.len() || fake.len() != fake_groups.len() {
        return Err("source groups do not match the rows".into());
    }
    let dim = real[0].len();
    if dim == 0
        || dim > 49
        || real
            .iter()
            .chain(fake)
            .any(|r| r.len() != dim || r.iter().any(|v| !v.is_finite()))
    {
        return Err("invalid descriptor matrix".into());
    }
    if !(2..=5).contains(&config.folds)
        || !(1..=20).contains(&config.repeats)
        || config.permutations > 10_000
        || config.bootstrap > 10_000
    {
        return Err("analysis configuration is outside supported limits".into());
    }
    let rows = real
        .iter()
        .chain(fake)
        .map(|r| r.iter().map(|v| *v as f32 as f64).collect())
        .collect::<Vec<Vec<f64>>>();
    // a value past what f32 holds turns infinite on the way, and the fit can't use it
    if rows.iter().flatten().any(|v| !v.is_finite()) {
        return Err("invalid descriptor matrix".into());
    }
    let labels = (0..rows.len()).map(|i| i >= real.len()).collect::<Vec<_>>();
    let groups = real_groups
        .iter()
        .chain(fake_groups)
        .cloned()
        .collect::<Vec<_>>();
    let clustered = clusters(&groups);
    let make_plans = |assigned: &[bool]| {
        (0..config.repeats)
            .map(|repeat| {
                judge::folds(
                    &groups,
                    assigned,
                    config.folds,
                    seed(&[config.seed, repeat as u32, 0xC2A5]),
                )
            })
            .collect::<Vec<_>>()
    };
    let plans = make_plans(&labels);
    if plans.iter().any(|p| p.len() < 2) {
        return Err("need at least two independent source groups per population, add recordings or source trials".into());
    }
    let records = cv(&rows, &labels, &groups, &plans, cancel)?;
    let repeat_auc = records.iter().map(|r| aggregate(r)).collect::<Vec<_>>();
    let auc = mean(&repeat_auc);
    let mut bootstrap_rng = Pcg64::from_seed(seed(&[config.seed, 0xB007]));
    let mut bootstrap_values = Vec::new();
    for _ in 0..(config.bootstrap * 10).max(100) {
        if bootstrap_values.len() >= config.bootstrap {
            break;
        }
        check(cancel)?;
        let repeat = &records[bootstrap_rng.integer(records.len())];
        let mut total = 0.0;
        let mut weight = 0.0;
        for fold in repeat {
            let group_indices = clusters(&fold.groups);
            let sampled = (0..group_indices.len())
                .flat_map(|_| group_indices[bootstrap_rng.integer(group_indices.len())].clone())
                .collect::<Vec<_>>();
            let scores = sampled.iter().map(|i| fold.scores[*i]).collect::<Vec<_>>();
            let labels = sampled.iter().map(|i| fold.labels[*i]).collect::<Vec<_>>();
            let positive = labels.iter().filter(|v| **v).count();
            let w = (positive * (labels.len() - positive)) as f64;
            if w > 0.0 {
                total += judge::auc(&scores, &labels) * w;
                weight += w;
            }
        }
        if weight > 0.0 {
            bootstrap_values.push(total / weight);
        }
    }
    let mut rng = Pcg64::from_seed(seed(&[config.seed, 0x9E37]));
    let mut permutation_auc = Vec::new();
    let independent = clustered
        .iter()
        .all(|group| group.iter().all(|i| labels[*i] == labels[group[0]]));
    let mut permutation_error = None;
    for iteration in 0..config.permutations {
        check(cancel)?;
        let shuffled = permute(&labels, &clustered, &mut rng);
        let original_valid = plans.iter().flatten().all(|test| {
            let pos = test.iter().filter(|i| shuffled[**i]).count();
            let train_pos = shuffled.iter().filter(|v| **v).count() - pos;
            pos > 0 && pos < test.len() && train_pos > 0 && train_pos < rows.len() - test.len()
        });
        // independent labels change which groups belong to each class, so their stratified
        // folds must be reconstructed too. rejection against the original folds biases the null
        let rebuilt;
        let null_plans = if independent || !original_valid {
            rebuilt = make_plans(&shuffled);
            &rebuilt
        } else {
            &plans
        };
        if null_plans.iter().any(|p| p.len() < 2) {
            permutation_error = Some("AUC was computed, but grouped folds could not be formed for a null assignment; the permutation test is unavailable".into());
            permutation_auc.clear();
            break;
        }
        let null_records = cv(&rows, &shuffled, &groups, null_plans, cancel)?;
        permutation_auc.push(mean(
            &null_records
                .iter()
                .map(|r| aggregate(r))
                .collect::<Vec<_>>(),
        ));
        progress((iteration + 1) as f64 / config.permutations as f64);
    }
    let p_value = if permutation_auc.is_empty() {
        None
    } else {
        Some(
            (1 + permutation_auc
                .iter()
                .filter(|a| (**a - 0.5).abs() >= (auc - 0.5).abs())
                .count()) as f64
                / (permutation_auc.len() + 1) as f64,
        )
    };
    let null_p95 = if permutation_auc.is_empty() {
        None
    } else {
        Some(
            0.5 + linear_quantile(
                &sorted(
                    &permutation_auc
                        .iter()
                        .map(|a| (a - 0.5).abs())
                        .collect::<Vec<_>>(),
                ),
                0.95,
            ),
        )
    };
    // a fold with one group of a label can only draw that group again, so its interval is
    // only the folds' own AUCs
    let resampled = records.iter().flatten().all(|fold| {
        [true, false].into_iter().all(|label| {
            clusters(&fold.groups)
                .iter()
                .filter(|group| group.iter().any(|i| fold.labels[*i] == label))
                .count()
                >= 2
        })
    });
    let smallest_p = independent.then(|| {
        let n = clustered.len();
        let k = clustered.iter().filter(|group| labels[group[0]]).count();
        let assignments = (0..k).fold(1.0, |c, i| c * (n - i) as f64 / (i + 1) as f64);
        // with as many groups a side, the labels swapped are as far from chance as the real ones
        (if 2 * k == n { 2.0 } else { 1.0 }) / assignments
    });
    let auc_ci = if bootstrap_values.is_empty() || !resampled {
        None
    } else {
        let bootstrap = sorted(&bootstrap_values);
        Some([
            linear_quantile(&bootstrap, 0.025),
            linear_quantile(&bootstrap, 0.975),
        ])
    };
    Ok(Report {
        auc,
        auc_ci,
        repeat_auc,
        fold_auc: records
            .iter()
            .map(|r| r.iter().map(|f| f.auc).collect())
            .collect(),
        permutation_auc,
        p_value,
        smallest_p,
        null_p95,
        n_real: real.len(),
        n_fake: fake.len(),
        n_groups: clustered.len(),
        permutation_error,
        permutation_method: if independent {
            "whole-source labels shuffled; grouped stratified folds rebuilt for each assignment"
        } else {
            "within-source grouped assignments; original grouped folds retained when valid"
        }
        .into(),
    })
}

#[cfg(test)]
mod tests;
