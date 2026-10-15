//! target capture: the screen worker, the model's inference and its GPU input, and the CUDA
//! toolkit they load from
#[cfg(windows)]
pub mod cuda;
#[cfg(windows)]
pub mod inference;
#[cfg(windows)]
pub mod preprocess;
pub mod vision;
