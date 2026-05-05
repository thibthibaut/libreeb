//! Application state types.

use crate::producers::Producer;
use eframe::egui;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use web_time::Instant;

/// Main application state machine.
pub enum WasmState {
    Selection,
    Opening {
        label: String,
        init: InitSlot,
    },
    Viewing(ViewerState),
    Error(String),
}

/// Async init result slot.
pub type InitSlot = Arc<Mutex<Option<Result<ViewerState, String>>>>;

/// Active viewer state when playing back events.
pub struct ViewerState {
    pub width: u32,
    pub height: u32,
    pub label: String,
    pub header_info: Vec<(String, String)>,
    pub producer: Box<dyn Producer>,
    pub texture: Option<egui::TextureHandle>,
    pub frame_count: u64,
    pub window_us: f32,
    pub last_ev_count: usize,
    pub last_rate: f64,

    // Camera-mode bias controls (empty for file mode)
    pub is_camera: bool,
    pub bias_defaults: Vec<u8>,
    pub bias_offsets: Vec<i32>,

    // Playback state
    pub playback_state: PlaybackState,
    pub fps_tracker: FpsTracker,

    // Async operations
    pub pending_bias_load: Option<Arc<Mutex<Option<Vec<i32>>>>>,
}

/// Playback speed and FPS tracking.
pub struct PlaybackState {
    pub speed_multiplier: f32,
    pub target_fps: f32,
    pub actual_fps: f32,
}

impl PlaybackState {
    pub fn new_file() -> Self {
        PlaybackState {
            speed_multiplier: 1.0,
            target_fps: 30.0,
            actual_fps: 0.0,
        }
    }

    pub fn new_camera() -> Self {
        PlaybackState {
            speed_multiplier: 1.0,
            target_fps: 30.0,
            actual_fps: 0.0,
        }
    }
}

/// FPS measurement over a rolling window.
pub struct FpsTracker {
    frame_times: VecDeque<Instant>,
    max_samples: usize,
}

impl FpsTracker {
    pub fn new() -> Self {
        FpsTracker {
            frame_times: VecDeque::with_capacity(60),
            max_samples: 60,
        }
    }

    pub fn record_frame(&mut self) {
        let now = Instant::now();
        self.frame_times.push_back(now);
        while self.frame_times.len() > self.max_samples {
            self.frame_times.pop_front();
        }
    }

    pub fn current_fps(&self) -> f32 {
        if self.frame_times.len() < 2 {
            return 0.0;
        }

        let elapsed = self
            .frame_times
            .back()
            .unwrap()
            .duration_since(*self.frame_times.front().unwrap());

        if elapsed.as_secs_f32() == 0.0 {
            return 0.0;
        }

        (self.frame_times.len() - 1) as f32 / elapsed.as_secs_f32()
    }
}
