//! Camera-based event producer using WebUSB.

use super::{calculate_rate, Producer};
use crate::recording::RecordingState;
use image::RgbImage;
use libreeb::{io::header::RawFileHeader, SliceBy};
use std::sync::{Arc, Mutex};

pub struct WasmShared {
    pub latest_frame: Option<(RgbImage, usize, f64, u64)>,
    pub pending_slice: Option<SliceBy>,
    pub pending_biases: Option<Vec<(usize, u8, i32)>>,
    pub stop: bool,
    pub recorder: Option<Arc<Mutex<RecordingState>>>,
}

pub struct WasmCameraProducer {
    pub shared: Arc<Mutex<WasmShared>>,
    pub header: RawFileHeader,
}

impl Producer for WasmCameraProducer {
    fn poll_frame(&mut self) -> Option<(RgbImage, usize, f64, u64)> {
        self.shared.lock().ok()?.latest_frame.take()
    }

    fn set_slice(&mut self, s: SliceBy) {
        if let Ok(mut g) = self.shared.lock() {
            g.pending_slice = Some(s);
        }
    }

    fn apply_biases(&mut self, b: Vec<(usize, u8, i32)>) {
        if let Ok(mut g) = self.shared.lock() {
            g.pending_biases = Some(b);
        }
    }

    fn set_playback_speed(&mut self, _speed: f32) {
        // No-op for camera mode (fixed 30 FPS)
    }

    fn stop(&mut self) {
        if let Ok(mut g) = self.shared.lock() {
            g.stop = true;
        }
    }
    
    fn get_header(&self) -> &RawFileHeader {
        &self.header
    }
    
    fn start_recording(&mut self, recorder: Arc<Mutex<RecordingState>>) {
        if let Ok(mut g) = self.shared.lock() {
            g.recorder = Some(recorder);
        }
    }
    
    fn stop_recording(&mut self) {
        if let Ok(mut g) = self.shared.lock() {
            g.recorder = None;
        }
    }
}
