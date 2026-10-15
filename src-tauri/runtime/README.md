# Inference runtimes

Run `tools/vision/setup-vision.ps1` from the repository before a desktop build. It verifies and extracts official x64 CUDA and DirectML ONNX Runtime packages. DLLs and their notices are bundled by Tauri and ignored by Git.

CUDA uses ONNX Runtime 1.27.1 plus the permanent CUDA 13.3 / cuDNN 9.22 system installation. DirectML uses ONNX Runtime 1.24.4 and DirectML 1.15.4 under `directml/`. See `docs/target-capture.md` for installation and provider selection.
