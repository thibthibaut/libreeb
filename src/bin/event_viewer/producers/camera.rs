//! Camera-based event producer using WebUSB.
//!
//! `WasmCameraProducer::new` spawns an internal `spawn_local` async loop that
//! decodes USB bulk-transfers into [`EventBatch`] slices (5 ms each) and
//! pushes them directly into the shared [`TimeWindow`].  The UI reads from
//! that window; there is no `poll_frame` call.

use super::Producer;
use crate::recording::RecordingState;
use crate::utils::TimeWindow;
use libreeb::{
    io::biases::BIAS_TABLE,
    io::camera_web::WasmCameraReader,
    io::header::{CameraGeometry, RawFileHeader},
    EventBatch,
};
use std::sync::{Arc, Mutex};
use wasm_bindgen_futures::spawn_local;

/// ROI command queued from the UI task to the camera async loop.
#[derive(Clone, Copy)]
pub enum RoiCommand {
    Set(u32, u32, u32, u32), // x_min, y_min, x_max, y_max
    Clear,
}

/// State shared between the UI thread and the camera async loop.
pub struct WasmShared {
    pub pending_biases: Option<Vec<(usize, u8, i32)>>,
    pub pending_roi: Option<RoiCommand>,
    pub stop: bool,
    pub recorder: Option<Arc<Mutex<RecordingState>>>,
}

pub struct WasmCameraProducer {
    pub shared: Arc<Mutex<WasmShared>>,
    pub header: RawFileHeader,
}

impl WasmCameraProducer {
    /// Open a camera producer.
    ///
    /// The async decode loop is spawned immediately; it pushes 5 ms [`EventBatch`]
    /// slices into `event_buffer` until `stop()` is called.
    pub fn new(
        camera: WasmCameraReader,
        _geometry: CameraGeometry,
        event_buffer: Arc<Mutex<TimeWindow<EventBatch>>>,
        header: RawFileHeader,
        initial_biases: Option<Vec<(usize, u8, i32)>>,
        initial_roi: Option<RoiCommand>,
    ) -> Self {
        let shared = Arc::new(Mutex::new(WasmShared {
            pending_biases: initial_biases,
            pending_roi: initial_roi,
            stop: false,
            recorder: None,
        }));

        let shared_loop = Arc::clone(&shared);
        let event_buf_loop = Arc::clone(&event_buffer);

        spawn_local(async move {
            let mut camera = camera;
            let mut acc = EventBatch::new();
            // Fixed 5 ms slice — window_us is a rendering-only parameter now.
            const SLICE_US: u64 = 5_000;

            loop {
                let (stop, biases, roi_cmd) = {
                    match shared_loop.lock() {
                        Ok(mut s) => (s.stop, s.pending_biases.take(), s.pending_roi.take()),
                        Err(_) => break,
                    }
                };

                if stop {
                    camera.stop().await;
                    break;
                }

                // Apply pending bias writes.
                if let Some(bs) = biases {
                    for (idx, factory, offset) in bs {
                        let _ = camera
                            .write_bias(BIAS_TABLE[idx].addr, factory, offset)
                            .await;
                    }
                }

                // Apply pending ROI command.
                if let Some(cmd) = roi_cmd {
                    match cmd {
                        RoiCommand::Set(x0, y0, x1, y1) => {
                            let _ = camera.set_roi(x0, y0, x1, y1).await;
                        }
                        RoiCommand::Clear => {
                            let _ = camera.clear_roi().await;
                        }
                    }
                }

                // Read one USB bulk transfer.
                if let Some((mut batch, raw)) = camera.read_batch_with_bytes().await {
                    acc.append(&mut batch);

                    // Forward raw bytes to recorder if active.
                    if let Ok(s) = shared_loop.lock() {
                        if let Some(ref rec) = s.recorder {
                            if let Ok(mut r) = rec.lock() {
                                r.buffer.extend_from_slice(&raw);
                            }
                        }
                    }
                } else {
                    // Empty read: yield to the browser event loop to avoid starving egui.
                    wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(
                        &wasm_bindgen::JsValue::UNDEFINED,
                    ))
                    .await
                    .ok();
                }

                // Emit accumulated batch once 5 ms of sensor time has elapsed.
                let emit =
                    acc.t.len() >= 2 && acc.t.last().unwrap().saturating_sub(acc.t[0]) >= SLICE_US;

                if emit && !acc.is_empty() {
                    let batch = std::mem::replace(&mut acc, EventBatch::new());
                    event_buf_loop.lock().unwrap().push(batch);
                }
            }
        });

        WasmCameraProducer { shared, header }
    }
}

impl Producer for WasmCameraProducer {
    fn apply_biases(&mut self, b: Vec<(usize, u8, i32)>) {
        if let Ok(mut g) = self.shared.lock() {
            g.pending_biases = Some(b);
        }
    }

    fn set_roi(&mut self, roi: Option<[u32; 4]>) {
        if let Ok(mut g) = self.shared.lock() {
            g.pending_roi = Some(match roi {
                Some([x_min, y_min, x_max, y_max]) => RoiCommand::Set(x_min, y_min, x_max, y_max),
                None => RoiCommand::Clear,
            });
        }
    }

    fn set_playback_speed(&mut self, _speed: f32) {
        // No-op for camera mode.
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
