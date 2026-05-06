// event_viewer is a WASM-only binary served via trunk.
// For native builds only a stub main is compiled so `cargo check` succeeds.

#[cfg(target_arch = "wasm32")]
mod producers;
#[cfg(target_arch = "wasm32")]
mod recording;
#[cfg(target_arch = "wasm32")]
mod state;
#[cfg(target_arch = "wasm32")]
mod ui;

#[cfg(target_arch = "wasm32")]
use eframe::egui;
#[cfg(target_arch = "wasm32")]
use image::RgbImage;
#[cfg(target_arch = "wasm32")]
use libreeb::{
    biases::{BiasGroup, BIAS_TABLE},
    header::{CameraGeometry, RawFileHeader},
    processors::FrameGenerator,
    RawFileReader, SliceBy,
};
#[cfg(target_arch = "wasm32")]
use producers::{WasmCameraProducer, WasmFileProducer};
#[cfg(target_arch = "wasm32")]
use state::{FpsTracker, InitSlot, PlaybackState, ViewerState, WasmState};
#[cfg(target_arch = "wasm32")]
use std::sync::{Arc, Mutex};
#[cfg(target_arch = "wasm32")]
use ui::{format_bias_file, parse_bias_file, show_canvas, show_info_panel, show_selection_screen, BiasAction, SelectionAction};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::spawn_local;

// ── WASM App ──────────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
struct EventViewerApp {
    state: WasmState,
}

#[cfg(target_arch = "wasm32")]
impl EventViewerApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        EventViewerApp {
            state: WasmState::Selection,
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl eframe::App for EventViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint();
        let old = std::mem::replace(&mut self.state, WasmState::Selection);
        self.state = self.process(ctx, old);
    }
}

#[cfg(target_arch = "wasm32")]
impl EventViewerApp {
    fn process(&mut self, ctx: &egui::Context, state: WasmState) -> WasmState {
        match state {
            // ── Selection ─────────────────────────────────────────────────────
            WasmState::Selection => {
                match show_selection_screen(ctx) {
                    SelectionAction::OpenFile => start_open_file_wasm(),
                    SelectionAction::OpenCamera => start_open_camera_wasm(),
                    SelectionAction::None => WasmState::Selection,
                }
            }

            // ── Opening ───────────────────────────────────────────────────────
            WasmState::Opening { label, init } => {
                let mut cancel = false;
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(60.0);
                        ui.heading("Opening…");
                        ui.add_space(8.0);
                        ui.label(&label);
                        ui.add_space(16.0);
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
                if cancel {
                    return WasmState::Selection;
                }
                match init.lock().ok().and_then(|mut g| g.take()) {
                    Some(Ok(vs)) => WasmState::Viewing(vs),
                    Some(Err(e)) => WasmState::Error(e),
                    None => WasmState::Opening { label, init },
                }
            }

            // ── Viewing ───────────────────────────────────────────────────────
            WasmState::Viewing(mut v) => {
                // Poll for new frame
                if let Some((img, n, rate, timestamp)) = v.producer.poll_frame() {
                    upload_frame(ctx, &mut v, img, n, rate, timestamp);
                }

                // Show UI and get actions
                let actions = show_info_panel(ctx, &mut v);
                show_canvas(ctx, &v);

                // Handle actions
                if actions.close {
                    v.producer.stop();
                    return WasmState::Selection;
                }

                if actions.slice_changed {
                    v.producer.set_slice(SliceBy::Time(v.window_us as u64));
                }

                if actions.speed_changed {
                    v.producer
                        .set_playback_speed(v.playback_state.speed_multiplier);
                }

                // Handle recording actions
                if actions.recording_start {
                    if let Ok(mut rec) = v.recorder.lock() {
                        let header = v.producer.get_header();
                        rec.start(header);
                        web_sys::console::log_1(&"Recording started".into());
                    }
                    
                    // Pass the RecordingState to the producer
                    v.producer.start_recording(v.recorder.clone());
                }

                if actions.recording_stop {
                    // Stop recording in the producer first
                    v.producer.stop_recording();
                    
                    if let Ok(mut rec) = v.recorder.lock() {
                        let buffer = rec.stop();
                        let filename = rec.filename.clone();
                        
                        // Trigger browser download
                        download_file(&filename, &buffer);
                        
                        rec.reset();
                        web_sys::console::log_1(&format!("Recording stopped: {} ({} bytes)", filename, buffer.len()).into());
                    }
                }

                // Handle bias actions
                match actions.bias_action {
                    BiasAction::Save => {
                        let content = format_bias_file(&v.bias_offsets);
                        spawn_local(async move {
                            if let Some(handle) = rfd::AsyncFileDialog::new()
                                .add_filter("Bias files", &["bias"])
                                .set_file_name("biases.bias")
                                .save_file()
                                .await
                            {
                                handle.write(content.as_bytes()).await;
                            }
                        });
                    }
                    BiasAction::Load => {
                        let result_slot = Arc::new(Mutex::new(None));
                        let result_clone = Arc::clone(&result_slot);

                        spawn_local(async move {
                            if let Some(file) = rfd::AsyncFileDialog::new()
                                .add_filter("Bias files", &["bias"])
                                .pick_file()
                                .await
                            {
                                let bytes = file.read().await;
                                let content = String::from_utf8_lossy(&bytes);
                                match parse_bias_file(&content) {
                                    Ok(offsets) => {
                                        *result_clone.lock().unwrap() = Some(offsets);
                                    }
                                    Err(e) => {
                                        web_sys::console::error_1(
                                            &format!("Failed to parse bias file: {}", e).into(),
                                        );
                                    }
                                }
                            }
                        });

                        v.pending_bias_load = Some(result_slot);
                    }
                    BiasAction::Reset => {
                        for (i, bi) in BIAS_TABLE.iter().enumerate() {
                            if let Some(offset) = v.bias_offsets.get_mut(i) {
                                *offset = bi.default_offset;
                            }
                        }
                        // Will be applied below
                    }
                    BiasAction::None => {}
                }

                // Check for completed async bias load
                let mut bias_load_complete = false;
                let mut loaded_offsets = None;
                if let Some(ref slot) = v.pending_bias_load {
                    if let Ok(mut guard) = slot.try_lock() {
                        if let Some(offsets) = guard.take() {
                            loaded_offsets = Some(offsets);
                            bias_load_complete = true;
                        }
                    }
                }
                if bias_load_complete {
                    if let Some(offsets) = loaded_offsets {
                        v.bias_offsets = offsets;
                        // Mark as changed so they get applied
                        let biases = v
                            .bias_defaults
                            .iter()
                            .enumerate()
                            .map(|(i, &factory)| (i, factory, v.bias_offsets[i]))
                            .collect();
                        v.producer.apply_biases(biases);
                    }
                    v.pending_bias_load = None;
                }

                // Apply bias changes if any
                if actions.bias_changed {
                    let biases = v
                        .bias_defaults
                        .iter()
                        .enumerate()
                        .map(|(i, &factory)| (i, factory, v.bias_offsets[i]))
                        .collect();
                    v.producer.apply_biases(biases);
                }

                WasmState::Viewing(v)
            }

            // ── Error ─────────────────────────────────────────────────────────
            WasmState::Error(msg) => {
                let mut back = false;
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(60.0);
                        ui.label(
                            egui::RichText::new("Error")
                                .color(egui::Color32::RED)
                                .heading(),
                        );
                        ui.add_space(8.0);
                        ui.label(&msg);
                        ui.add_space(16.0);
                        if ui.button("Back").clicked() {
                            back = true;
                        }
                    });
                });
                if back {
                    WasmState::Selection
                } else {
                    WasmState::Error(msg)
                }
            }
        }
    }
}

/// Download a file to the browser.
#[cfg(target_arch = "wasm32")]
fn download_file(filename: &str, data: &[u8]) {
    use wasm_bindgen::JsCast;
    use web_sys::{Blob, BlobPropertyBag, HtmlAnchorElement, Url};

    let window = match web_sys::window() {
        Some(w) => w,
        None => {
            web_sys::console::error_1(&"Failed to get window".into());
            return;
        }
    };

    let document = match window.document() {
        Some(d) => d,
        None => {
            web_sys::console::error_1(&"Failed to get document".into());
            return;
        }
    };

    // Create Blob from data
    let array = js_sys::Uint8Array::from(data);
    let blob_parts = js_sys::Array::new();
    blob_parts.push(&array);

    let blob_props = BlobPropertyBag::new();
    blob_props.set_type("application/octet-stream");

    let blob = match Blob::new_with_u8_array_sequence_and_options(&blob_parts, &blob_props) {
        Ok(b) => b,
        Err(e) => {
            web_sys::console::error_1(&format!("Failed to create blob: {:?}", e).into());
            return;
        }
    };

    // Create download link
    let url = match Url::create_object_url_with_blob(&blob) {
        Ok(u) => u,
        Err(e) => {
            web_sys::console::error_1(&format!("Failed to create object URL: {:?}", e).into());
            return;
        }
    };

    let anchor = match document.create_element("a") {
        Ok(a) => match a.dyn_into::<HtmlAnchorElement>() {
            Ok(anchor) => anchor,
            Err(e) => {
                web_sys::console::error_1(&format!("Failed to cast to anchor: {:?}", e).into());
                let _ = Url::revoke_object_url(&url);
                return;
            }
        },
        Err(e) => {
            web_sys::console::error_1(&format!("Failed to create anchor: {:?}", e).into());
            let _ = Url::revoke_object_url(&url);
            return;
        }
    };

    anchor.set_href(&url);
    anchor.set_download(filename);
    anchor.click();

    // Cleanup
    let _ = Url::revoke_object_url(&url);
    
    web_sys::console::log_1(&format!("Downloaded: {}", filename).into());
}

/// Upload frame to GPU texture and update stats.
#[cfg(target_arch = "wasm32")]
fn upload_frame(ctx: &egui::Context, v: &mut ViewerState, img: RgbImage, ev_count: usize, rate: f64, timestamp: u64) {
    let pixels: Vec<egui::Color32> = img
        .pixels()
        .map(|p| egui::Color32::from_rgb(p[0], p[1], p[2]))
        .collect();
    let ci = egui::ColorImage {
        size: [v.width as usize, v.height as usize],
        pixels,
    };
    match &mut v.texture {
        Some(t) => t.set(ci, egui::TextureOptions::NEAREST),
        None => {
            v.texture = Some(ctx.load_texture("frame", ci, egui::TextureOptions::NEAREST));
        }
    }
    v.frame_count += 1;
    v.last_ev_count = ev_count;
    v.last_rate = rate;

    // Update timestamp tracking
    if v.first_timestamp.is_none() {
        v.first_timestamp = Some(timestamp);
    }
    v.last_timestamp = Some(timestamp);

    // Update FPS tracking
    v.fps_tracker.record_frame();
    v.playback_state.actual_fps = v.fps_tracker.current_fps();
}

// ── File opening ──────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn start_open_file_wasm() -> WasmState {
    let init: InitSlot = Arc::new(Mutex::new(None));
    let init2 = Arc::clone(&init);
    spawn_local(async move {
        *init2.lock().unwrap() = Some(open_file_wasm_async().await);
    });
    WasmState::Opening {
        label: "Picking file…".into(),
        init,
    }
}

#[cfg(target_arch = "wasm32")]
async fn open_file_wasm_async() -> Result<ViewerState, String> {
    let file = rfd::AsyncFileDialog::new()
        .add_filter("RAW event files", &["raw"])
        .pick_file()
        .await
        .ok_or("No file selected")?;

    let label = file.file_name();
    let bytes: Arc<[u8]> = file.read().await.into();

    // Parse header; scan geometry if missing.
    let mut probe = RawFileReader::from_reader(std::io::Cursor::new(Arc::clone(&bytes)))
        .map_err(|e| e.to_string())?;
    if probe.needs_geometry_scan() {
        probe.scan_geometry(8);
    }
    let geometry = CameraGeometry {
        width: probe.header.camera_geometry.width.max(1),
        height: probe.header.camera_geometry.height.max(1),
    };
    let header = probe.header;
    let mut header_info: Vec<(String, String)> = header
        .header_dict
        .iter()
        .map(|(k, v)| (k.clone(), v.trim().to_string()))
        .collect();
    header_info.sort_by(|a, b| a.0.cmp(&b.0));

    let producer = Box::new(WasmFileProducer::new(
        Arc::clone(&bytes),
        geometry,
        SliceBy::Time(33_333),
        header,
    ));

    Ok(ViewerState {
        width: geometry.width,
        height: geometry.height,
        label,
        header_info,
        producer,
        texture: None,
        frame_count: 0,
        window_us: 33_333.0,
        last_ev_count: 0,
        last_rate: 0.0,
        is_camera: false,
        bias_defaults: vec![],
        bias_offsets: vec![],
        playback_state: PlaybackState::new_file(),
        fps_tracker: FpsTracker::new(),
        first_timestamp: None,
        last_timestamp: None,
        recorder: Arc::new(Mutex::new(recording::RecordingState::new())),
        pending_bias_load: None,
    })
}

// ── Camera opening ────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn start_open_camera_wasm() -> WasmState {
    let init: InitSlot = Arc::new(Mutex::new(None));
    let init2 = Arc::clone(&init);
    spawn_local(async move {
        *init2.lock().unwrap() = Some(open_camera_wasm_async().await);
    });
    WasmState::Opening {
        label: "Connecting EVK4… (~3 s)".into(),
        init,
    }
}

#[cfg(target_arch = "wasm32")]
async fn open_camera_wasm_async() -> Result<ViewerState, String> {
    use libreeb::camera_web::WasmCameraReader;
    use producers::camera::WasmShared;

    let camera = WasmCameraReader::open()
        .await
        .map_err(|e| format!("{e:?}"))?;
    let geometry = CameraGeometry {
        width: camera.width(),
        height: camera.height(),
    };
    let label = format!("EVK4 — {}", camera.sensor_name());
    let bias_defaults = camera.bias_defaults.clone();

    // Pre-compute the less-noisy default offsets and apply them on first loop tick.
    let bias_offsets: Vec<i32> = BIAS_TABLE.iter().map(|bi| bi.default_offset).collect();
    let initial_biases: Vec<(usize, u8, i32)> = bias_defaults
        .iter()
        .enumerate()
        .filter(|(i, _)| BIAS_TABLE[*i].default_offset != 0)
        .map(|(i, &factory)| (i, factory, BIAS_TABLE[i].default_offset))
        .collect();

    let shared = Arc::new(Mutex::new(WasmShared {
        latest_frame: None,
        pending_slice: None,
        pending_biases: if initial_biases.is_empty() {
            None
        } else {
            Some(initial_biases)
        },
        stop: false,
        recorder: None,
    }));
    let shared_task = Arc::clone(&shared);
    let generator = FrameGenerator::new(geometry);
    
    // Create a minimal header for camera mode
    let header = RawFileHeader {
        header_dict: std::collections::HashMap::new(),
        event_type: libreeb::header::RawEventType::Evt3, // EVK4 uses EVT3
        camera_geometry: geometry,
    };
    let mut slice = SliceBy::Time(33_333);

    spawn_local(async move {
        let mut camera = camera;
        let mut acc = libreeb::EventBatch::new();
        loop {
            // Extract all pending commands and release the lock *before* any await.
            let (should_stop, new_slice, pending_biases) = {
                let mut s = match shared_task.lock() {
                    Ok(g) => g,
                    Err(_) => break,
                };
                (s.stop, s.pending_slice.take(), s.pending_biases.take())
            };
            if should_stop {
                camera.stop().await;
                break;
            }
            if let Some(sl) = new_slice {
                slice = sl;
                acc = libreeb::EventBatch::new();
            }
            // Apply biases outside the lock — each write_bias is an await point.
            if let Some(biases) = pending_biases {
                for (idx, factory, offset) in biases {
                    let _ = camera
                        .write_bias(BIAS_TABLE[idx].addr, factory, offset)
                        .await;
                }
            }
            if let Some((batch, raw_bytes)) = camera.read_batch_with_bytes().await {
                acc.append(batch);
                
                // If recording, append raw bytes to RecordingState.buffer
                if let Ok(s) = shared_task.lock() {
                    if let Some(ref recorder) = s.recorder {
                        if let Ok(mut rec) = recorder.lock() {
                            rec.buffer.extend_from_slice(&raw_bytes);
                        }
                    }
                }
            }
            // Emit a frame when the accumulator satisfies the slice condition.
            let emit = match slice {
                SliceBy::Time(us) => {
                    acc.t.len() >= 2 && acc.t.last().unwrap().saturating_sub(acc.t[0]) >= us
                }
                SliceBy::Count(n) => acc.len() >= n,
                SliceBy::Both(us, n) => {
                    (acc.t.len() >= 2 && acc.t.last().unwrap().saturating_sub(acc.t[0]) >= us)
                        || acc.len() >= n
                }
            };
            if emit && !acc.is_empty() {
                let ev = acc.len();
                let dt = acc.t.last().unwrap().saturating_sub(acc.t[0]);
                let rate = if dt > 0 {
                    ev as f64 / (dt as f64 * 1e-6)
                } else {
                    0.0
                };
                let last_ts = acc.t.last().copied().unwrap_or(0);
                let frame = generator.generate(&acc);
                acc = libreeb::EventBatch::new();
                if let Ok(mut s) = shared_task.lock() {
                    s.latest_frame = Some((frame, ev, rate, last_ts));
                }
            }
        }
    });

    Ok(ViewerState {
        width: geometry.width,
        height: geometry.height,
        label,
        header_info: vec![],
        producer: Box::new(WasmCameraProducer { shared, header }),
        texture: None,
        frame_count: 0,
        window_us: 33_333.0,
        last_ev_count: 0,
        last_rate: 0.0,
        is_camera: true,
        bias_offsets,
        bias_defaults,
        playback_state: PlaybackState::new_camera(),
        fps_tracker: FpsTracker::new(),
        first_timestamp: None,
        last_timestamp: None,
        recorder: Arc::new(Mutex::new(recording::RecordingState::new())),
        pending_bias_load: None,
    })
}

// ── Entry points ──────────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("event_viewer is a WASM-only binary. Use `trunk serve` / `trunk build`.");
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use wasm_bindgen::JsCast;
    wasm_bindgen_futures::spawn_local(async {
        let canvas = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("the_canvas_id"))
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
            .expect("no <canvas id='the_canvas_id'>");

        let opts = eframe::WebOptions::default();
        eframe::WebRunner::new()
            .start(canvas, opts, Box::new(|cc| Ok(Box::new(EventViewerApp::new(cc)))))
            .await
            .expect("eframe start failed");
    });
}
