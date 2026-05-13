//! Producer trait and implementations for event sources.

#[cfg(target_arch = "wasm32")]
use crate::recording::RecordingState;
#[cfg(target_arch = "wasm32")]
use image::RgbImage;
#[cfg(target_arch = "wasm32")]
use libreeb::{io::header::RawFileHeader, EventBatch, SliceBy};
#[cfg(target_arch = "wasm32")]
use std::sync::{Arc, Mutex};

#[cfg(target_arch = "wasm32")]
pub use camera::RoiCommand;

#[cfg(target_arch = "wasm32")]
pub trait Producer: Send + 'static {
    /// Returns (batch, frame, event_count, event_rate_ev_per_s, last_timestamp_us) for the latest slice.
    fn poll_frame(&mut self) -> Option<(EventBatch, RgbImage, usize, f64, u64)>;

    /// Set slice window.
    fn set_slice(&mut self, slice: SliceBy);

    /// Apply bias settings (camera mode only).
    fn apply_biases(&mut self, biases: Vec<(usize, u8, i32)>);

    /// Set or clear hardware ROI (`None` = clear).  No-op for file mode.
    fn set_roi(&mut self, roi: Option<[u32; 4]>) {
        let _ = roi; // default no-op
    }

    /// Set playback speed multiplier (file mode only).
    fn set_playback_speed(&mut self, speed: f32);

    /// Stop the producer.
    fn stop(&mut self);

    /// Get reference to header (for recording).
    fn get_header(&self) -> &RawFileHeader;

    /// Start recording to the given RecordingState.
    fn start_recording(&mut self, recorder: Arc<Mutex<RecordingState>>);

    /// Stop recording.
    fn stop_recording(&mut self);
}

#[cfg(target_arch = "wasm32")]
pub(crate) mod file;

#[cfg(target_arch = "wasm32")]
pub(crate) mod camera;

#[cfg(target_arch = "wasm32")]
pub use file::WasmFileProducer;

#[cfg(target_arch = "wasm32")]
pub use camera::WasmCameraProducer;

/// Calculate event rate from a batch.
#[cfg(target_arch = "wasm32")]
pub(crate) fn calculate_rate(batch: &libreeb::EventBatch) -> f64 {
    let n = batch.len();
    if n >= 2 {
        let dt_us = batch.t.last().unwrap().saturating_sub(batch.t[0]);
        if dt_us > 0 {
            n as f64 / (dt_us as f64 * 1e-6)
        } else {
            0.0
        }
    } else {
        0.0
    }
}
