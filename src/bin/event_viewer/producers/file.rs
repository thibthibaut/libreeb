//! File-based event producer with timestamp-based playback pacing.
//!
//! `WasmFileProducer::new` spawns an internal `spawn_local` async loop that:
//!
//! * Decodes one sensor-time slice per display tick.
//! * Scales the slice duration by `playback_speed` while keeping the tick near 60 Hz.
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

const TARGET_FRAME_US: u64 = 16_667;
const TARGET_FRAME_MS: i32 = 16;

// ── Shared control state ──────────────────────────────────────────────────────

struct FileShared {
    stop: bool,
    paused: bool,
    playback_speed: f32,
    recorder: Option<Arc<Mutex<RecordingState>>>,
}

// ── Producer ──────────────────────────────────────────────────────────────────

pub struct WasmFileProducer {
    shared: Arc<Mutex<FileShared>>,
    header: RawFileHeader,
    /// Kept for `start_recording` (copies all event bytes on demand).
    bytes: Arc<[u8]>,
}

impl WasmFileProducer {
    pub fn new(
        bytes: Arc<[u8]>,
        _geometry: CameraGeometry,
        event_buffer: Arc<Mutex<TimeWindow<EventBatch>>>,
        header: RawFileHeader,
    ) -> Self {
        let shared = Arc::new(Mutex::new(FileShared {
            stop: false,
            paused: false,
            playback_speed: 1.0,
            recorder: None,
        }));

        let shared_loop = Arc::clone(&shared);
        let bytes_loop = Arc::clone(&bytes);
        let event_buf_loop = Arc::clone(&event_buffer);

        spawn_local(async move {
            let mut decoder = make_decoder(Arc::clone(&bytes_loop), frame_slice_us(1.0));

            loop {
                // ── Read control flags ────────────────────────────────────────
                let (stop, paused, speed) = {
                    let s = shared_loop.lock().unwrap();
                    (s.stop, s.paused, s.playback_speed)
                };
                if stop {
                    break;
                }
                if paused {
                    sleep_ms(TARGET_FRAME_MS).await;
                    continue;
                }

                decoder.set_slice_by(SliceBy::Time(frame_slice_us(speed)));

                // Pull the next non-empty sensor-time slice from the decoder.
                let mut found: Option<EventBatch> = None;
                'decode: loop {
                    match decoder.next() {
                        Some(b) if !b.is_empty() => {
                            found = Some(b);
                            break 'decode;
                        }
                        Some(_) => continue, // skip empty batches
                        None => {
                            // EOF: recreate decoder.  TimeWindow detects the clock reset.
                            decoder = make_decoder(Arc::clone(&bytes_loop), frame_slice_us(speed));
                            break 'decode;
                        }
                    }
                }

                if let Some(batch) = found {
                    event_buf_loop.lock().unwrap().push(batch);
                }

                sleep_ms(TARGET_FRAME_MS).await;
            }
        });

        WasmFileProducer {
            shared,
            header,
            bytes,
        }
    }
}

// ── Producer impl ─────────────────────────────────────────────────────────────

impl Producer for WasmFileProducer {
    fn apply_biases(&mut self, _: Vec<(usize, u8, i32)>) {
        // No-op for file mode.
    }

    fn set_playback_speed(&mut self, speed: f32) {
        if let Ok(mut s) = self.shared.lock() {
            s.playback_speed = speed.clamp(0.001, 3.0);
        }
    }

    fn set_paused(&mut self, paused: bool) {
        if let Ok(mut s) = self.shared.lock() {
            s.paused = paused;
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

fn make_decoder(bytes: Arc<[u8]>, slice_us: u64) -> EventDecoder<EventBatch> {
    match RawFileReader::from_reader(Cursor::new(bytes)) {
        Ok(r) => {
            let mut dec = r.into_decoder();
            dec.set_slice_by(SliceBy::Time(slice_us));
            dec
        }
        Err(e) => {
            web_sys::console::error_1(&format!("WasmFileProducer decoder error: {e}").into());
            panic!("Cannot create decoder: {e}");
        }
    }
}

fn frame_slice_us(speed: f32) -> u64 {
    ((TARGET_FRAME_US as f64) * speed.clamp(0.001, 3.0) as f64)
        .round()
        .max(1.0) as u64
}

async fn sleep_ms(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        } else {
            let _ = resolve.call0(&wasm_bindgen::JsValue::UNDEFINED);
        }
    });
    wasm_bindgen_futures::JsFuture::from(promise).await.ok();
}
