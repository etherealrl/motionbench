# third-party notices

- the judges and the reference files in the tests come from [ABCurves](https://github.com/optima-manent/ABCurves/tree/e197441f8d343d3b4a4069c7cae20764d0953d15), MIT, with [its license](../licenses/abcurves.LICENSE) kept
- the boxes go through [anybox](https://github.com/etherealrl/anybox), MPL-2.0, which drives the Medius through the [medius](https://github.com/K4HVH/medius) crate
- the UI components are copied from [shadcn/ui](https://github.com/shadcn-ui/ui), MIT, with [its license](../licenses/shadcn.LICENSE) kept. Radix and Lucide keep their own package licenses
- Inter comes from [Fontsource](https://fontsource.org/fonts/inter) and is bundled under the SIL Open Font License, [its license](https://github.com/rsms/inter/blob/master/LICENSE.txt)
- hidapi rebuilds a mouse's report layout from what Windows knows about it, under its own licenses
- target capture uses ONNX Runtime 1.27.1 (CUDA 13) and 1.24.4 (DirectML) through the MIT `ort` bindings, and DirectML 1.15.4. their licenses and notices ship next to their DLLs
- CUDA 13.3.1, cuDNN 9.22.0.52 and TensorRT 10.14.1.48 are NVIDIA's, installed separately under NVIDIA's licenses and never bundled. TensorRT's license sits next to it in `%USERPROFILE%\.motion-bench\runtimes\tensorrt\10.14.1.48`
- a model you pick keeps its own license, and engines are built on your PC
- Tauri, React, Vite, Tailwind and the rest keep their own licenses. NumPy and PyTorch only generate the test fixtures

the exact versions are in `Cargo.lock` and `bun.lock`
