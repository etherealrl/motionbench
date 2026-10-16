use motion_bench::sync::LockExt;
use motion_bench::targets::vision::{Config, Evidence, Frame, VISION_SCHEMA};
use serde::Serialize;
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Instant,
};
#[cfg(windows)]
mod runtime;
#[cfg(windows)]
mod tensorrt;

#[derive(Clone, Default, Serialize)]
pub struct Status {
    #[serde(skip)]
    pub(crate) experiment_wakeup: Option<std::sync::mpsc::SyncSender<()>>,
    pub running: bool,
    pub ready: bool,
    pub frames: u64,
    pub skipped: u64,
    pub provider: String,
    pub model_shape: String,
    pub transport: String,
    pub optimization_warning: Option<String>,
    pub inference: Option<serde_json::Value>,
    pub last: Option<Frame>,
    pub recording: bool,
    pub error: Option<String>,
    pub clock: Option<motion_bench::targets::vision::Clock>,
    pub fps: f64,
    pub capture_fps: f64,
    /// over the last ten seconds, in frames per second, the worst being the 1% lows
    pub analyzed: Option<Spread>,
    pub captured: Option<Spread>,
    /// over the last ten seconds, in milliseconds, the worst being the slowest 1%
    pub pipeline: Option<Spread>,
    pub frame_age_ms: Option<f64>,
    /// the captured display's size, zero until it's open
    pub viewport: [u32; 2],
    pub throttled: u64,
    /// the digest of the model bytes the worker loaded
    #[serde(skip)]
    pub(crate) model_sha256: Option<String>,
    pub event_ready: bool,
    #[serde(skip)]
    analyzed_rate: Window,
    #[serde(skip)]
    capture_rate: Window,
    #[serde(skip)]
    pipeline_ms: Window,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Spread {
    pub average: f64,
    pub worst: f64,
}
/// the rate shown now is over the last two seconds, the spread over the last ten
const RATE_SPAN: std::time::Duration = std::time::Duration::from_secs(2);
const SPREAD_SPAN: std::time::Duration = std::time::Duration::from_secs(10);
/// the last ten seconds of a stream: when each sample came and its value
#[derive(Default)]
struct Window {
    started: Option<Instant>,
    samples: std::collections::VecDeque<(Instant, f64)>,
}
/// a copy of the status carries the numbers read from the windows, not their samples, so the
/// copy made for every frame of the ui stays small
impl Clone for Window {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl Window {
    fn push(&mut self, now: Instant, value: f64) {
        self.started.get_or_insert(now);
        self.samples.push_back((now, value));
        while self
            .samples
            .front()
            .is_some_and(|(t, _)| now.duration_since(*t) > SPREAD_SPAN)
            || self.samples.len() > 16384
        {
            self.samples.pop_front();
        }
    }
    /// a frame, its value the milliseconds since the one before
    fn push_frame(&mut self, now: Instant) {
        let gap = self.samples.back().map_or(f64::NAN, |(t, _)| {
            now.duration_since(*t).as_secs_f64() * 1000.0
        });
        self.push(now, gap);
    }
    fn last(&self) -> Option<Instant> {
        self.samples.back().map(|(t, _)| *t)
    }
    fn per_second(&self, now: Instant) -> f64 {
        let duration = self
            .started
            .map_or(0.0, |t| now.duration_since(t).min(RATE_SPAN).as_secs_f64());
        if duration < 0.1 {
            return 0.0;
        }
        self.samples
            .iter()
            .filter(|(t, _)| now.duration_since(*t) < RATE_SPAN)
            .count() as f64
            / duration
    }
    /// the average and the mean of the highest 1% over the last ten seconds
    fn spread(&self, now: Instant) -> Option<Spread> {
        let mut values: Vec<f64> = self
            .samples
            .iter()
            .filter(|(t, v)| now.duration_since(*t) < SPREAD_SPAN && v.is_finite())
            .map(|(_, v)| *v)
            .collect();
        if values.is_empty() {
            return None;
        }
        let average = values.iter().sum::<f64>() / values.len() as f64;
        let worst = values.len().div_ceil(100);
        values.select_nth_unstable_by(worst - 1, |a, b| b.total_cmp(a));
        Some(Spread {
            average,
            worst: values[..worst].iter().sum::<f64>() / worst as f64,
        })
    }
    /// the spread of the gaps between frames, as frames per second
    fn rate_spread(&self, now: Instant) -> Option<Spread> {
        self.spread(now).map(|gaps| Spread {
            average: 1000.0 / gaps.average,
            worst: 1000.0 / gaps.worst,
        })
    }
}
struct Sink {
    path: PathBuf,
    writer: BufWriter<File>,
    evidence: Evidence,
    started: u64,
}
static PREPARING: AtomicBool = AtomicBool::new(false);
/// a model is still being prepared, maybe for a worker already stopped; a new one waits for it
pub fn preparing() -> bool {
    PREPARING.load(Ordering::Acquire)
}
struct Preparation;
impl Drop for Preparation {
    fn drop(&mut self) {
        PREPARING.store(false, Ordering::Release);
    }
}
/// what a recording needs of target capture, held apart from the worker so the recording's
/// files are made and finished without the app's lock
#[derive(Clone)]
pub struct Journal {
    config: Config,
    status: Arc<Mutex<Status>>,
    sink: Arc<Mutex<Option<Sink>>>,
}
impl Journal {
    pub fn begin(&self, pcap: &Path) -> Result<(), String> {
        let status = self.status.locked().clone();
        if let Some(error) = status.error {
            return Err(error);
        }
        if !status.ready {
            return Err("the target model wasn't ready yet".into());
        }
        let evidence = Evidence {
            schema: VISION_SCHEMA.into(),
            config: self.config.clone(),
            screen: status.viewport,
            model_sha256: status
                .model_sha256
                .ok_or("target capture has no model loaded")?,
            provider: status.provider,
            pipeline: status.transport,
            inference: status.inference,
            complete: false,
            error: None,
            clock: status.clock,
        };
        std::fs::write(
            pcap.with_extension("vision.json"),
            serde_json::to_vec_pretty(&evidence).unwrap(),
        )
        .map_err(|e| e.to_string())?;
        let sink = Sink {
            path: pcap.into(),
            writer: BufWriter::new(
                File::create(pcap.with_extension("targets.jsonl")).map_err(|e| e.to_string())?,
            ),
            evidence,
            started: motion_bench::capture::session::wall_us(),
        };
        let mut slot = self.sink.locked();
        if slot.is_some() {
            return Err("screen evidence is already recording".into());
        }
        *slot = Some(sink);
        self.status.locked().recording = true;
        Ok(())
    }
    pub fn finish(&self) -> Result<(), String> {
        // taken out first, so the frame writer stops writing to it and never waits on the flush
        let Some(mut recording) = self.sink.locked().take() else {
            return Ok(());
        };
        let error = self
            .status
            .locked()
            .error
            .clone()
            .or_else(|| recording.writer.flush().err().map(|e| e.to_string()))
            .or_else(|| {
                recording
                    .writer
                    .get_ref()
                    .sync_all()
                    .err()
                    .map(|e| e.to_string())
            });
        recording.evidence.complete = error.is_none();
        recording.evidence.error = error.clone();
        std::fs::write(
            recording.path.with_extension("vision.json"),
            serde_json::to_vec_pretty(&recording.evidence).unwrap(),
        )
        .map_err(|e| e.to_string())?;
        self.status.locked().recording = false;
        error.map_or(Ok(()), Err)
    }
}

pub struct Worker {
    pub config: Config,
    pub status: Arc<Mutex<Status>>,
    sink: Arc<Mutex<Option<Sink>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Worker {
    pub fn start(config: Config) -> Result<Self, String> {
        config.validate()?;
        if !Path::new(&config.model).is_file() {
            return Err("choose an existing ONNX target model".into());
        }
        if PREPARING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("previous model preparation is still finishing in the background, retry after it completes".into());
        }
        let preparation = Preparation;
        let status = Arc::new(Mutex::new(Status {
            running: true,
            viewport: [0, 0],
            event_ready: !config.scenario.trim().is_empty(),
            ..Status::default()
        }));
        let sink = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let (s, k, c, settings) = (status.clone(), sink.clone(), stop.clone(), config.clone());
        let thread = std::thread::spawn(move || {
            let mut preparation = Some(preparation);
            // a panic ends target capture with a word too, or it would still look ready and
            // its recording's evidence would finish as whole
            let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run(&settings, &s, &k, &c, &mut preparation)
            }))
            .unwrap_or_else(|_| Err("target capture stopped unexpectedly".into()));
            if let Err(error) = ran {
                s.locked().error = Some(error);
            }
            let mut status = s.locked();
            status.running = false;
            status.ready = false;
        });
        Ok(Self {
            config,
            status,
            sink,
            stop,
            thread: Some(thread),
        })
    }
    /// a worker with no thread that reports `status`, for tests of what depends on it
    #[cfg(test)]
    pub(crate) fn with_status(config: Config, status: Status) -> Self {
        Self {
            config,
            status: Arc::new(Mutex::new(status)),
            sink: Arc::new(Mutex::new(None)),
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }
    pub fn snapshot(&self) -> Status {
        let held = self.status.locked();
        let mut state = held.clone();
        let now = Instant::now();
        if held.running {
            state.fps = held.analyzed_rate.per_second(now);
            state.capture_fps = held.capture_rate.per_second(now);
            state.analyzed = held.analyzed_rate.rate_spread(now);
            state.captured = held.capture_rate.rate_spread(now);
            state.pipeline = held.pipeline_ms.spread(now);
        }
        state.frame_age_ms = held
            .analyzed_rate
            .last()
            .map(|t| now.duration_since(t).as_secs_f64() * 1000.0);
        state
    }
    pub fn journal(&self) -> Journal {
        Journal {
            config: self.config.clone(),
            status: self.status.clone(),
            sink: self.sink.clone(),
        }
    }
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let ready = self.status.locked().ready;
        // ORT engine compilation cannot be interrupted; its worker owns the remaining resources
        if let Some(thread) = self.thread.take()
            && (ready || thread.is_finished())
        {
            let _ = thread.join();
        }
        let _ = self.journal().finish();
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}
/// a target model's bytes, read once so its digest is of exactly what was loaded, even if the
/// file changes after
pub(crate) struct Model {
    bytes: Vec<u8>,
    pub sha256: String,
}
impl Model {
    pub fn read(path: &str) -> Result<Self, String> {
        use sha2::{Digest, Sha256};
        let bytes =
            std::fs::read(path).map_err(|e| format!("the target model can't be read: {e}"))?;
        Ok(Self {
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            bytes,
        })
    }
}

fn sha256(path: &str) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut bytes = [0u8; 65536];
    loop {
        let n = file.read(&mut bytes).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// the operator's model for the tests that need a GPU, `v8_<size>_target.onnx` in the folder
/// MOTION_BENCH_TEST_MODELS names
#[cfg(all(test, windows))]
pub(crate) fn test_model(size: usize) -> String {
    let folder = std::env::var_os("MOTION_BENCH_TEST_MODELS")
        .expect("set MOTION_BENCH_TEST_MODELS to the folder with the test models");
    Path::new(&folder)
        .join(format!("v8_{size}_target.onnx"))
        .display()
        .to_string()
}

#[cfg(all(test, windows))]
fn load_model(config: &Config) -> Result<ort::session::Session, String> {
    load_model_options(config, &Model::read(&config.model)?, false, false)
}

#[cfg(windows)]
pub(crate) fn load_model_options(
    config: &Config,
    file: &Model,
    graph: bool,
    nhwc: bool,
) -> Result<ort::session::Session, String> {
    static FAMILY: Mutex<Option<String>> = Mutex::new(None);
    let mut family = FAMILY.locked();
    let requested = if config.provider == "DirectML" {
        "directml"
    } else {
        "cuda"
    };
    if family.as_ref().is_some_and(|loaded| loaded != requested) {
        return Err(
            "restart the app to switch between TensorRT/CUDA/CPU and DirectML runtimes".into(),
        );
    }
    use ort::{ep, session::Session};
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::System::LibraryLoader::{
            AddDllDirectory, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_USER_DIRS,
            SetDefaultDllDirectories,
        },
        core::PCWSTR,
    };
    let packaged = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .unwrap()
        .join("runtime");
    let mut roots = vec![
        packaged,
        tensorrt::root_path()?.join("runtimes").join("onnx"),
    ];
    // the checkout's runtime, for a dev build run from the source folder
    if cfg!(debug_assertions) {
        roots.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("runtime"));
    }
    let runtime = runtime::resolve(&config.provider, &roots)?;
    let toolkit = if matches!(config.provider.as_str(), "CUDA" | "TensorRT") {
        let [cuda, cudnn] = crate::targets::cuda::toolkit()?;
        if !cuda.join("cudart64_13.dll").is_file() || !cudnn.join("cudnn64_9.dll").is_file() {
            return Err(format!(
                "CUDA 13.3 or cuDNN 9.22 isn't in {} or {}, so follow docs/target-capture.md and then restart target capture",
                cuda.display(),
                cudnn.display()
            ));
        }
        vec![cuda, cudnn]
    } else {
        Vec::new()
    };
    unsafe {
        SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_DEFAULT_DIRS | LOAD_LIBRARY_SEARCH_USER_DIRS)
    }
    .map_err(|e| e.to_string())?;
    let trt = tensorrt::runtime_path()?;
    if config.provider == "TensorRT" {
        runtime::check_tensorrt(&trt)?;
    }
    for directory in [&runtime, &trt].into_iter().chain(&toolkit) {
        if directory.is_dir() {
            let wide = directory
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>();
            if unsafe { AddDllDirectory(PCWSTR(wide.as_ptr())) }.is_null() {
                return Err(windows::core::Error::from_thread().to_string());
            }
        }
    }
    ort::init_from(runtime.join("onnxruntime.dll"))
        .map_err(|e| e.to_string())?
        .commit();
    *family = Some(requested.into());
    let builder = Session::builder()
        .map_err(|e| e.to_string())?
        .with_intra_threads(2)
        .map_err(|e| e.to_string())?;
    let mut builder = match config.provider.as_str() {
        "TensorRT" => {
            let cache = tensorrt::cache(config, &file.sha256, &runtime, graph)?;
            builder
                .with_profiling(cache.path.join("startup"))
                .map_err(|e| e.to_string())?
                .with_execution_providers([
                    ep::TensorRT::default()
                        .with_device_id(config.cuda_device as i32)
                        .with_fp16(config.tensorrt_fp16)
                        .with_max_workspace_size(1 << 30)
                        .with_builder_optimization_level(5)
                        .with_auxiliary_streams(0)
                        .with_engine_cache(true)
                        .with_engine_cache_path(cache.path.to_string_lossy())
                        .with_timing_cache(true)
                        .with_timing_cache_path(cache.path.to_string_lossy())
                        .with_cuda_graph(graph)
                        .build()
                        .error_on_failure(),
                    ep::CUDA::default()
                        .with_device_id(config.cuda_device as i32)
                        .with_tf32(true)
                        .build()
                        .error_on_failure(),
                ])
                .map_err(|e| format!("TensorRT initialization: {e}"))?
        }
        "CUDA" => builder
            .with_execution_providers([ep::CUDA::default()
                .with_device_id(config.cuda_device as i32)
                .with_tf32(true)
                .with_cuda_graph(graph)
                .with_prefer_nhwc(nhwc)
                .with_conv_algorithm_search(ep::cuda::ConvAlgorithmSearch::Exhaustive)
                .build()
                .error_on_failure()])
            .map_err(|e| e.to_string())?,
        "CPU" => builder,
        "DirectML" => builder
            .with_memory_pattern(false)
            .map_err(|e| e.to_string())?
            .with_parallel_execution(false)
            .map_err(|e| e.to_string())?
            .with_execution_providers([ep::DirectML::default()
                .with_device_id(config.adapter as i32)
                .build()
                .error_on_failure()])
            .map_err(|e| e.to_string())?,
        _ => return Err("choose TensorRT, CUDA, DirectML or CPU inference".into()),
    };
    builder
        .commit_from_memory(&file.bytes)
        .map_err(|e| format!("ONNX/{}: {e}", config.provider))
}

#[cfg(windows)]
pub(crate) fn finish_tensorrt_profile(
    model: &mut ort::session::Session,
) -> Result<serde_json::Value, String> {
    tensorrt::finish_profile(model)
}

/// the screen's duplication. Windows refuses it with access denied while the desktop isn't one
/// it can copy yet, as when target capture starts with the app or a UAC prompt is up, so that
/// refusal is waited out for a few seconds
#[cfg(windows)]
fn duplicate(
    output: &windows::Win32::Graphics::Dxgi::IDXGIOutput1,
    device: &windows::Win32::Graphics::Direct3D11::ID3D11Device,
    stop: &AtomicBool,
) -> windows::core::Result<windows::Win32::Graphics::Dxgi::IDXGIOutputDuplication> {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match unsafe { output.DuplicateOutput(device) } {
            Err(error)
                if error.code() == windows::Win32::Foundation::E_ACCESSDENIED
                    && std::time::Instant::now() < until
                    && !stop.load(Ordering::Relaxed) =>
            {
                std::thread::sleep(std::time::Duration::from_millis(250))
            }
            result => return result,
        }
    }
}

/// a frame held from the screen's duplication. the next one can't be acquired until it is given
/// back, so every way out of the loop body releases it, and `release` reports what a drop can't
#[cfg(windows)]
struct Held<F: FnOnce() -> Result<(), String>>(Option<F>);
#[cfg(windows)]
impl<F: FnOnce() -> Result<(), String>> Held<F> {
    fn release(mut self) -> Result<(), String> {
        self.0.take().map_or(Ok(()), |release| release())
    }
}
#[cfg(windows)]
impl<F: FnOnce() -> Result<(), String>> Drop for Held<F> {
    fn drop(&mut self) {
        if let Some(release) = self.0.take() {
            let _ = release();
        }
    }
}

#[cfg(windows)]
fn run(
    config: &Config,
    status: &Arc<Mutex<Status>>,
    sink: &Arc<Mutex<Option<Sink>>>,
    stop: &AtomicBool,
    preparation: &mut Option<Preparation>,
) -> Result<(), String> {
    use windows::{
        Win32::{
            Foundation::HMODULE,
            Graphics::{Direct3D::D3D_DRIVER_TYPE_UNKNOWN, Direct3D11::*, Dxgi::*},
        },
        core::Interface,
    };
    let fail = |e: windows::core::Error| e.to_string();
    let file = Model::read(&config.model)?;
    let model = load_model_options(
        config,
        &file,
        matches!(config.provider.as_str(), "CUDA" | "TensorRT") && config.cuda_graph,
        false,
    )?;
    if stop.load(Ordering::Relaxed) {
        return Ok(());
    }
    if model.inputs().len() != 1 || model.outputs().len() != 1 {
        return Err("target model must have one image input and one YOLOv8 output".into());
    }
    let shape = match model.inputs()[0].dtype() {
        ort::value::ValueType::Tensor { shape, ty, .. }
            if *ty == ort::value::TensorElementType::Float32 =>
        {
            shape.clone()
        }
        _ => return Err("target model must accept float32 NCHW input".into()),
    };
    if shape.len() != 4
        || shape[0] != 1
        || shape[1] != 3
        || shape[2] <= 0
        || shape[2] != shape[3]
        || shape[2] > 1280
    {
        return Err(format!(
            "unsupported model input {shape:?}, need fixed [1,3,S,S]"
        ));
    }
    let size = shape[2] as usize;
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.map_err(fail)?;
    let adapter = unsafe { factory.EnumAdapters1(config.adapter) }.map_err(fail)?;
    let output = unsafe { adapter.EnumOutputs(config.output) }.map_err(fail)?;
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
    .map_err(fail)?;
    let (device, context) = (device.unwrap(), context.unwrap());
    let output: IDXGIOutput1 = output.cast().map_err(fail)?;
    // the display as the projection reads it: unrotated, and its size is the screen the FOV spans
    let usable = |duplication: &IDXGIOutputDuplication| {
        let desc = unsafe { duplication.GetDesc() };
        if desc.Rotation != windows::Win32::Graphics::Dxgi::Common::DXGI_MODE_ROTATION_IDENTITY {
            return Err("rotated displays are unsupported for target projection".to_string());
        }
        Ok([desc.ModeDesc.Width, desc.ModeDesc.Height])
    };
    let mut duplication = duplicate(&output, &device, stop).map_err(fail)?;
    let screen = usable(&duplication)?;
    status.locked().viewport = screen;
    let [width, height] = screen;
    let preprocess =
        crate::targets::preprocess::Preprocess::new(&device, width, height, size).map_err(fail)?;
    let model_shape = format!("{shape:?}");
    let mut engine =
        crate::targets::inference::Engine::new(model, &file, config, &adapter, &preprocess)?;
    preparation.take();
    if stop.load(Ordering::Relaxed) {
        return Ok(());
    }
    let anchor = motion_bench::capture::clock::Anchor::take();
    let qpc = anchor.midpoint();
    let frequency = motion_bench::capture::clock::frequency();
    let epoch = anchor.utc_us;
    let mut clock_monitor = motion_bench::capture::clock::Monitor::default();
    clock_monitor.observe(&anchor, frequency)?;
    {
        let mut s = status.locked();
        s.ready = true;
        s.provider = config.provider.clone();
        s.model_shape = model_shape;
        s.transport = engine.transport.clone();
        s.optimization_warning = engine.warning.clone();
        s.inference = engine.inference.clone();
        s.model_sha256 = Some(file.sha256.clone());
        s.clock = Some(motion_bench::targets::vision::Clock {
            initial_bracket_us: anchor.uncertainty_us(frequency),
            qpc_origin: qpc,
            qpc_frequency: frequency,
            epoch_origin_us: epoch,
        });
    }
    let scale = (size as f64 / width as f64).min(size as f64 / height as f64);
    let resized_w = (width as f64 * scale).round() as usize;
    let resized_h = (height as f64 * scale).round() as usize;
    let left = (size - resized_w) / 2;
    let top = (size - resized_h) / 2;
    let interval = config.frame_interval();
    let mut next = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource = None;
        match unsafe { duplication.AcquireNextFrame(16, &mut info, &mut resource) } {
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => continue,
            // Windows takes the copy away on a desktop switch, a UAC prompt, a mode change or a
            // fullscreen app, and a new one carries on. the display it copies is checked again
            Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                drop(duplication);
                let again = duplicate(&output, &device, stop);
                if stop.load(Ordering::Relaxed) {
                    return Ok(());
                }
                duplication = again.map_err(|e| format!("DXGI capture stopped ({e}), so restart it after changing displays or fullscreen mode"))?;
                // the targets already found were placed on the old size
                let now = usable(&duplication)?;
                if now != screen {
                    return Err(format!(
                        "the display changed from {}×{} to {}×{} while target capture ran, so restart it",
                        screen[0], screen[1], now[0], now[1]
                    ));
                }
                continue;
            }
            Err(e) => {
                return Err(format!(
                    "DXGI capture stopped ({e}), so restart it after changing displays or fullscreen mode"
                ));
            }
            Ok(()) => {}
        }
        let held = Held(Some(|| unsafe { duplication.ReleaseFrame() }.map_err(fail)));
        if info.LastPresentTime == 0 {
            held.release()?;
            continue;
        }
        {
            let mut s = status.locked();
            s.capture_rate.push_frame(Instant::now());
            s.skipped += info.AccumulatedFrames.saturating_sub(1) as u64;
        }
        if interval.is_some() && Instant::now() < next {
            status.locked().throttled += 1;
            held.release()?;
            continue;
        }
        if let Some(interval) = interval {
            next = Instant::now() + interval;
        }
        let source: ID3D11Texture2D = resource.unwrap().cast().map_err(fail)?;
        let pipeline = Instant::now();
        let receipt = motion_bench::capture::clock::Anchor::take();
        let received_us = receipt.utc_us;
        let now_qpc = receipt.midpoint();
        clock_monitor.observe(&receipt, frequency)?;
        preprocess.copy(&context, &source);
        held.release()?;
        engine.prepare(&preprocess, &context)?;
        let preprocess_ms = pipeline.elapsed().as_secs_f64() * 1000.0;
        let began = Instant::now();
        let (channels, candidates) = (engine.channels, engine.candidates);
        let values = engine.infer()?;
        let inference_ms = began.elapsed().as_secs_f64() * 1000.0;
        let mut detections =
            motion_bench::targets::vision::yolo8(values, channels, candidates, config)?;
        for d in &mut detections {
            d.bounds = [
                (d.bounds[0] - left as f64) / scale,
                (d.bounds[1] - top as f64) / scale,
                (d.bounds[2] - left as f64) / scale,
                (d.bounds[3] - top as f64) / scale,
            ];
        }
        detections.retain(|d| {
            d.bounds[0] >= 0.0
                && d.bounds[1] >= 0.0
                && d.bounds[2] <= width as f64
                && d.bounds[3] <= height as f64
        });
        let frame = Frame {
            clock_anchor: receipt.clone(),
            source_qpc: info.LastPresentTime,
            received_qpc: now_qpc,
            completed_qpc: motion_bench::capture::clock::qpc(),
            source_us: receipt
                .epoch_us(info.LastPresentTime, frequency)
                .ok_or("screen presentation time exceeded the clock mapping")?,
            received_us,
            completed_us: motion_bench::capture::session::wall_us(),
            preprocess_ms,
            pipeline_ms: pipeline.elapsed().as_secs_f64() * 1000.0,
            inference_ms,
            skipped: info.AccumulatedFrames.saturating_sub(1),
            detections,
        };
        if status.locked().last.as_ref().is_some_and(|previous| {
            previous.source_us >= frame.source_us || previous.source_qpc >= frame.source_qpc
        }) {
            return Err("screen presentation clock regressed, raw evidence is retained".into());
        }
        {
            let mut sink = sink.locked();
            if let Some(sink) = sink.as_mut()
                && frame.source_us >= sink.started
            {
                serde_json::to_writer(&mut sink.writer, &frame).map_err(|e| e.to_string())?;
                sink.writer.write_all(b"\n").map_err(|e| e.to_string())?;
            }
        }
        let mut s = status.locked();
        s.frames += 1;
        let now = Instant::now();
        s.analyzed_rate.push_frame(now);
        s.pipeline_ms.push(now, frame.pipeline_ms);
        s.last = Some(frame);
        if let Some(wakeup) = &s.experiment_wakeup {
            let _ = wakeup.try_send(());
        }
    }
    Ok(())
}
#[cfg(not(windows))]
fn run(
    _: &Config,
    _: &Arc<Mutex<Status>>,
    _: &Arc<Mutex<Option<Sink>>>,
    _: &AtomicBool,
    _: &mut Option<Preparation>,
) -> Result<(), String> {
    Err("DXGI capture requires Windows".into())
}

#[cfg(all(test, windows))]
mod tests;
