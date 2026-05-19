//! Application-level state.
//!
//! Keeps the flat `ViewerState` as simple as possible — one struct with
//! plain fields, no nested sub-states.  Enums are kept to a minimum.

use crate::producers::Producer;
use crate::recording::RecordingState;
use crate::utils::{RateSample, TimeWindow};
use eframe::egui;
use libreeb::{processing::{FrameGenerator, TimeSurface}, EventBatch};
use std::sync::{Arc, Mutex};
use web_time::Instant;

// ── Display mode ──────────────────────────────────────────────────────────────

/// Which visualisation occupies the central canvas.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DisplayMode {
    /// Colour-coded event-density frame (default).
    Normal, // TODO: Rename to RGB frame
    /// Per-pixel time-surface with jet colourmap + exponential decay.
    TimeSurface,
    /// Interactive 3-D XYT point cloud (drag to rotate, scroll to zoom).
    Xyt3D,
}

// ── Top-level WASM state machine ──────────────────────────────────────────────

pub enum AppState {
    /// Waiting for user to pick a source.
    Selection,
    /// Async init in progress (file parsing or USB camera open).
    Opening { label: String, init: InitSlot },
    /// Source open and streaming.
    Viewing(ViewerState),
    /// Unrecoverable error; shows message + Back button.
    Error(String),
}

pub type InitSlot = Arc<Mutex<Option<Result<ViewerState, String>>>>;

// ── ViewerState ───────────────────────────────────────────────────────────────

pub struct ViewerState {
    // ── Source metadata ───────────────────────────────────────────────────────
    pub width: u32,
    pub height: u32,
    pub label: String,
    pub header_info: Vec<(String, String)>, // TODO: couldn't we use a struct from the header.rs here ? and maybe a to_vec_of_string method ? I think the logic of converting the Header Strcut to a Vec of string (or hashmap) should belong to the header
    pub is_camera: bool,
    /// Hardware sensor ID used as localStorage key (camera mode only).
    pub sensor_id: Option<u32>,

    // ── Data source ───────────────────────────────────────────────────────────
    pub producer: Box<dyn Producer>,

    // ── Shared event buffer ───────────────────────────────────────────────────
    /// Sensor-time sliding window shared with the producer's async loop.
    /// The producer pushes batches; all rendering reads from this.
    /// Caps: 2 s time window, 30 000 batch count.
    pub event_buffer: Arc<Mutex<TimeWindow<EventBatch>>>,

    // ── Timing / stats ────────────────────────────────────────────────────────
    pub first_timestamp: Option<u64>, // TODO: document what is this
    pub last_timestamp: Option<u64>,  // TODO: document what is this
    pub frame_count: u64,
    pub fps_tracker: FpsTracker,
    pub render_fps: f32, // UI render FPS

    /// Highest sensor timestamp already accounted for in stats and TimeSurface.
    pub last_processed_ts: u64,
    /// Total batches dropped by the event_buffer count cap (informational).
    pub dropped_count: u64,

    // ── Playback (file mode) ──────────────────────────────────────────────────
    pub window_us: f32,
    pub playback_speed: f32,
    pub playback_fps: f32,

    // ── Canvas display ────────────────────────────────────────────────────────
    pub display_mode: DisplayMode,

    // Normal frame texture — regenerated each render from `window_us` slice of event_buffer.
    pub frame_texture: Option<egui::TextureHandle>,
    /// Frame generator used for Normal mode (holds camera geometry).
    pub frame_generator: FrameGenerator,

    // Time surface: accumulator + rendered texture.
    // `ts_texture` is updated inside `process()` only when mode == TimeSurface.
    pub time_surface: TimeSurface,
    pub ts_texture: Option<egui::TextureHandle>,
    pub ts_decay_us: u64,

    // ── XYT 3-D point cloud ───────────────────────────────────────────────────
    /// XYT renderer reads directly from `event_buffer` (no separate buffer).
    pub xyt_azimuth: f32,
    pub xyt_elevation: f32,
    pub xyt_point_size: f32,
    pub xyt_zoom: f32,

    // ── Event-rate history (bottom panel) ─────────────────────────────────────
    /// Sliding window of rate samples — 5-min sensor-time retention.
    /// `rate_lookback_s` (below) controls the visible display window only;
    /// it never truncates stored samples.
    pub rate_history: TimeWindow<RateSample>,
    /// Display-only lookback for the event-rate plot (seconds).
    pub rate_lookback_s: f64,

    // ── Hardware ROI (camera only) ────────────────────────────────────────────
    pub roi_enabled: bool, // TODO: Encapsulate this state in a struct! Why not a sensor_roi.rs next to baises.s ?
    pub roi_left: u32,
    pub roi_top: u32,
    pub roi_width: u32,
    pub roi_height: u32,

    // ── Biases (camera only) ──────────────────────────────────────────────────
    pub bias_defaults: Vec<u8>, // TODO: Why is this not handled in biases.rs ? by the bias struct ?
    pub bias_offsets: Vec<i32>,
    /// Slot for async bias-file load (rfd picks file on a background task).
    pub pending_bias_load: Option<Arc<Mutex<Option<Vec<i32>>>>>,

    // ── Recording ─────────────────────────────────────────────────────────────
    pub recorder: Arc<Mutex<RecordingState>>,
}

// ── FPS tracker ───────────────────────────────────────────────────────────────

pub struct FpsTracker {
    times: std::collections::VecDeque<Instant>,
    max_samples: usize,
}

impl FpsTracker {
    pub fn new() -> Self {
        FpsTracker {
            times: std::collections::VecDeque::with_capacity(60),
            max_samples: 60,
        }
    }

    pub fn record(&mut self) {
        self.times.push_back(Instant::now());
        while self.times.len() > self.max_samples {
            self.times.pop_front();
        }
    }

    pub fn fps(&self) -> f32 {
        if self.times.len() < 2 {
            return 0.0;
        }
        let elapsed = self
            .times
            .back()
            .unwrap()
            .duration_since(*self.times.front().unwrap())
            .as_secs_f32();
        if elapsed == 0.0 {
            0.0
        } else {
            (self.times.len() - 1) as f32 / elapsed
        }
    }
}
