use super::*;
use std::time::Duration;
#[test]
fn a_held_frame_is_released_once_on_every_way_out() {
    let released = std::cell::Cell::new(0);
    let count = || {
        released.set(released.get() + 1);
        Ok(())
    };
    Held(Some(count)).release().unwrap();
    assert_eq!(released.get(), 1);
    let cast_fails = || -> Result<(), String> {
        let _held = Held(Some(count));
        Err("cast failed".into())
    };
    assert!(cast_fails().is_err());
    assert_eq!(released.get(), 2);
    let refused = Held(Some(|| Err("release refused".to_string()))).release();
    assert_eq!(refused.unwrap_err(), "release refused");
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _held = Held(Some(count));
        panic!("preprocessing panicked");
    }));
    assert!(panicked.is_err());
    assert_eq!(released.get(), 3);
}
#[test]
fn stop_during_preparation_returns_without_allowing_an_overlapping_build() {
    assert!(!PREPARING.swap(true, Ordering::AcqRel));
    let preparation = Preparation;
    let (release, wait) = std::sync::mpsc::channel();
    let (finished, done) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let _ = wait.recv_timeout(Duration::from_secs(5));
        drop(preparation);
        finished.send(()).unwrap();
    });
    let mut worker = Worker {
        config: Config::default(),
        status: Arc::new(Mutex::new(Status::default())),
        sink: Arc::new(Mutex::new(None)),
        stop: Arc::new(AtomicBool::new(false)),
        thread: Some(thread),
    };
    let start = Instant::now();
    worker.stop();
    assert!(start.elapsed() < Duration::from_millis(500));
    assert!(worker.stop.load(Ordering::Relaxed));
    let model = tempfile::NamedTempFile::new().unwrap();
    let config = Config {
        model: model.path().to_str().unwrap().into(),
        ..Config::default()
    };
    assert!(
        Worker::start(config)
            .err()
            .unwrap()
            .contains("still finishing")
    );
    release.send(()).unwrap();
    done.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(!PREPARING.load(Ordering::Acquire));
}
#[test]
fn a_models_digest_is_of_the_bytes_it_read_even_after_the_file_changes() {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("model.onnx");
    std::fs::write(&path, b"weights").unwrap();
    let model = Model::read(path.to_str().unwrap()).unwrap();
    std::fs::write(&path, b"other weights").unwrap();
    assert_eq!(model.bytes, b"weights");
    assert_eq!(
        model.sha256,
        "9a129038d9a00aed0cf6a7ea059ca50a813449061ab87848cf1a13eafdf33b2c"
    );
}

#[test]
fn a_recording_is_told_why_target_capture_failed() {
    let folder = tempfile::tempdir().unwrap();
    let status = Status {
        error: Some("DXGI capture stopped".into()),
        ..Status::default()
    };
    let worker = Worker::with_status(Config::default(), status);
    assert_eq!(
        worker.journal().begin(&folder.path().join("a.pcap")),
        Err("DXGI capture stopped".into())
    );
}

#[test]
fn frame_rates_expire_during_idle_and_are_bounded() {
    let mut rate = Window::default();
    let start = Instant::now();
    for i in 0..120 {
        rate.push_frame(start + Duration::from_millis(i * 16));
    }
    let now = start + Duration::from_secs(2);
    assert!((rate.per_second(now) - 60.0).abs() < 1.0);
    assert_eq!(rate.per_second(now + Duration::from_secs(3)), 0.0);
    assert!(rate.spread(now + SPREAD_SPAN).is_none());
    for _ in 0..20000 {
        rate.push_frame(now);
    }
    assert_eq!(rate.samples.len(), 16384);
}
#[test]
fn the_1_percent_lows_are_the_slowest_frames() {
    let mut rate = Window::default();
    let mut at = Instant::now();
    // frames 4 ms apart, with a 20 ms one every hundred
    for i in 0..1000 {
        at += Duration::from_millis(if i % 100 == 50 { 20 } else { 4 });
        rate.push_frame(at);
    }
    let spread = rate.rate_spread(at).unwrap();
    assert!((spread.worst - 50.0).abs() < 1e-6, "{spread:?}");
    // the first frame has no gap: 999 gaps, 989 of 4 ms and 10 of 20 ms
    let average = 999.0 / (989.0 * 4.0 + 10.0 * 20.0) * 1000.0;
    assert!((spread.average - average).abs() < 1e-6, "{spread:?}");
}
#[test]
#[ignore = "requires a live unscaled 1440p display and the operator's model"]
fn desktop_worker_captures_and_stops() {
    for size in [320, 640] {
        let config = Config {
            model: test_model(size),
            ..Config::default()
        };
        let mut worker = Worker::start(config).unwrap();
        let until = Instant::now() + Duration::from_secs(8);
        loop {
            let state = worker.status.locked().clone();
            assert!(state.error.is_none(), "{:?}", state.error);
            if state.ready && state.frames > 0 {
                break;
            }
            assert!(Instant::now() < until, "no screen frame arrived");
            std::thread::sleep(Duration::from_millis(20));
        }
        let state = worker.status.locked().clone();
        assert_eq!(state.throttled, 0);
        assert!(
            state.transport.contains("CUDA graph"),
            "{:?}",
            state.optimization_warning
        );
        assert!(state.optimization_warning.is_none());
        println!(
            "DXGI: {} frames; latest pipeline {:.3} ms",
            state.frames,
            state.last.unwrap().pipeline_ms
        );
        worker.stop();
        assert!(!worker.status.locked().running);
    }
}
#[test]
#[ignore = "requires a live unscaled 1440p display and a cached TensorRT 320px engine"]
fn tensorrt_desktop_worker_captures_and_preserves_provider_evidence() {
    let config = Config {
        provider: "TensorRT".into(),
        model: test_model(320),
        ..Config::default()
    };
    let mut worker = Worker::start(config).unwrap();
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        let state = worker.snapshot();
        assert!(state.error.is_none(), "{:?}", state.error);
        if state.ready && state.frames > 0 {
            break;
        }
        assert!(Instant::now() < until, "no TensorRT screen frame arrived");
        std::thread::sleep(Duration::from_millis(20));
    }
    let directory = tempfile::tempdir().unwrap();
    let pcap = directory.path().join("test.pcap");
    worker.journal().begin(&pcap).unwrap();
    worker.stop();
    let state = worker.snapshot();
    assert!(!state.running);
    assert!(state.optimization_warning.is_none());
    assert_eq!(state.throttled, 0);
    let evidence: Evidence =
        serde_json::from_slice(&std::fs::read(pcap.with_extension("vision.json")).unwrap())
            .unwrap();
    assert!(evidence.complete);
    assert_eq!(evidence.provider, "TensorRT");
    assert!(evidence.pipeline.contains("CUDA device tensor"));
    let inference = evidence.inference.unwrap();
    let expected_root = PathBuf::from(std::env::var_os("USERPROFILE").unwrap())
        .canonicalize()
        .unwrap()
        .join(".motion-bench");
    assert_eq!(
        tensorrt::root_path().unwrap().canonicalize().unwrap(),
        expected_root
    );
    assert!(
        tensorrt::runtime_path()
            .unwrap()
            .canonicalize()
            .unwrap()
            .starts_with(&expected_root)
    );
    assert!(
        Path::new(inference["engine_cache"].as_str().unwrap())
            .canonicalize()
            .unwrap()
            .starts_with(&expected_root)
    );
    assert!(
        inference["startup_kernel_events"]["TensorrtExecutionProvider"]
            .as_u64()
            .unwrap()
            > 0
    );
    println!(
        "TensorRT DXGI: {} frames; latest pipeline {:.3} ms",
        state.frames,
        state.last.unwrap().pipeline_ms
    );
}
#[test]
#[ignore = "requires the operator's local ONNX model and CUDA runtime"]
fn provided_model_runs_on_cuda() {
    let config = Config {
        model: test_model(640),
        ..Config::default()
    };
    let mut model = load_model(&config).unwrap();
    println!(
        "input {:?}; output {:?}",
        model.inputs()[0].dtype(),
        model.outputs()[0].dtype()
    );
    let mut times = Vec::new();
    for _ in 0..12 {
        let tensor =
            ort::value::Tensor::from_array(([1usize, 3, 640, 640], vec![0.0f32; 3 * 640 * 640]))
                .unwrap();
        let start = Instant::now();
        let outputs = model.run(ort::inputs![tensor]).unwrap();
        let (shape, values) = outputs[0].try_extract_tensor::<f32>().unwrap();
        assert_eq!(shape[0], 1);
        assert_eq!(shape[1], 5);
        assert!(
            motion_bench::targets::vision::yolo8(
                values,
                shape[1] as usize,
                shape[2] as usize,
                &config
            )
            .unwrap()
            .is_empty()
        );
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    println!("CUDA inference ms: {times:?}");
}
#[test]
#[ignore = "requires the operator's local ONNX model and DirectML runtime"]
fn provided_model_runs_on_directml() {
    let config = Config {
        provider: "DirectML".into(),
        model: test_model(640),
        ..Config::default()
    };
    let mut model = load_model(&config).unwrap();
    for i in 0..12 {
        let tensor =
            ort::value::Tensor::from_array(([1usize, 3, 640, 640], vec![0f32; 3 * 640 * 640]))
                .unwrap();
        let start = Instant::now();
        let outputs = model.run(ort::inputs![tensor]).unwrap();
        let (shape, values) = outputs[0].try_extract_tensor::<f32>().unwrap();
        assert_eq!(shape.as_ref(), &[1, 5, 8400]);
        assert!(
            motion_bench::targets::vision::yolo8(values, 5, 8400, &config)
                .unwrap()
                .is_empty()
        );
        println!(
            "DirectML inference {i}: {:.3} ms",
            start.elapsed().as_secs_f64() * 1000.0
        );
    }
}
