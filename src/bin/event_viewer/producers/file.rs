//! File-based event producer with timestamp-based playback pacing.
//!
//! `WasmFileProducer::new` spawns an internal `spawn_local` async loop that:
//!
//! * Decodes the file in 5 ms sensor-time slices (`SliceBy::Time(5_000)`).
//! * Paces delivery against wall-clock time scaled by `playback_speed`.
//! * Loops back to the beginning on EOF.
//! * Pushes each ready [`EventBatch`] into the shared [`TimeWindow`].

use super::Producer;
use crate::recording::RecordingState;
use crate::utils::TimeWindow;
use libreeb::{
    io::header::{CameraGeometry, RawFileHeader},
    EventBatch, EventDecoder, RawFileReader, SliceBy,
};
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use wasm_bindgen_futures::spawn_local;
use web_time::Instant;

// ── Shared control state ──────────────────────────────────────────────────────

struct FileShared {
    stop:          bool,
    playback_speed: f32,
    /// Set by `set_playback_speed`; the async loop resets timing on the next
    /// iteration and clears this flag.
    reset_timing:  bool,
    recorder:      Option<Arc<Mutex<RecordingState>>>,
}

// ── Producer ──────────────────────────────────────────────────────────────────

pub struct WasmFileProducer {
    shared: Arc<Mutex<FileShared>>,
    header: RawFileHeader,
    /// Kept for `start_recording` (copies all event bytes on demand).
    bytes:  Arc<[u8]>,
}

impl WasmFileProducer {
    pub fn new(
        bytes:        Arc<[u8]>,
        _geometry:    CameraGeometry,
        event_buffer: Arc<Mutex<TimeWindow<EventBatch>>>,
        header:       RawFileHeader,
    ) -> Self {
        let shared = Arc::new(Mutex::new(FileShared {
            stop:           false,
            playback_speed: 1.0,
            reset_timing:   false,
            recorder:       None,
        }));

        let shared_loop    = Arc::clone(&shared);
        let bytes_loop     = Arc::clone(&bytes);
        let event_buf_loop = Arc::clone(&event_buffer);

        spawn_local(async move {
            let mut decoder        = make_decoder(Arc::clone(&bytes_loop));
            let mut playback_start: Option<Instant> = None;
            let mut first_event_ts: Option<u64>     = None;
            let mut pending_batch:  Option<EventBatch> = None;

            loop {
                // ── Read control flags ────────────────────────────────────────
                let (stop, speed, reset) = {
                    let mut s = shared_loop.lock().unwrap();
                    let r = std::mem::replace(&mut s.reset_timing, false);
                    (s.stop, s.playback_speed, r)
                };
                if stop { break; }
                if reset {
                    playback_start = None;
                    first_event_ts = None;
                }

                // ── Obtain next batch (pending or freshly decoded) ────────────
                let batch = if let Some(b) = pending_batch.take() {
                    b
                } else {
                    // Pull the next non-empty batch from the decoder.
                    let mut found: Option<EventBatch> = None;
                    'decode: loop {
                        match decoder.next() {
                            Some(b) if !b.is_empty() => {
                                found = Some(b);
                                break 'decode;
                            }
                            Some(_) => continue, // skip empty batches
                            None => {
                                // EOF — recreate decoder (loop the file) and yield once.
                                decoder        = make_decoder(Arc::clone(&bytes_loop));
                                playback_start = None;
                                first_event_ts = None;
                                yield_to_browser().await;
                                break 'decode; // found = None → outer `continue`
                            }
                        }
                    }
                    match found {
                        Some(b) => b,
                        None    => continue, // restart outer loop after EOF reset
                    }
                };

                // ── Initialise timing on first batch of each playthrough ──────
                if playback_start.is_none() {
                    playback_start = Some(Instant::now());
                    first_event_ts = batch.t.first().copied();
                }

                // ── Pacing: is it time to push this batch? ───────────────────
                let ready = match (playback_start, first_event_ts, batch.t.first().copied()) {
                    (Some(start), Some(first_ts), Some(batch_ts)) => {
                        let elapsed_sensor_us = batch_ts.saturating_sub(first_ts);
                        let target_real_us =
                            (elapsed_sensor_us as f64 / speed as f64) as u64;
                        let target =
                            start + std::time::Duration::from_micros(target_real_us);
                        Instant::now() >= target
                    }
                    _ => true, // no timing info yet → push immediately
                };

                if ready {
                    event_buf_loop.lock().unwrap().push(batch);
                } else {
                    // Too early; put batch back and yield to the browser.
                    pending_batch = Some(batch);
                    yield_to_browser().await;
                }
            }
        });

        WasmFileProducer { shared, header, bytes }
    }
}

// ── Producer impl ─────────────────────────────────────────────────────────────

impl Producer for WasmFileProducer {
    fn apply_biases(&mut self, _: Vec<(usize, u8, i32)>) {
        // No-op for file mode.
    }

    fn set_playback_speed(&mut self, speed: f32) {
        if let Ok(mut s) = self.shared.lock() {
            s.playback_speed = speed.clamp(0.003, 3.0);
            s.reset_timing   = true;
        }
    }

    fn stop(&mut self) {
        if let Ok(mut s) = self.shared.lock() {
            s.stop = true;
        }
    }

    fn get_header(&self) -> &RawFileHeader {
        &self.header
    }

    fn start_recording(&mut self, recorder: Arc<Mutex<RecordingState>>) {
        // File mode: copy all event bytes immediately on record start.
        if let Ok(mut rec) = recorder.lock() {
            // Find where the text header ends (first non-comment line).
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
        if let Ok(mut s) = self.shared.lock() {
            s.recorder = Some(recorder);
        }
    }

    fn stop_recording(&mut self) {
        if let Ok(mut s) = self.shared.lock() {
            s.recorder = None;
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn make_decoder(bytes: Arc<[u8]>) -> EventDecoder<EventBatch> {
    match RawFileReader::from_reader(Cursor::new(bytes)) {
        Ok(r) => {
            let mut dec = r.into_decoder();
            dec.set_slice_by(SliceBy::Time(5_000)); // 5 ms slices for smooth pacing
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

/// Yield control back to the browser event loop for one microtask tick.
async fn yield_to_browser() {
    wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(
        &wasm_bindgen::JsValue::UNDEFINED,
    ))
    .await
    .ok();
}
