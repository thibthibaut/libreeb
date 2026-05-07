//! File-based event producer with timestamp-based playback timing.

use super::{calculate_rate, Producer};
use crate::recording::RecordingState;
use image::RgbImage;
use libreeb::{
    header::{CameraGeometry, RawFileHeader},
    processors::FrameGenerator, EventBatch, EventDecoder, RawFileReader, SliceBy,
};
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use web_time::Instant;

pub struct WasmFileProducer {
    bytes: Arc<[u8]>,
    header: RawFileHeader,
    generator: FrameGenerator,
    decoder: EventDecoder<EventBatch>,
    slice: SliceBy,

    // Timing state for real-time playback
    playback_speed: f32,
    playback_start: Option<Instant>,
    first_event_ts: Option<u64>,
    buffered_batch: Option<EventBatch>,

    // Recording state
    recorder: Option<Arc<Mutex<RecordingState>>>,
}

impl WasmFileProducer {
    pub fn new(
        bytes: Arc<[u8]>,
        geometry: CameraGeometry,
        slice: SliceBy,
        header: RawFileHeader,
    ) -> Self {
        let decoder = Self::make_decoder(Arc::clone(&bytes), slice);
        WasmFileProducer {
            bytes,
            header,
            generator: FrameGenerator::new(geometry),
            decoder,
            slice,
            playback_speed: 1.0,
            playback_start: None,
            first_event_ts: None,
            buffered_batch: None,
            recorder: None,
        }
    }

    fn make_decoder(bytes: Arc<[u8]>, slice: SliceBy) -> EventDecoder<EventBatch> {
        match RawFileReader::from_reader(Cursor::new(bytes)) {
            Ok(r) => {
                let mut dec = r.into_decoder();
                dec.set_slice_by(Some(slice));
                dec
            }
            Err(e) => {
                web_sys::console::error_1(
                    &format!("WasmFileProducer decoder error: {e}").into(),
                );
                panic!("Cannot create decoder: {e}");
            }
        }
    }

    fn should_display_batch(&self, batch: &EventBatch) -> bool {
        let Some(playback_start) = self.playback_start else {
            return true;
        };
        let Some(first_ts) = self.first_event_ts else {
            return true;
        };
        let Some(&batch_ts) = batch.t.first() else {
            return true;
        };

        let elapsed_event_us = batch_ts - first_ts;
        let target_real_us = (elapsed_event_us as f64 / self.playback_speed as f64) as u64;
        let target_time = playback_start + std::time::Duration::from_micros(target_real_us);

        Instant::now() >= target_time
    }

    fn reset_playback(&mut self) {
        self.decoder = Self::make_decoder(Arc::clone(&self.bytes), self.slice);
        self.playback_start = None;
        self.first_event_ts = None;
        self.buffered_batch = None;
    }

    fn render_batch(&self, batch: EventBatch) -> (RgbImage, usize, f64, u64) {
        let n = batch.len();
        let rate = calculate_rate(&batch);
        let last_ts = batch.t.last().copied().unwrap_or(0);
        (self.generator.generate(&batch), n, rate, last_ts)
    }
}

impl Producer for WasmFileProducer {
    fn poll_frame(&mut self) -> Option<(RgbImage, usize, f64, u64)> {
        // Check buffered batch first
        if let Some(batch) = self.buffered_batch.take() {
            if self.should_display_batch(&batch) {
                return Some(self.render_batch(batch));
            } else {
                self.buffered_batch = Some(batch);
                return None;
            }
        }

        // Get next non-empty batch
        let batch = loop {
            match self.decoder.next() {
                Some(b) if !b.is_empty() => break b,
                Some(_) => continue,
                None => {
                    self.reset_playback();
                    return None;
                }
            }
        };

        // Initialize timing on first batch
        if self.playback_start.is_none() {
            self.playback_start = Some(Instant::now());
            self.first_event_ts = batch.t.first().copied();
            return Some(self.render_batch(batch));
        }

        // Check if ready to display
        if self.should_display_batch(&batch) {
            Some(self.render_batch(batch))
        } else {
            self.buffered_batch = Some(batch);
            None
        }
    }

    fn set_slice(&mut self, slice: SliceBy) {
        self.slice = slice;
        self.reset_playback();
    }

    fn set_playback_speed(&mut self, speed: f32) {
        self.playback_speed = speed.clamp(0.003, 3.0);
        self.playback_start = None;
        self.first_event_ts = None;
    }

    fn apply_biases(&mut self, _: Vec<(usize, u8, i32)>) {
        // No-op for file mode
    }

    fn stop(&mut self) {
        // No-op for file mode
    }

    fn get_header(&self) -> &RawFileHeader {
        &self.header
    }

    fn start_recording(&mut self, recorder: Arc<Mutex<RecordingState>>) {
        self.recorder = Some(recorder.clone());

        // For file mode, copy all event data (everything after header) immediately
        if let Ok(mut rec) = recorder.lock() {
            let mut header_end = 0;
            for (i, &byte) in self.bytes.iter().enumerate() {
                if byte == b'\n' {
                    let next_pos = i + 1;
                    if next_pos < self.bytes.len() && self.bytes[next_pos] != b'%' {
                        header_end = next_pos;
                        break;
                    }
                }
            }
            if header_end < self.bytes.len() {
                rec.buffer.extend_from_slice(&self.bytes[header_end..]);
                web_sys::console::log_1(
                    &format!(
                        "File recording: copied {} bytes of event data",
                        self.bytes.len() - header_end
                    )
                    .into(),
                );
            }
        }
    }

    fn stop_recording(&mut self) {
        self.recorder = None;
    }
}
