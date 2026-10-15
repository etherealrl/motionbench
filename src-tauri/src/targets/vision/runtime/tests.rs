use super::*;

#[test]
fn incomplete_packaged_runtime_does_not_mask_a_complete_installation() {
    let scratch = tempfile::tempdir().unwrap();
    let packaged = scratch.path().join("packaged");
    let installed = scratch.path().join("installed");
    std::fs::create_dir_all(&packaged).unwrap();
    std::fs::create_dir_all(&installed).unwrap();
    for file in [
        "onnxruntime.dll",
        "onnxruntime_providers_shared.dll",
        "onnxruntime_providers_cuda.dll",
    ] {
        std::fs::write(packaged.join(file), []).unwrap();
        std::fs::write(installed.join(file), []).unwrap();
    }
    std::fs::write(installed.join("onnxruntime_providers_tensorrt.dll"), []).unwrap();
    let roots = [packaged.clone(), installed.clone()];
    assert_eq!(resolve("TensorRT", &roots).unwrap(), installed);
    assert_eq!(resolve("CUDA", &roots).unwrap(), packaged);
    assert!(
        resolve("TensorRT", &roots[..1])
            .unwrap_err()
            .contains("onnxruntime_providers_tensorrt.dll")
    );

    let directml = roots[1].join("directml");
    std::fs::create_dir(&directml).unwrap();
    std::fs::write(directml.join("onnxruntime.dll"), []).unwrap();
    assert!(
        resolve("DirectML", &roots)
            .unwrap_err()
            .contains("DirectML.dll")
    );
    std::fs::write(directml.join("DirectML.dll"), []).unwrap();
    assert_eq!(resolve("DirectML", &roots).unwrap(), directml);
    assert!(
        check_tensorrt(&roots[1])
            .unwrap_err()
            .contains("nvonnxparser_10.dll")
    );
}
