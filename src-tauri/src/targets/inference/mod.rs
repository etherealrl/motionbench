use crate::targets::{
    cuda::{Cuda, GraphicsBuffer},
    preprocess::Preprocess,
    vision::{Model, load_model_options},
};
use motion_bench::targets::vision::Config;
use ort::{
    memory::{AllocationDevice, Allocator, AllocatorType, MemoryInfo, MemoryType},
    session::{IoBinding, Session},
    value::{Tensor, TensorRef, ValueType},
};
use std::sync::Arc;
use windows::Win32::Graphics::{Direct3D11::ID3D11DeviceContext, Dxgi::IDXGIAdapter1};

pub struct Engine {
    model: Session,
    gpu: Option<Gpu>,
    input: Vec<f32>,
    output: Vec<f32>,
    pub size: usize,
    pub channels: usize,
    pub candidates: usize,
    pub transport: String,
    pub warning: Option<String>,
    pub inference: Option<serde_json::Value>,
}

struct Gpu {
    binding: IoBinding,
    input: Tensor<f32>,
    output: *const std::ffi::c_void,
    graphics: GraphicsBuffer,
    api: Arc<Cuda>,
    // ORT tensors retain allocator callbacks; the allocator must outlive the bound tensors
    _allocator: Allocator,
}

impl Engine {
    pub fn new(
        mut model: Session,
        file: &Model,
        config: &Config,
        adapter: &IDXGIAdapter1,
        preprocess: &Preprocess,
    ) -> Result<Self, String> {
        let shape = match model.inputs()[0].dtype() {
            ValueType::Tensor { shape, .. } => shape,
            _ => return Err("expected an image tensor".into()),
        };
        let size = shape[2] as usize;
        let shape = match model.outputs()[0].dtype() {
            ValueType::Tensor { shape, ty, .. }
                if *ty == ort::value::TensorElementType::Float32 =>
            {
                shape
            }
            _ => return Err("target model must produce float32 YOLO predictions".into()),
        };
        if shape.len() != 3
            || shape[0] != 1
            || !(5..=1024).contains(&shape[1])
            || !(1..=4_000_000).contains(&shape[2])
        {
            return Err(format!(
                "target model needs fixed [1,classes+4,candidates] output, got {shape:?}"
            ));
        }
        let (channels, candidates) = (shape[1] as usize, shape[2] as usize);
        let input = vec![0f32; 3 * size * size];
        let mut warning = None;
        let gpu = if config.provider == "TensorRT"
            || (config.provider == "CUDA" && config.cuda_graph)
        {
            match Gpu::new(
                &mut model,
                config,
                adapter,
                preprocess,
                [size, channels, candidates],
                &input,
            ) {
                Ok(gpu) => Some(gpu),
                Err(error) => {
                    if config.provider == "TensorRT" {
                        return Err(format!(
                            "TensorRT GPU I/O binding failed: {error}; choose CUDA to use its fallback path"
                        ));
                    }
                    warning = Some(format!(
                        "GPU graph unavailable; using standard CUDA: {error}"
                    ));
                    model = load_model_options(config, file, false, false)?;
                    None
                }
            }
        } else {
            None
        };
        let transport = if config.provider == "TensorRT" {
            format!(
                "D3D11 to CUDA device tensor, TensorRT {}{}, CUDA operator fallback",
                if config.tensorrt_fp16 { "FP16" } else { "FP32" },
                if config.cuda_graph {
                    ", CUDA graph"
                } else {
                    ""
                }
            )
        } else if gpu.is_some() {
            "D3D11 to CUDA device tensor, CUDA graph".into()
        } else {
            "D3D11 to CPU tensor to inference".into()
        };
        let mut engine = Self {
            model,
            gpu,
            input,
            output: vec![0f32; channels * candidates],
            size,
            channels,
            candidates,
            transport,
            warning,
            inference: None,
        };
        engine.infer()?;
        if config.provider == "TensorRT" {
            engine.inference = Some(crate::targets::vision::finish_tensorrt_profile(
                &mut engine.model,
            )?);
        }
        Ok(engine)
    }

    pub fn prepare(
        &mut self,
        preprocess: &Preprocess,
        context: &ID3D11DeviceContext,
    ) -> Result<(), String> {
        if let Some(gpu) = self.gpu.as_mut() {
            preprocess.dispatch(context);
            unsafe {
                gpu.graphics
                    .copy_to(gpu.input.data_ptr_mut(), self.input.len() * 4)
            }
        } else {
            preprocess
                .tensor(context, &mut self.input)
                .map_err(|e| e.to_string())
        }
    }

    pub fn infer(&mut self) -> Result<&[f32], String> {
        if let Some(gpu) = &self.gpu {
            self.model
                .run_binding(&gpu.binding)
                .map_err(|e| e.to_string())?;
            gpu.binding
                .synchronize_outputs()
                .map_err(|e| e.to_string())?;
            unsafe {
                gpu.api.read(gpu.output, &mut self.output)?;
            }
        } else {
            let tensor =
                TensorRef::from_array_view(([1usize, 3, self.size, self.size], &self.input[..]))
                    .map_err(|e| e.to_string())?;
            let outputs = self
                .model
                .run(ort::inputs![tensor])
                .map_err(|e| e.to_string())?;
            let (shape, values) = outputs[0]
                .try_extract_tensor::<f32>()
                .map_err(|e| e.to_string())?;
            if shape.as_ref() != [1, self.channels as i64, self.candidates as i64] {
                return Err(format!("model output changed shape to {shape:?}"));
            }
            self.output.copy_from_slice(values);
        }
        Ok(&self.output)
    }
}

impl Gpu {
    fn new(
        model: &mut Session,
        config: &Config,
        adapter: &IDXGIAdapter1,
        preprocess: &Preprocess,
        dimensions: [usize; 3],
        zeros: &[f32],
    ) -> Result<Self, String> {
        let [size, channels, candidates] = dimensions;
        let api = Cuda::new(config.cuda_device, adapter)?;
        let allocator = Allocator::new(
            model,
            MemoryInfo::new(
                AllocationDevice::CUDA,
                config.cuda_device as i32,
                AllocatorType::Device,
                MemoryType::Default,
            )
            .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let mut input =
            Tensor::<f32>::new(&allocator, [1usize, 3, size, size]).map_err(|e| e.to_string())?;
        unsafe {
            api.upload(zeros, input.data_ptr_mut())?;
        }
        let output = Tensor::<f32>::new(&allocator, [1usize, channels, candidates])
            .map_err(|e| e.to_string())?;
        let pointer = output.data_ptr();
        let mut binding = model.create_binding().map_err(|e| e.to_string())?;
        binding
            .bind_input(model.inputs()[0].name(), &input)
            .map_err(|e| e.to_string())?;
        binding
            .bind_output(model.outputs()[0].name(), output)
            .map_err(|e| e.to_string())?;
        let graphics = api.register(preprocess.output())?;
        model.run_binding(&binding).map_err(|e| e.to_string())?;
        Ok(Self {
            api,
            graphics,
            input,
            output: pointer,
            binding,
            _allocator: allocator,
        })
    }
}

#[cfg(test)]
mod tests;
