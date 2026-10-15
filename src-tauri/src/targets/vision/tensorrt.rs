use motion_bench::targets::vision::Config;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const VERSION: &str = "10.14.1.48";
pub struct Cache {
    pub path: PathBuf,
}
pub fn root_path() -> Result<PathBuf, String> {
    let root = std::env::var_os("USERPROFILE").ok_or("USERPROFILE is unavailable")?;
    Ok(PathBuf::from(root).join(".motion-bench"))
}
pub fn runtime_path() -> Result<PathBuf, String> {
    Ok(root_path()?.join("runtimes").join("tensorrt").join(VERSION))
}
pub fn cache(
    config: &Config,
    model_sha256: &str,
    ort: &Path,
    graph: bool,
) -> Result<Cache, String> {
    let runtime = runtime_path()?;
    let provenance = json!({
        "schema":"motion-bench.tensorrt.v1", "tensorrt_version":VERSION,
        "model_sha256":model_sha256,
        "ort_sha256":hash_file(&ort.join("onnxruntime.dll"))?,
        "provider_sha256":hash_file(&ort.join("onnxruntime_providers_tensorrt.dll"))?,
        "tensorrt_sha256":hash_file(&runtime.join("nvinfer_10.dll"))?,
        "parser_sha256":hash_file(&runtime.join("nvonnxparser_10.dll"))?,
        "gpu":crate::targets::cuda::device_identity(config.cuda_device)?,
        "fp16":config.tensorrt_fp16, "cuda_graph":graph,
        "workspace_bytes":1usize<<30, "builder_optimization_level":5,
        "auxiliary_streams":0, "cuda":"13.3", "cudnn":"9.22",
    });
    let key = cache_key(&provenance);
    let path = root_path()?.join("engines").join(key);
    std::fs::create_dir_all(&path).map_err(|e| format!("TensorRT cache: {e}"))?;
    std::fs::write(
        path.join("configuration.json"),
        serde_json::to_vec_pretty(&provenance).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    Ok(Cache { path })
}
/// hashed once a process, because Windows keeps a loaded dll's bytes for the process even if the
/// file is renamed and replaced. onnxruntime.dll is loaded before this, the others with the first
/// session
// ponytail: a dll swapped after a first session that failed before loading it keeps its old
// digest, cache only after a session commits if that ever matters
fn hash_file(path: &Path) -> Result<String, String> {
    use motion_bench::sync::LockExt;
    static HASHED: std::sync::Mutex<std::collections::BTreeMap<PathBuf, String>> =
        std::sync::Mutex::new(std::collections::BTreeMap::new());
    if let Some(digest) = HASHED.locked().get(path) {
        return Ok(digest.clone());
    }
    let digest = super::sha256(path.to_str().ok_or("runtime path is not UTF-8")?)?;
    HASHED.locked().insert(path.into(), digest.clone());
    Ok(digest)
}
fn cache_key(provenance: &Value) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(provenance).unwrap())
    )
}
pub fn finish_profile(model: &mut ort::session::Session) -> Result<Value, String> {
    let path = model.end_profiling().map_err(|e| e.to_string())?;
    let events: Vec<Value> =
        serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let mut providers = std::collections::BTreeMap::<String, usize>::new();
    for event in events {
        if let Some(provider) = event["args"]["provider"].as_str() {
            *providers.entry(provider.into()).or_default() += 1;
        }
    }
    if !providers.contains_key("TensorrtExecutionProvider") {
        return Err("TensorRT was picked but none of its kernels ran, so pick CUDA or check which operators the model uses".into());
    }
    let cache = Path::new(&path)
        .parent()
        .ok_or("profiling path has no cache directory")?;
    let configuration: Value = serde_json::from_slice(
        &std::fs::read(cache.join("configuration.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(
        json!({"startup_kernel_events":providers, "startup_profile":path, "engine_cache":cache, "configuration":configuration}),
    )
}

#[cfg(test)]
mod tests;
