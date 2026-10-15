use motion_bench::analysis::{
    classify::{self, Config},
    dataset::Dataset,
};
use std::{path::Path, sync::atomic::AtomicBool};

#[test]
fn descriptor_panels_match_pinned_upstream() {
    let dataset = Dataset::load(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/events.npz"
    )))
    .unwrap();
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/upstream-parity.json")).unwrap();
    assert_eq!(reference["revision"], classify::REVISION);
    for panel in ["texture19", "trajectory14", "full49"] {
        let (names, rows) = dataset.rows(panel).unwrap();
        let expected: Vec<Vec<f64>> =
            serde_json::from_value(reference["panels"][panel]["real"].clone()).unwrap();
        assert_eq!(rows.len(), expected.len(), "{panel}");
        assert!(
            rows.iter()
                .zip(&expected)
                .all(|(row, want)| row.len() == names.len() && want.len() == names.len()),
            "{panel}"
        );
        for (i, (row, want)) in rows.iter().zip(expected.iter()).enumerate() {
            for (j, (a, b)) in row.iter().zip(want).enumerate() {
                assert!(
                    (a - b).abs() < 1e-6,
                    "{panel} event {i} feature {j}: {a} != {b}"
                );
            }
        }
    }
}

/// the names in the reference source's own tuples, so a column can't be relabelled while its
/// values still match
fn reference_names(file: &str, tuple: &str) -> Vec<String> {
    let source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/reference/abcurves")
            .join(file),
    )
    .unwrap();
    let start = source.find(&format!("{tuple} = (")).unwrap();
    let end = start + source[start..].find(')').unwrap();
    source[start..end]
        .split(['"', '\''])
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

#[test]
fn panels_name_their_columns_as_the_reference_does() {
    let dataset = Dataset::load(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/events.npz"
    )))
    .unwrap();
    let texture = reference_names("texture.py", "TEXTURE_FEATURE_NAMES");
    let trajectory = reference_names("judges.py", "TRAJ_FEATURE_NAMES");
    let target = reference_names("judges.py", "TARGET_EVENT_FEATURE_NAMES");
    let full = [trajectory.clone(), texture.clone(), target].concat();
    assert_eq!((texture.len(), trajectory.len(), full.len()), (19, 14, 49));
    for (panel, want) in [
        ("texture19", texture),
        ("trajectory14", trajectory),
        ("full49", full),
    ] {
        assert_eq!(dataset.rows(panel).unwrap().0, want, "{panel}");
    }
}

#[test]
fn grouped_reports_match_upstream() {
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/upstream-parity.json")).unwrap();
    for panel in ["texture19", "trajectory14", "full49"] {
        let case = &reference["panels"][panel];
        let real: Vec<Vec<f64>> = serde_json::from_value(case["real"].clone()).unwrap();
        let fake: Vec<Vec<f64>> = serde_json::from_value(case["fake"].clone()).unwrap();
        let groups: Vec<String> = serde_json::from_value(case["groups"].clone()).unwrap();
        let config = Config {
            repeats: 2,
            bootstrap: 20,
            permutations: 8,
            ..Config::default()
        };
        let result = classify::run(
            &real,
            &fake,
            &groups,
            &groups,
            &config,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        let expected = case["report"]["auc"].as_f64().unwrap();

        assert!(
            (result.auc - expected).abs() < 0.0011,
            "{panel} AUC {} != {expected}",
            result.auc
        );
        let folds: Vec<Vec<f64>> =
            serde_json::from_value(case["report"]["fold_auc"].clone()).unwrap();
        let null: Vec<f64> =
            serde_json::from_value(case["report"]["permutation_auc"].clone()).unwrap();
        assert_eq!(result.fold_auc.len(), folds.len());
        assert_eq!(result.permutation_auc.len(), null.len());
        let fold_tolerance = if panel == "full49" { 0.0101 } else { 1e-10 };
        for (a, b) in result.fold_auc.iter().flatten().zip(folds.iter().flatten()) {
            assert!((a - b).abs() < fold_tolerance, "{panel} fold {a} != {b}");
        }
        for (a, b) in result.permutation_auc.iter().zip(&null) {
            assert!((a - b).abs() < 0.0033, "{panel} null {a} != {b}");
        }
        let ci: Vec<f64> = serde_json::from_value(case["report"]["auc_ci"].clone()).unwrap();
        for (a, b) in result.auc_ci.unwrap().iter().zip(ci) {
            assert!((a - b).abs() < 1e-10, "{panel} bootstrap {a} != {b}");
        }
        assert!(
            (result.p_value.unwrap() - case["report"]["permutation_pvalue"].as_f64().unwrap())
                .abs()
                < 1e-10
        );
    }
}

#[test]
fn incomplete_event_geometry_disables_only_the_affected_panels() {
    use std::io::{Read, Write};
    let source = Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/events.npz"
    ));
    for missing in ["target_radius", "target_rel_y_at_B", "prefix_mask"] {
        let path = std::env::temp_dir().join(format!(
            "mpm-event-missing-{}-{missing}.npz",
            std::process::id()
        ));
        let mut input = zip::ZipArchive::new(std::fs::File::open(source).unwrap()).unwrap();
        let mut output = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for i in 0..input.len() {
            let mut file = input.by_index(i).unwrap();
            if file.name() == format!("{missing}.npy") {
                continue;
            }
            let name = file.name().to_string();
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).unwrap();
            output
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            output.write_all(&bytes).unwrap();
        }
        output.finish().unwrap();
        let dataset = Dataset::load(&path).unwrap();
        assert!(dataset.rows("texture19").is_ok());
        let affected = if missing == "prefix_mask" {
            "full49"
        } else {
            "trajectory14"
        };
        assert!(dataset.rows(affected).err().unwrap().contains(missing));
        if missing == "prefix_mask" {
            assert!(dataset.rows("trajectory14").is_ok());
        }
        std::fs::remove_file(path).unwrap();
    }
}
