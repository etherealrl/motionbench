use std::path::{Path, PathBuf};

pub fn resolve(provider: &str, roots: &[PathBuf]) -> Result<PathBuf, String> {
    let required: &[&str] = match provider {
        "TensorRT" => &[
            "onnxruntime.dll",
            "onnxruntime_providers_shared.dll",
            "onnxruntime_providers_cuda.dll",
            "onnxruntime_providers_tensorrt.dll",
        ],
        "CUDA" => &[
            "onnxruntime.dll",
            "onnxruntime_providers_shared.dll",
            "onnxruntime_providers_cuda.dll",
        ],
        "DirectML" => &["onnxruntime.dll", "DirectML.dll"],
        _ => &["onnxruntime.dll"],
    };
    let candidates = roots.iter().map(|root| {
        if provider == "DirectML" {
            root.join("directml")
        } else {
            root.clone()
        }
    });
    let mut failures = Vec::new();
    for candidate in candidates {
        let missing = missing(&candidate, required);
        if missing.is_empty() {
            return Ok(candidate);
        }
        failures.push(format!(
            "{}: missing {}",
            candidate.display(),
            missing.join(", ")
        ));
    }
    Err(format!(
        "ONNX Runtime for {provider} is missing files, so run tools/vision/setup-vision.ps1 from the source (docs/target-capture.md says how) and restart the app. it looked in {}",
        failures.join("; ")
    ))
}

fn missing<'a>(directory: &Path, files: &[&'a str]) -> Vec<&'a str> {
    files
        .iter()
        .copied()
        .filter(|file| !directory.join(file).is_file())
        .collect()
}

pub fn check_tensorrt(directory: &Path) -> Result<(), String> {
    let missing = missing(
        directory,
        &[
            "nvinfer_10.dll",
            "nvonnxparser_10.dll",
            "nvinfer_plugin_10.dll",
        ],
    );
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "the TensorRT install at {} is missing {}, so run tools/vision/setup-tensorrt.ps1 from the source and restart the app",
            directory.display(),
            missing.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests;
