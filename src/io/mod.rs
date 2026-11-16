//! I/O operations: file reading, camera interfaces, and stream handling.
//!
//! This module provides:
//! - [`RawFileReader`]: Read `.raw` event files from disk or memory
//! - [`EventDecoder`]: Iterator-based event decoder with slicing support
//! - [`header`]: RAW file header parsing (% lines)
//! - [`biases`]: Sensor hardware configuration and bias presets
//! - [`camera`]: USB camera interface (native, with `camera` feature)
//! - [`camera_web`]: WebUSB camera interface (WASM only)
//! - [`tee`]: Stream multiplexing/splitting utilities

pub mod biases;
pub mod header;
pub mod reader;
pub mod tee;

#[cfg(all(not(target_arch = "wasm32"), feature = "camera"))]
pub mod camera;

#[cfg(target_arch = "wasm32")]
pub mod camera_web;

pub use header::{CameraGeometry, RawEventType, RawFileHeader};
pub use reader::EventDecoder;

#[cfg(all(not(target_arch = "wasm32"), feature = "camera"))]
pub use camera::CameraReader;

#[cfg(target_arch = "wasm32")]
pub use camera_web::WasmCameraReader;
