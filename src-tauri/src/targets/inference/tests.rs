use super::*;
use windows::Win32::{
    Foundation::HMODULE,
    Graphics::{
        Direct3D::D3D_DRIVER_TYPE_UNKNOWN,
        Direct3D11::*,
        Dxgi::Common::*,
        Dxgi::{CreateDXGIFactory1, IDXGIFactory1},
    },
};

#[test]
#[ignore = "requires NVIDIA TensorRT, CUDA and the operator's 320px model"]
fn tensorrt_fp32_and_fp16_match_cuda_detections_and_follow_changing_frames() {
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.unwrap();
    let adapter = unsafe { factory.EnumAdapters1(0) }.unwrap();
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            &adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .unwrap();
    let (device, context) = (device.unwrap(), context.unwrap());
    let config = Config {
        model: crate::targets::vision::test_model(320),
        ..Config::default()
    };
    let file = Model::read(&config.model).unwrap();
    let baseline_preprocess = Preprocess::new(&device, 2560, 1440, 320).unwrap();
    let mut baseline = Engine::new(
        load_model_options(&config, &file, true, false).unwrap(),
        &file,
        &config,
        &adapter,
        &baseline_preprocess,
    )
    .unwrap();
    for fp16 in [false, true] {
        let preprocess = Preprocess::new(&device, 2560, 1440, 320).unwrap();
        let config = Config {
            provider: "TensorRT".into(),
            tensorrt_fp16: fp16,
            cuda_graph: fp16,
            ..config.clone()
        };
        let mut engine = Engine::new(
            load_model_options(&config, &file, config.cuda_graph, false).unwrap(),
            &file,
            &config,
            &adapter,
            &preprocess,
        )
        .unwrap();
        assert!(engine.gpu.is_some());
        assert!(
            engine.inference.as_ref().unwrap()["startup_kernel_events"]["TensorrtExecutionProvider"]
                .as_u64()
                .unwrap() > 0
        );
        let mut previous = None;
        for step in 0..3i32 {
            let mut pixels = [25u8, 25, 25, 255].repeat(2560 * 1440);
            for y in 560..760usize {
                for x in 1100..1450usize {
                    if (x as i32 - 1234 - step * 35).pow(2) + (y as i32 - 654).pow(2) <= 80 * 80 {
                        pixels[(y * 2560 + x) * 4..(y * 2560 + x + 1) * 4]
                            .copy_from_slice(&[255, 140, 0, 255]);
                    }
                }
            }
            let desc = D3D11_TEXTURE2D_DESC {
                Width: 2560,
                Height: 1440,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                ..Default::default()
            };
            let data = D3D11_SUBRESOURCE_DATA {
                pSysMem: pixels.as_ptr().cast(),
                SysMemPitch: 2560 * 4,
                SysMemSlicePitch: 2560 * 1440 * 4,
            };
            let mut texture = None;
            unsafe { device.CreateTexture2D(&desc, Some(&data), Some(&mut texture)) }.unwrap();
            preprocess.copy(&context, texture.as_ref().unwrap());
            baseline_preprocess.copy(&context, texture.as_ref().unwrap());
            baseline.prepare(&baseline_preprocess, &context).unwrap();
            let expected = baseline.infer().unwrap().to_vec();
            engine.prepare(&preprocess, &context).unwrap();
            let actual = engine.infer().unwrap().to_vec();
            assert!(actual.iter().all(|v| v.is_finite()));
            let expected = motion_bench::targets::vision::yolo8(
                &expected,
                baseline.channels,
                baseline.candidates,
                &config,
            )
            .unwrap();
            let detected = motion_bench::targets::vision::yolo8(
                &actual,
                engine.channels,
                engine.candidates,
                &config,
            )
            .unwrap();
            assert_eq!(expected.len(), 1);
            assert_eq!(detected.len(), expected.len());
            assert_eq!(detected[0].class, expected[0].class);
            let max_delta = detected[0]
                .bounds
                .iter()
                .zip(expected[0].bounds)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f64::max);
            assert!(
                max_delta < 1.0,
                "FP16={fp16}, box difference {max_delta} input pixels"
            );
            assert!((detected[0].confidence - expected[0].confidence).abs() < 0.03);
            if let Some(previous) = previous {
                assert_ne!(actual, previous, "TensorRT replayed a stale image");
            }
            previous = Some(actual);
            println!(
                "TensorRT FP16={fp16}, frame {step}: max box difference {max_delta:.4} input pixels"
            );
        }
    }
}

#[test]
#[ignore = "requires NVIDIA CUDA and the operator's original and converted models"]
fn graph_matches_standard_and_updates_changing_images() {
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.unwrap();
    let adapter = unsafe { factory.EnumAdapters1(0) }.unwrap();
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            &adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .unwrap();
    let (device, context) = (device.unwrap(), context.unwrap());
    for size in [320, 640] {
        let config = Config {
            model: crate::targets::vision::test_model(size),
            ..Config::default()
        };
        let preprocess = Preprocess::new(&device, 64, 32, size).unwrap();
        let file = Model::read(&config.model).unwrap();
        let model = load_model_options(&config, &file, true, false).unwrap();
        let mut graph = Engine::new(model, &file, &config, &adapter, &preprocess).unwrap();
        assert!(graph.gpu.is_some(), "{:?}", graph.warning);
        let config = Config {
            cuda_graph: false,
            ..config
        };
        let model = load_model_options(&config, &file, false, false).unwrap();
        let mut standard = Engine::new(model, &file, &config, &adapter, &preprocess).unwrap();
        let mut previous = None;
        for step in 0..3 {
            let mut pixels = [25u8, 25, 25, 255].repeat(64 * 32);
            for y in 0..32usize {
                for x in 0..64usize {
                    if (x as i32 - 20 - step * 10).pow(2) + (y as i32 - 16).pow(2) <= 7 * 7 {
                        pixels[(y * 64 + x) * 4..(y * 64 + x + 1) * 4]
                            .copy_from_slice(&[255, 140, 0, 255]);
                    }
                }
            }
            let desc = D3D11_TEXTURE2D_DESC {
                Width: 64,
                Height: 32,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                ..Default::default()
            };
            let data = D3D11_SUBRESOURCE_DATA {
                pSysMem: pixels.as_ptr().cast(),
                SysMemPitch: 64 * 4,
                SysMemSlicePitch: 64 * 32 * 4,
            };
            let mut texture = None;
            unsafe { device.CreateTexture2D(&desc, Some(&data), Some(&mut texture)) }.unwrap();
            preprocess.copy(&context, texture.as_ref().unwrap());
            graph.prepare(&preprocess, &context).unwrap();
            let actual = graph.infer().unwrap().to_vec();
            standard.prepare(&preprocess, &context).unwrap();
            let expected = standard.infer().unwrap();
            let mut max_delta = 0f32;
            for (&a, &b) in actual.iter().zip(expected) {
                assert!(a.is_finite() && b.is_finite());
                max_delta = max_delta.max((a - b).abs());
                assert!(
                    (a - b).abs() <= 0.02 + b.abs() * 0.0005,
                    "{size}: graph {a} vs standard {b}"
                );
            }
            println!("{size}px frame {step}: max absolute output difference {max_delta}");
            if let Some(previous) = previous {
                assert_ne!(actual, previous, "graph reused stale image data");
            }
            previous = Some(actual);
        }
    }
}
