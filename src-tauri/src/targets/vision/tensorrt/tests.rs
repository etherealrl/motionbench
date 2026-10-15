use super::*;
#[test]
fn changed_model_runtime_gpu_or_precision_never_reuses_an_engine_key() {
    let base = json!({"model":"a", "runtime":"b", "gpu":"c", "fp16":true, "graph":true});
    for field in ["model", "runtime", "gpu", "fp16", "graph"] {
        let mut changed = base.clone();
        changed[field] = json!("changed");
        assert_ne!(cache_key(&base), cache_key(&changed));
    }
    assert_eq!(cache_key(&base), cache_key(&base.clone()));
}
