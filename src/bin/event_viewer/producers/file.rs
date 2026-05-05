//! File-based event producer with timestamp-based playback timing.

use super::{calculate_rate, Producer};
use image::RgbImage;
use libreeb::{header::CameraGeometry, processors::FrameGenerator, EventBatch, RawFileReader, SliceBy, SliceIter};
use std::sync::Arc;
use web_time::Instant;

type BoxBatchIter = Box<dyn Iterator<Item = EventBatch> + Send + Sync + 'static>;

pub struct WasmFileProducer {
    bytes: Arc<[u8]>,
    generator: FrameGenerator,
    slicer: SliceIter<BoxBatchIter>,
    slice: SliceBy,
    
    // Timing state for real-time playback
    playback_speed: f32,
    playback_start: Option<Instant>,
    first_event_ts: Option<u64>,
    buffered_batch: Option<EventBatch>,
}

impl WasmFileProducer {
    pub fn new(bytes: Arc<[u8]>, geometry: CameraGeometry, slice: SliceBy) -> Self {
        let slicer = Self::make_slicer(Arc::clone(&bytes), slice);
        WasmFileProducer {
            bytes,
            generator: FrameGenerator::new(geometry),
            slicer,
            slice,
            playback_speed: 1.0, // Default: real-time
            playback_start: None,
            first_event_ts: None,
            buffered_batch: None,
        }
    }

    fn make_slicer(bytes: Arc<[u8]>, slice: SliceBy) -> SliceIter<BoxBatchIter> {
        match RawFileReader::from_reader(std::io::Cursor::new(bytes)) {
            Ok(r) => SliceIter::new(r.into_event_iter(), slice),
            Err(e) => {
                web_sys::console::error_1(&format!("WasmFileProducer reader error: {e}").into());
                SliceIter::new(
                    Box::new(std::iter::empty::<EventBatch>()) as BoxBatchIter,
                    slice,
                )
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

        // Calculate how much event-time has passed
        let elapsed_event_us = batch_ts - first_ts;

        // Calculate target real-world delay
        // At 1x speed: elapsed_event_us microseconds should take elapsed_event_us real microseconds
        // At 0.01x speed: should take 100x longer
        // At 3x speed: should take 1/3 the time
        let target_real_us = (elapsed_event_us as f64 / self.playback_speed as f64) as u64;
        let target_time = playback_start + std::time::Duration::from_micros(target_real_us);

        // Display if we've reached the target time
        Instant::now() >= target_time
    }

    fn reset_playback(&mut self) {
        self.slicer = Self::make_slicer(Arc::clone(&self.bytes), self.slice);
        self.playback_start = None;
        self.first_event_ts = None;
        self.buffered_batch = None;
    }

    fn render_batch(&self, batch: EventBatch) -> (RgbImage, usize, f64) {
        let n = batch.len();
        let rate = calculate_rate(&batch);
        (self.generator.generate(&batch), n, rate)
    }
}

impl Producer for WasmFileProducer {
    fn poll_frame(&mut self) -> Option<(RgbImage, usize, f64)> {
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
            match self.slicer.next() {
                Some(b) if !b.is_empty() => break b,
                Some(_) => continue, // empty batch (all TIME_HIGH) — keep going
                None => {
                    // EOF: restart on next call
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
        // Don't reset timing - continue with new speed
    }

    fn apply_biases(&mut self, _: Vec<(usize, u8, i32)>) {
        // No-op for file mode
    }

    fn stop(&mut self) {
        // No-op for file mode
    }
}
