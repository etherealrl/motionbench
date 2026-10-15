//! comparisons, their results and exports
use crate::{
    app::state::{Bench, Shared, off_main},
    library,
};
use motion_bench::sync::LockExt;

use crate::library::Run;
use motion_bench::analysis::{
    classify::Config,
    compare::{self, ResultRow},
};
use serde::Deserialize;
use serde_json::json;
use std::path::{Path, PathBuf};
use tauri::State;

#[derive(Clone, Deserialize)]
pub(crate) struct AnalysisRequest {
    #[serde(default)]
    pub(crate) matching: bool,
    pub(crate) question: Option<String>,
    pub(crate) baseline: Vec<String>,
    pub(crate) candidate: Vec<String>,
    pub(crate) paired: bool,
    pub(crate) panel: String,
    pub(crate) config: Config,
    pub(crate) range: Option<[f64; 2]>,
}
#[tauri::command]
pub(crate) fn analyze(request: AnalysisRequest, state: State<Shared>) -> Result<u64, String> {
    analyze_inner(request, &state)
}
pub(crate) fn check_request(request: &AnalysisRequest) -> Result<(), String> {
    // a paired comparison reads one recording against its own input, so a second one would be
    // dropped without a word and the result saved beside whichever came first
    if request.paired && request.candidate.len() != 1 {
        return Err(format!(
            "a paired comparison reads one recording against its own box input, so pick just one, {} are selected",
            request.candidate.len()
        ));
    }
    if request
        .range
        .is_some_and(|r| !r[0].is_finite() || !r[1].is_finite() || r[0] < 0.0 || r[1] <= r[0])
    {
        return Err("that time range isn't valid".into());
    }
    if request.config.width_ms < 8 || request.config.width_ms > 1000 {
        return Err("texture width must be between 8 and 1000 ms".into());
    }
    Ok(())
}
pub(crate) fn analyze_inner(request: AnalysisRequest, state: &Shared) -> Result<u64, String> {
    check_request(&request)?;
    let (library, jobs) = {
        let bench = state.bench.locked();
        (bench.library.clone(), bench.jobs.clone())
    };
    // the job starts while the runs can't change, so a deletion can't slip in between and leave
    // the job working on a run that is gone
    let (runs, (id, cancel)) =
        library.with_runs(|runs| Ok::<_, String>((runs.to_vec(), jobs.start()?)))?;
    let candidate = request.candidate.clone();
    let work = move |progress: &dyn Fn(f64)| {
        let resolve = |ids: &[String]| -> Result<Vec<Run>, String> {
            ids.iter()
                .map(|id| {
                    runs.iter()
                        .find(|r| &r.id == id)
                        .cloned()
                        .ok_or("selected run is unavailable".into())
                })
                .collect()
        };
        let mut result = (|| -> Result<ResultRow, motion_bench::analysis::error::Error> {
            let candidates = resolve(&request.candidate)?;
            // paired mode has no baseline, a stale one left selected must not fail it
            let baselines = if request.paired {
                Vec::new()
            } else {
                resolve(&request.baseline)?
            };
            if request.paired {
                let run = candidates.first().ok_or("select a candidate run")?;
                compare::paired_selected(
                    &compare::Source {
                        path: PathBuf::from(&run.path),
                        decoder: Some(run.decoder.clone()),
                        endpoint: run.selected.clone(),
                        crop: run.crop,
                    },
                    request.config,
                    request.range,
                    &cancel,
                    progress,
                )
            } else {
                let sources = |runs: &[Run]| {
                    runs.iter()
                        .map(|r| compare::Source {
                            path: PathBuf::from(&r.path),
                            decoder: Some(r.decoder.clone()),
                            endpoint: r.selected.clone(),
                            crop: r.crop,
                        })
                        .collect::<Vec<_>>()
                };
                let base = sources(&baselines);
                let candidate = sources(&candidates);
                let (base, candidate) = if request.matching {
                    compare::matching_sources(&base, &candidate)?
                } else {
                    (base, candidate)
                };
                let mut result = compare::independent_selected(
                    &base,
                    &candidate,
                    &request.panel,
                    request.range,
                    request.config,
                    &cancel,
                    progress,
                )?;
                if request.matching {
                    result.comparison = "matching source events".into();
                }
                Ok(result)
            }
        })()?;
        if !result.metadata.is_object() {
            result.metadata = json!({"recording":result.metadata});
        }
        result.metadata["question"] = json!(request.question);
        result.metadata["grouping"] = json!(if request.paired {
            "eight consecutive wire windows"
        } else {
            "source identity or recording/session"
        });
        result.identify();
        Ok(result)
    };
    let publish = move |result: &ResultRow| {
        library
            .attach(result, &candidate)
            .err()
            .map(|error| format!("analysis complete, though it couldn't be saved: {error}"))
    };
    library::jobs::spawn(jobs, id, work, publish);
    Ok(id)
}
#[tauri::command]
pub(crate) fn cancel_analysis(id: u64, state: State<Shared>) -> Result<(), String> {
    let jobs = state.bench.locked().jobs.clone();
    jobs.cancel(id)
}
/// where the result on screen came from, so an export never picks up another one
#[derive(Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "lowercase")]
pub(crate) enum ResultSource {
    Job(u64),
    /// a saved result by its own id
    Result(String),
}
#[tauri::command]
pub(crate) async fn export_result(
    path: String,
    source: ResultSource,
    state: State<'_, Shared>,
) -> Result<(), String> {
    off_main(state, move |state| {
        // the file is written outside the lock, a slow disk holds nothing else
        let result = exported(&state.bench.locked(), source)?;
        compare::write_result(Path::new(&path), &result)
    })
    .await
}
#[cfg(test)]
pub(crate) fn export(bench: &Bench, path: &Path, source: ResultSource) -> Result<(), String> {
    compare::write_result(path, &exported(bench, source)?)
}
pub(crate) fn exported(bench: &Bench, source: ResultSource) -> Result<ResultRow, String> {
    match source {
        ResultSource::Job(id) => bench.jobs.result(id),
        ResultSource::Result(id) => bench.library.result(&id),
    }
}

#[tauri::command]
pub(crate) fn saved_result(id: String, state: State<Shared>) -> Result<ResultRow, String> {
    state.library().result(&id)
}
#[tauri::command]
pub(crate) fn analysis_result(id: u64, state: State<Shared>) -> Result<ResultRow, String> {
    let jobs = state.bench.locked().jobs.clone();
    jobs.result(id)
}
