use super::*;
use crate::library::jobs;

fn fixture_pcap() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../tests/fixtures/mouse-1000hz.pcap"
    ))
}

/// the 1 kHz fixture recording as `name` in `folder`, its metadata beside it
fn recorded(folder: &Path, name: &str) -> PathBuf {
    let pcap = folder.join(name);
    std::fs::copy(fixture_pcap(), &pcap).unwrap();
    std::fs::copy(
        fixture_pcap().with_extension("json"),
        pcap.with_extension("json"),
    )
    .unwrap();
    pcap
}

/// the library in `folder` as a restart reads it
fn restarted(folder: &Path) -> (Library, Vec<String>) {
    let library = Library::new(folder.to_path_buf());
    let warnings = library.restore();
    (library, warnings)
}

fn saved_result(library: &Library, candidates: &[String], panel: &str) -> ResultRow {
    let mut result = jobs::tests::row(panel);
    result.crops = candidates.iter().map(|id| (id.clone(), None)).collect();
    result.identify();
    library.attach(&result, candidates).unwrap();
    result
}

#[test]
fn a_restart_keeps_names_crops_devices_and_results() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path();
    // a capture with nothing beside it was not recorded here, so the scan leaves it out
    std::fs::copy(fixture_pcap(), folder.join("elsewhere.pcap")).unwrap();
    let pcap = recorded(folder, "existing.pcap");
    let library = Library::new(folder.to_path_buf());
    let run = library.add(import(&pcap).unwrap()).unwrap();
    library.rename(&run.id, "Direct mouse 1").unwrap();
    let crop = Selection {
        from_us: 1,
        to_us: 2,
    };
    library
        .set_crop(&run.id, Some(crop), &jobs::Jobs::default())
        .unwrap();
    let result = saved_result(&library, std::slice::from_ref(&run.id), "kept");

    let (library, warnings) = restarted(folder);
    assert!(warnings.is_empty(), "{warnings:?}");
    let runs = library.runs();
    assert_eq!(runs.len(), 1);
    let restored = &runs[0];
    assert_eq!(restored.id, "existing");
    assert_eq!(restored.name, "Direct mouse 1");
    assert_eq!(restored.crop, Some(crop));
    assert_eq!(restored.selected, run.selected);
    assert!(matches!(restored.decoder, Decoder::WindowsDescriptor(_)));
    assert!(restored.panels.iter().any(|p| p == "texture19"));
    // the crop the result read isn't this one, so it's put away until they agree again
    assert!(restored.result.is_none());
    assert_eq!(library.result(&result.id).unwrap().panel, "kept");
}

/// the folder moved somewhere else keeps everything it had, a recording taken out stays out
#[test]
fn moving_the_folder_keeps_every_recording_as_it_was() {
    let before = tempfile::tempdir().unwrap();
    let kept = recorded(before.path(), "kept.pcap");
    let gone = recorded(before.path(), "gone.pcap");
    let library = Library::new(before.path().to_path_buf());
    let kept = library.add(import(&kept).unwrap()).unwrap();
    let gone = library.add(import(&gone).unwrap()).unwrap();
    library.rename(&kept.id, "renamed").unwrap();
    library.remove(&gone.id, &jobs::Jobs::default()).unwrap();
    let result = saved_result(&library, std::slice::from_ref(&kept.id), "moved");

    let after = tempfile::tempdir().unwrap();
    for file in std::fs::read_dir(before.path()).unwrap() {
        let file = file.unwrap();
        let to = after.path().join(file.file_name());
        if file.path().is_dir() {
            std::fs::create_dir(&to).unwrap();
            for inner in std::fs::read_dir(file.path()).unwrap() {
                let inner = inner.unwrap();
                std::fs::copy(inner.path(), to.join(inner.file_name())).unwrap();
            }
        } else {
            std::fs::copy(file.path(), to).unwrap();
        }
    }
    drop(before);

    let (library, warnings) = restarted(after.path());
    assert!(warnings.is_empty(), "{warnings:?}");
    let runs = library.runs();
    assert_eq!(runs.len(), 1, "the one taken out stays out");
    assert_eq!(runs[0].id, kept.id);
    assert_eq!(runs[0].name, "renamed");
    assert!(
        runs[0]
            .path
            .starts_with(&after.path().display().to_string())
    );
    assert_eq!(runs[0].result.as_ref().unwrap().id, result.id);
    // the catalog points at the new place now
    let (again, _) = restarted(after.path());
    assert_eq!(again.runs()[0].path, runs[0].path);
}

/// imported from somewhere else a recording is the same one, with what it had
#[test]
fn an_import_from_elsewhere_keeps_the_recordings_identity() {
    let home = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let away = recorded(elsewhere.path(), "away.pcap");
    let library = Library::new(home.path().to_path_buf());
    let run = library.add(import(&away).unwrap()).unwrap();
    library.rename(&run.id, "from the other disk").unwrap();
    let (library, warnings) = restarted(home.path());
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(library.runs()[0].name, "from the other disk");
    let again = library.add(import(&away).unwrap()).unwrap();
    assert_eq!(
        (again.id.as_str(), again.name.as_str()),
        ("away", "from the other disk")
    );
    assert_eq!(library.runs().len(), 1);
}

/// two recordings that only share a file name are never taken for one
#[test]
fn a_recording_with_the_same_name_but_other_bytes_is_refused() {
    let home = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let listed = recorded(home.path(), "twin.pcap");
    let library = Library::new(home.path().to_path_buf());
    library.add(import(&listed).unwrap()).unwrap();
    // the same bytes somewhere else are the same recording
    let copy = recorded(elsewhere.path(), "twin.pcap");
    assert!(library.add(import(&copy).unwrap()).is_ok());
    let mut bytes = std::fs::read(&copy).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&copy, bytes).unwrap();
    let refused = library.add(import(&copy).unwrap()).err().unwrap();
    assert!(refused.contains("not the same contents"), "{refused}");
    assert_eq!(library.runs().len(), 1);
}

/// one result shows on every recording it read, after a restart too, and is found by its own id
#[test]
fn a_result_stays_with_every_candidate_it_read() {
    let dir = tempfile::tempdir().unwrap();
    let library = Library::new(dir.path().to_path_buf());
    let ids: Vec<String> = ["first.pcap", "second.pcap"]
        .iter()
        .map(|name| {
            let pcap = recorded(dir.path(), name);
            library.add(import(&pcap).unwrap()).unwrap().id
        })
        .collect();
    let result = saved_result(&library, &ids, "both");
    let (library, _) = restarted(dir.path());
    for run in library.runs() {
        assert_eq!(run.result.as_ref().unwrap().id, result.id, "{}", run.id);
    }
    assert!(library.result("not-a-result").is_err());
    // the id is the result's own, the same content gives the same one
    let mut again = result.clone();
    again.identify();
    assert_eq!(again.id, result.id);
    again.panel = "other".into();
    again.identify();
    assert_ne!(again.id, result.id);
}

#[test]
fn the_runs_are_listed_at_a_revision_that_moves_with_every_change() {
    let dir = tempfile::tempdir().unwrap();
    let library = Library::new(dir.path().to_path_buf());
    let mut seen = vec![library.revision()];
    let id = library
        .add(import(&recorded(dir.path(), "first.pcap")).unwrap())
        .unwrap()
        .id;
    seen.push(library.revision());
    let mut result = jobs::tests::row("texture19");
    result.real_features = vec![vec![1.0; 19]; 4];
    result.crops = [(id.clone(), None)].into();
    result.identify();
    library.attach(&result, std::slice::from_ref(&id)).unwrap();
    seen.push(library.revision());
    let (restored, _) = restarted(dir.path());
    assert!(seen.windows(2).all(|pair| pair[0] < pair[1]), "{seen:?}");
    assert!(restored.revision() > 0);
    // a listed result is its summary, its feature rows are read when it is opened
    let listed = serde_json::to_value(library.listed()).unwrap();
    assert_eq!(listed["revision"], seen[2]);
    assert_eq!(listed["runs"][0]["result"]["id"], result.id.as_str());
    assert_eq!(listed["runs"][0]["result"]["real_features"], json!([]));
    assert_eq!(library.result(&result.id).unwrap().real_features.len(), 4);
}

/// a change that couldn't be saved isn't kept either, Runs never shows what the catalog lacks
#[test]
fn a_change_that_cannot_be_saved_is_not_kept() {
    let dir = tempfile::tempdir().unwrap();
    let pcap = recorded(dir.path(), "kept.pcap");
    let library = Library::new(dir.path().to_path_buf());
    let run = library.add(import(&pcap).unwrap()).unwrap();
    let catalog = dir.path().join("library.json");
    std::fs::remove_file(&catalog).unwrap();
    std::fs::create_dir(&catalog).unwrap();
    assert!(library.rename(&run.id, "lost").is_err());
    assert_eq!(library.run(&run.id).unwrap().name, "kept.pcap");
    assert!(library.remove(&run.id, &jobs::Jobs::default()).is_err());
    assert_eq!(library.runs().len(), 1);
}

#[test]
fn a_running_analysis_keeps_its_runs_and_their_crops() {
    let dir = tempfile::tempdir().unwrap();
    let pcap = recorded(dir.path(), "busy.pcap");
    let library = Library::new(dir.path().to_path_buf());
    let run = library.add(import(&pcap).unwrap()).unwrap();
    let jobs = jobs::Jobs::default();
    let _job = jobs.start().unwrap();
    assert!(
        library
            .remove(&run.id, &jobs)
            .unwrap_err()
            .contains("analysis")
    );
    assert!(
        library
            .set_crop(&run.id, None, &jobs)
            .unwrap_err()
            .contains("analysis")
    );
    assert_eq!(library.runs().len(), 1);
}

#[test]
fn corrupt_optional_events_do_not_hide_the_saved_capture_on_import_or_restore() {
    let folder = tempfile::tempdir().unwrap();
    let path = recorded(folder.path(), "capture.pcap");
    std::fs::write(path.with_extension("events.npz"), b"broken dataset").unwrap();
    let run = import(&path).unwrap();
    assert!(run.panels.iter().any(|p| p == "texture19"));
    assert!(run.missing["full49"].contains("could not be loaded"));
    Library::new(folder.path().to_path_buf()).add(run).unwrap();
    let (library, warnings) = restarted(folder.path());
    assert_eq!(library.runs().len(), 1);
    assert!(!warnings.iter().any(|s| s.contains("capture.pcap")));
    assert!(import(&path.with_extension("events.npz")).is_err());
}

#[test]
fn removed_runs_stay_removed_until_explicit_import_without_deleting_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path();
    let path = recorded(folder, "recording.pcap");
    let library = Library::new(folder.to_path_buf());
    let run = library.add(import(&path).unwrap()).unwrap();
    let result = saved_result(&library, std::slice::from_ref(&run.id), "kept");
    library.remove(&run.id, &jobs::Jobs::default()).unwrap();
    let (library, warnings) = restarted(folder);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(library.runs().is_empty());
    assert_eq!(
        std::fs::read(&path).unwrap(),
        std::fs::read(fixture_pcap()).unwrap()
    );
    assert!(library.result(&result.id).is_ok());
    // imported again on purpose it's back, with its result
    let back = library.add(import(&path).unwrap()).unwrap();
    assert_eq!(back.result.unwrap().id, result.id);
    assert_eq!(restarted(folder).0.runs().len(), 1);
}

#[test]
fn an_unreadable_library_is_only_replaced_after_a_copy_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path();
    let run = import(fixture_pcap()).unwrap();
    let path = folder.join("library.json");
    let copy = folder.join("library.corrupt-1.json");
    std::fs::write(&path, b"broken library").unwrap();

    // the copy cannot be written, so nothing may touch the library
    std::fs::create_dir(&copy).unwrap();
    let library = Library::new(folder.to_path_buf());
    assert!(library.add(run.clone()).is_err());
    assert!(library.remove(&run.id, &jobs::Jobs::default()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"broken library");

    std::fs::remove_dir(&copy).unwrap();
    let library = Library::new(folder.to_path_buf());
    library.add(run.clone()).unwrap();
    assert_eq!(std::fs::read(&copy).unwrap(), b"broken library");
    assert_eq!(restarted(folder).0.runs().len(), 1);

    // a catalog of another schema is not read either, it's kept aside the same way
    std::fs::write(
        &path,
        br#"{"schema":"motion-bench.library.v1","recordings":{}}"#,
    )
    .unwrap();
    let (_, warnings) = restarted(folder);
    assert!(
        warnings[0].contains("motion-bench.library.v1"),
        "{warnings:?}"
    );
    assert!(folder.join("library.corrupt-2.json").is_file());
    // the restore wrote the catalog anew, so the next start has nothing to say
    let (_, warnings) = restarted(folder);
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn a_library_that_cannot_be_read_as_a_file_is_never_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path();
    let run = import(fixture_pcap()).unwrap();
    std::fs::create_dir(folder.join("library.json")).unwrap();
    let library = Library::new(folder.to_path_buf());
    assert!(library.add(run.clone()).is_err());
    assert!(!restarted(folder).1.is_empty());
    assert!(folder.join("library.json").is_dir());
    assert!(!folder.join("library.corrupt-1.json").exists());
}

#[test]
fn broken_library_files_are_copied_aside_without_changes() {
    let corpus = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../tests/fixtures/corpus"
    ));
    let folder = tempfile::tempdir().unwrap();
    recorded(folder.path(), "recording.pcap");
    let library = folder.path().join("library.json");
    let files = || -> BTreeMap<PathBuf, Vec<u8>> {
        std::fs::read_dir(folder.path())
            .unwrap()
            .filter_map(|e| {
                let path = e.unwrap().path();
                Some((path.clone(), std::fs::read(path).ok()?))
            })
            .collect()
    };
    let mut cases: Vec<PathBuf> = std::fs::read_dir(corpus.join("catalog"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    cases.sort();
    assert!(!cases.is_empty());
    for case in cases {
        let text = std::fs::read_to_string(&case).unwrap();
        std::fs::write(&library, &text).unwrap();
        let before = files();
        let (restored, warnings) = restarted(folder.path());
        assert!(!warnings.is_empty(), "{case:?}");
        // the recording in the folder is still listed under its file name
        assert_eq!(restored.runs().len(), 1, "{case:?}");
        // the unreadable library is copied aside byte for byte, then written anew so its notice
        // shows once. a reader may add files but never change another one
        let after = files();
        assert!(
            after.iter().any(|(path, bytes)| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("library.corrupt-")
                    && bytes == text.as_bytes()
            }),
            "{case:?}"
        );
        for (path, bytes) in &before {
            if *path == library {
                let written: serde_json::Value = serde_json::from_slice(&after[path]).unwrap();
                assert_eq!(written["schema"], CATALOG_SCHEMA, "{case:?}");
            } else {
                assert_eq!(after.get(path), Some(bytes), "{case:?} changed {path:?}");
            }
        }
    }
}

/// the catalog knows each recording's bytes, so another recording with a listed one's name
/// is never taken for it, even once the listed one is gone
#[test]
fn a_listed_name_with_other_bytes_is_never_the_listed_recording() {
    let home = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let listed = recorded(home.path(), "twin.pcap");
    let library = Library::new(home.path().to_path_buf());
    library.add(import(&listed).unwrap()).unwrap();
    let other = recorded(elsewhere.path(), "twin.pcap");
    let mut bytes = std::fs::read(&other).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&other, &bytes).unwrap();
    for extension in ["pcap", "json"] {
        std::fs::remove_file(listed.with_extension(extension)).unwrap();
    }
    let refused = library.add(import(&other).unwrap()).err().unwrap();
    assert!(refused.contains("not the same contents"), "{refused}");
    let (library, _) = restarted(home.path());
    assert!(library.add(import(&other).unwrap()).is_err());
}

/// a recording the catalog lists whose file is gone is said, and its entry stays for later
#[test]
fn a_listed_recording_whose_file_is_gone_is_said_and_kept() {
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let away = recorded(elsewhere.path(), "away.pcap");
    Library::new(dir.path().to_path_buf())
        .add(import(&away).unwrap())
        .unwrap();
    let hidden = elsewhere.path().join("hidden");
    std::fs::create_dir(&hidden).unwrap();
    let files = ["pcap", "json"].map(|extension| away.with_extension(extension));
    for file in &files {
        std::fs::rename(file, hidden.join(file.file_name().unwrap())).unwrap();
    }
    let (library, warnings) = restarted(dir.path());
    assert!(library.runs().is_empty());
    assert!(
        warnings.iter().any(|w| w.contains("away.pcap")),
        "{warnings:?}"
    );
    for file in &files {
        std::fs::rename(hidden.join(file.file_name().unwrap()), file).unwrap();
    }
    assert_eq!(restarted(dir.path()).0.runs().len(), 1);
}

#[test]
fn stale_target_events_are_never_offered_and_built_again() {
    let folder = tempfile::tempdir().unwrap();
    let path = recorded(folder.path(), "capture.pcap");
    std::fs::write(path.with_extension("events.npz"), b"the old events").unwrap();
    std::fs::write(path.with_extension("events.json"), b"{}").unwrap();
    std::fs::write(path.with_extension("vision.json"), br#"{"complete":true}"#).unwrap();
    assert!(owed_events(&path).is_none());
    std::fs::write(
        motion_bench::evidence::reader::layout::stale_marker(&path),
        "screen timestamps regressed",
    )
    .unwrap();
    let run = import(&path).unwrap();
    for panel in ["trajectory14", "full49"] {
        assert!(
            run.missing[panel]
                .ends_with("failed so none of them are scored, screen timestamps regressed"),
            "{}",
            run.missing[panel]
        );
    }
    assert!(owed_events(&path).is_some());
}

/// what copying and sending the whole run list costs a live frame, by library size
#[test]
#[ignore = "a measurement, run by hand with --nocapture"]
fn measure_the_run_list_per_frame() {
    let sidecar: serde_json::Value = serde_json::from_slice(
        &std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../tests/fixtures/recording/sidecar.json"
        ))
        .unwrap(),
    )
    .unwrap();
    let folder = tempfile::tempdir().unwrap();
    let run = import(&recorded(folder.path(), "capture.pcap")).unwrap();
    let mut result = jobs::tests::row("texture19");
    result.w1 = vec![0.1; 19];
    result.metadata = serde_json::json!({ "sources": [sidecar.clone(), sidecar.clone()] });
    for (n, with_results) in [(20, false), (20, true), (100, true), (500, true)] {
        let runs: Vec<Run> = (0..n)
            .map(|_| Run {
                metadata: sidecar.clone(),
                result: with_results.then(|| result.clone()),
                ..run.clone()
            })
            .collect();
        let frames = 200;
        let started = std::time::Instant::now();
        let mut bytes = 0;
        for _ in 0..frames {
            let copied: Vec<Run> = runs
                .iter()
                .map(|run| Run {
                    result: run.result.as_ref().map(ResultRow::summary),
                    ..run.clone()
                })
                .collect();
            bytes = serde_json::to_vec(&copied).unwrap().len();
        }
        let per_frame = started.elapsed() / frames;
        println!(
            "{n} runs, results {with_results}: {bytes} bytes, {per_frame:?} to copy and serialize a frame"
        );
    }
}
