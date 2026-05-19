//! Producer trait and implementations for event sources.

#[cfg(target_arch = "wasm32")]
use crate::recording::RecordingState;
#[cfg(target_arch = "wasm32")]
use libreeb::io::header::RawFileHeader;
#[cfg(target_arch = "wasm32")]
use std::sync::{Arc, Mutex};

/// Push-based event source.
///
/// Producers spawn their own internal async loop (via `spawn_local`) that
/// pushes [`EventBatch`] items into the shared `TimeWindow`.  The UI reads
/// from that window independently — there is no `poll_frame` call.
///
/// The methods below are the only controls the UI needs to reach into the
/// producer after construction.
#[cfg(target_arch = "wasm32")]
pub trait Producer: Send + 'static {
    /// Apply bias settings (camera mode only).
    fn apply_biases(&mut self, biases: Vec<(usize, u8, i32)>);

    /// Set or clear hardware ROI (`None` = clear).  No-op for file mode.
    fn set_roi(&mut self, roi: Option<[u32; 4]>) {
        let _ = roi; // default no-op
    }

    /// Set playback speed multiplier (file mode only).
    fn set_playback_speed(&mut self, speed: f32);

    /// Stop the producer's async loop.
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
