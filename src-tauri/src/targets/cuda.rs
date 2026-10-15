use libloading::Library;
use std::{
    ffi::{CStr, c_char, c_void},
    path::PathBuf,
    ptr::NonNull,
    sync::Arc,
};
use windows::{
    Win32::Graphics::{Direct3D11::ID3D11Buffer, Dxgi::IDXGIAdapter1},
    core::Interface,
};

type SetDevice = unsafe extern "system" fn(i32) -> i32;
type AdapterDevice = unsafe extern "system" fn(*mut i32, *mut c_void) -> i32;
type Register = unsafe extern "system" fn(*mut *mut c_void, *mut c_void, u32) -> i32;
type Unregister = unsafe extern "system" fn(*mut c_void) -> i32;
type Map = unsafe extern "system" fn(i32, *mut *mut c_void, *mut c_void) -> i32;
type Pointer = unsafe extern "system" fn(*mut *mut c_void, *mut usize, *mut c_void) -> i32;
type Copy = unsafe extern "system" fn(*mut c_void, *const c_void, usize, i32) -> i32;
type Synchronize = unsafe extern "system" fn(*mut c_void) -> i32;
type ErrorString = unsafe extern "system" fn(i32) -> *const c_char;

/// the CUDA 13.3 and cuDNN 9.22 binaries, where their installers say they put them or in their
/// default folders
pub fn toolkit() -> Result<[PathBuf; 2], String> {
    let installed = |variable: &str, default: &str| match std::env::var_os(variable) {
        Some(path) => Ok(PathBuf::from(path)),
        None => crate::storage::folders::program_files().map(|folder| folder.join(default)),
    };
    Ok([
        installed("CUDA_PATH_V13_3", "NVIDIA GPU Computing Toolkit/CUDA/v13.3")?.join("bin/x64"),
        installed("CUDNN_PATH", "NVIDIA/CUDNN/v9.22")?.join("bin/x64"),
    ])
}

pub fn device_identity(index: u32) -> Result<serde_json::Value, String> {
    unsafe {
        let driver = Library::new("nvcuda.dll").map_err(|e| e.to_string())?;
        let init = driver
            .get::<unsafe extern "system" fn(u32) -> i32>(b"cuInit\0")
            .map_err(|e| e.to_string())?;
        let get = driver
            .get::<unsafe extern "system" fn(*mut i32, i32) -> i32>(b"cuDeviceGet\0")
            .map_err(|e| e.to_string())?;
        let uuid = driver
            .get::<unsafe extern "system" fn(*mut [u8; 16], i32) -> i32>(b"cuDeviceGetUuid_v2\0")
            .map_err(|e| e.to_string())?;
        let version = driver
            .get::<unsafe extern "system" fn(*mut i32) -> i32>(b"cuDriverGetVersion\0")
            .map_err(|e| e.to_string())?;
        let mut device = 0;
        let mut bytes = [0u8; 16];
        let mut driver_version = 0;
        if init(0) != 0
            || get(&mut device, index as i32) != 0
            || uuid(&mut bytes, device) != 0
            || version(&mut driver_version) != 0
        {
            return Err(format!(
                "cannot identify CUDA device {index} for TensorRT cache"
            ));
        }
        Ok(
            serde_json::json!({"uuid": bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(), "driver_version": driver_version}),
        )
    }
}

pub struct Cuda {
    _library: Library,
    register: Register,
    unregister: Unregister,
    map: Map,
    unmap: Map,
    pointer: Pointer,
    copy: Copy,
    synchronize: Synchronize,
    error_string: ErrorString,
}

impl Cuda {
    pub fn new(device: u32, adapter: &IDXGIAdapter1) -> Result<Arc<Self>, String> {
        unsafe {
            let [cuda, _] = toolkit()?;
            let library = Library::new(cuda.join("cudart64_13.dll")).map_err(|e| e.to_string())?;
            let set_device = *library
                .get::<SetDevice>(b"cudaSetDevice\0")
                .map_err(|e| e.to_string())?;
            let adapter_device = *library
                .get::<AdapterDevice>(b"cudaD3D11GetDevice\0")
                .map_err(|e| e.to_string())?;
            let api = Self {
                register: *library
                    .get(b"cudaGraphicsD3D11RegisterResource\0")
                    .map_err(|e| e.to_string())?,
                unregister: *library
                    .get(b"cudaGraphicsUnregisterResource\0")
                    .map_err(|e| e.to_string())?,
                map: *library
                    .get(b"cudaGraphicsMapResources\0")
                    .map_err(|e| e.to_string())?,
                unmap: *library
                    .get(b"cudaGraphicsUnmapResources\0")
                    .map_err(|e| e.to_string())?,
                pointer: *library
                    .get(b"cudaGraphicsResourceGetMappedPointer\0")
                    .map_err(|e| e.to_string())?,
                copy: *library.get(b"cudaMemcpy\0").map_err(|e| e.to_string())?,
                synchronize: *library
                    .get(b"cudaStreamSynchronize\0")
                    .map_err(|e| e.to_string())?,
                error_string: *library
                    .get(b"cudaGetErrorString\0")
                    .map_err(|e| e.to_string())?,
                _library: library,
            };
            let mut graphics_device = -1;
            api.check(
                adapter_device(&mut graphics_device, adapter.as_raw()),
                "select graphics adapter",
            )?;
            if graphics_device != device as i32 {
                return Err(format!(
                    "DXGI adapter uses CUDA device {graphics_device}, but inference selects {device}"
                ));
            }
            api.check(set_device(device as i32), "select CUDA device")?;
            Ok(Arc::new(api))
        }
    }

    fn check(&self, code: i32, operation: &str) -> Result<(), String> {
        if code == 0 {
            return Ok(());
        }
        let message = unsafe { CStr::from_ptr((self.error_string)(code)) }.to_string_lossy();
        Err(format!("CUDA {operation}: {message} ({code})"))
    }

    pub fn register(self: &Arc<Self>, buffer: &ID3D11Buffer) -> Result<GraphicsBuffer, String> {
        let mut resource = std::ptr::null_mut();
        self.check(
            unsafe { (self.register)(&mut resource, buffer.as_raw(), 0) },
            "register D3D11 buffer",
        )?;
        Ok(GraphicsBuffer {
            resource: NonNull::new(resource).ok_or("CUDA returned a null graphics resource")?,
            api: self.clone(),
            _buffer: buffer.clone(),
        })
    }

    pub unsafe fn read(
        &self,
        source: *const c_void,
        destination: &mut [f32],
    ) -> Result<(), String> {
        self.check(
            unsafe {
                (self.copy)(
                    destination.as_mut_ptr().cast(),
                    source,
                    std::mem::size_of_val(destination),
                    2,
                )
            },
            "read predictions",
        )
    }

    pub unsafe fn upload(&self, source: &[f32], destination: *mut c_void) -> Result<(), String> {
        self.check(
            unsafe {
                (self.copy)(
                    destination,
                    source.as_ptr().cast(),
                    std::mem::size_of_val(source),
                    1,
                )
            },
            "upload tensor",
        )?;
        self.check(
            unsafe { (self.synchronize)(std::ptr::null_mut()) },
            "finish upload",
        )
    }
}

pub struct GraphicsBuffer {
    resource: NonNull<c_void>,
    api: Arc<Cuda>,
    _buffer: ID3D11Buffer,
}

impl GraphicsBuffer {
    pub unsafe fn copy_to(&self, destination: *mut c_void, bytes: usize) -> Result<(), String> {
        let mut resource = self.resource.as_ptr();
        self.api.check(
            unsafe { (self.api.map)(1, &mut resource, std::ptr::null_mut()) },
            "map D3D11 buffer",
        )?;
        let result = (|| {
            let mut source = std::ptr::null_mut();
            let mut available = 0;
            self.api.check(
                unsafe { (self.api.pointer)(&mut source, &mut available, resource) },
                "get D3D11 tensor",
            )?;
            if available < bytes {
                return Err("CUDA graphics buffer is smaller than the model input".into());
            }
            // mapped addresses may change. copy into the stable address bound to the CUDA graph
            self.api.check(
                unsafe { (self.api.copy)(destination, source, bytes, 3) },
                "copy device tensor",
            )
        })();
        let unmap = self.api.check(
            unsafe { (self.api.unmap)(1, &mut resource, std::ptr::null_mut()) },
            "unmap D3D11 buffer",
        );
        let sync = self.api.check(
            unsafe { (self.api.synchronize)(std::ptr::null_mut()) },
            "finish tensor transfer",
        );
        result.and(unmap).and(sync)
    }
}

impl Drop for GraphicsBuffer {
    fn drop(&mut self) {
        let _ = unsafe { (self.api.unregister)(self.resource.as_ptr()) };
    }
}
