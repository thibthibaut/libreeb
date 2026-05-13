// event_viewer is a WASM-only binary served via trunk.
// For native builds only a stub main is compiled so `cargo check` succeeds.

#[cfg(target_arch = "wasm32")]
mod producers;
#[cfg(target_arch = "wasm32")]
mod recording;
#[cfg(target_arch = "wasm32")]
mod state;
#[cfg(target_arch = "wasm32")]
mod storage;
#[cfg(target_arch = "wasm32")]
mod ui;
#[cfg(target_arch = "wasm32")]
mod utils;

#[cfg(target_arch = "wasm32")]
use eframe::egui;
#[cfg(target_arch = "wasm32")]
use image::RgbImage;
#[cfg(target_arch = "wasm32")]
use libreeb::{
    io::biases::BIAS_TABLE,
    io::header::{CameraGeometry, RawFileHeader},
    processing::{FrameGenerator, TimeSurface},
    EventBatch, RawFileReader, SliceBy,
};
#[cfg(target_arch = "wasm32")]
use producers::{WasmCameraProducer, WasmFileProducer};
#[cfg(target_arch = "wasm32")]
use state::{AppState, DisplayMode, FpsTracker, InitSlot, ViewerState};
#[cfg(target_arch = "wasm32")]
use std::sync::{Arc, Mutex};
#[cfg(target_arch = "wasm32")]
use ui::{
    format_bias_file, parse_bias_file, show_canvas, show_info_panel, show_selection_screen,
    show_timeline_panel, BiasAction, SelectionAction,
};
#[cfg(target_arch = "wasm32")]
use utils::{RateSample, TimeWindow};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::spawn_local;

// ── App ───────────────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
struct EventViewerApp {
    state: AppState,
}

#[cfg(target_arch = "wasm32")]
impl EventViewerApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        EventViewerApp {
            state: AppState::Selection,
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl eframe::App for EventViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint();
        let old = std::mem::replace(&mut self.state, AppState::Selection);
        self.state = self.process(ctx, old);
    }
}

#[cfg(target_arch = "wasm32")]
impl EventViewerApp {
    fn process(&mut self, ctx: &egui::Context, state: AppState) -> AppState {
        match state {
            AppState::Selection => match show_selection_screen(ctx) {
                SelectionAction::OpenFile => start_open_file_wasm(),
                SelectionAction::OpenCamera => start_open_camera_wasm(),
                SelectionAction::None => AppState::Selection,
            },

            AppState::Opening { label, init } => {
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
                    return AppState::Selection;
                }
                match init.lock().ok().and_then(|mut g| g.take()) {
                    Some(Ok(vs)) => AppState::Viewing(vs),
                    Some(Err(e)) => AppState::Error(e),
                    None => AppState::Opening { label, init },
                }
            }

            AppState::Viewing(mut v) => {
                // ── Poll frame ────────────────────────────────────────────────
                if let Some((batch, img, _n, rate, ts)) = v.producer.poll_frame() {
                    upload_frame(ctx, &mut v, batch, img, rate, ts);
                }

                // ── UI ────────────────────────────────────────────────────────
                let panel_actions = show_info_panel(ctx, &mut v);
                show_timeline_panel(ctx, &mut v);
                show_canvas(ctx, &mut v);

                // ── ROI → hardware ────────────────────────────────────────────
                if panel_actions.roi_changed && v.is_camera {
                    if v.roi_enabled {
                        let x0 = v.roi_left;
                        let y0 = v.roi_top;
                        let x1 = (v.roi_left + v.roi_width).saturating_sub(1);
                        let y1 = (v.roi_top + v.roi_height).saturating_sub(1);
                        v.producer.set_roi(Some([x0, y0, x1, y1]));
                    } else {
                        v.producer.set_roi(None);
                    }
                    // Persist ROI to localStorage.
                    if let Some(sid) = v.sensor_id {
                        storage::save_roi(
                            sid,
                            v.roi_left,
                            v.roi_top,
                            v.roi_width,
                            v.roi_height,
                            v.roi_enabled,
                        );
                    }
                }

                if panel_actions.close {
                    v.producer.stop();
                    return AppState::Selection;
                }
                if panel_actions.slice_changed {
                    v.producer.set_slice(SliceBy::Time(v.window_us as u64));
                }
                if panel_actions.speed_changed {
                    v.producer.set_playback_speed(v.playback_speed);
                }

                // ── Recording ─────────────────────────────────────────────────
                if panel_actions.recording_start {
                    if let Ok(mut rec) = v.recorder.lock() {
                        rec.start(v.producer.get_header());
                    }
                    v.producer.start_recording(v.recorder.clone());
                }
                if panel_actions.recording_stop {
                    v.producer.stop_recording();
                    if let Ok(mut rec) = v.recorder.lock() {
                        let buf = rec.stop();
                        let fname = rec.filename.clone();
                        download_file(&fname, &buf);
                        rec.reset();
                    }
                }

                // ── Bias file load/save/reset ──────────────────────────────────
                match panel_actions.bias_action {
                    BiasAction::Save => {
                        let content = format_bias_file(&v.bias_offsets);
                        spawn_local(async move {
                            if let Some(h) = rfd::AsyncFileDialog::new()
                                .add_filter("Bias files", &["bias"])
                                .set_file_name("biases.bias")
                                .save_file()
                                .await
                            {
                                let _ = h.write(content.as_bytes()).await;
                            }
                        });
                    }
                    BiasAction::Load => {
                        let slot = Arc::new(Mutex::new(None::<Vec<i32>>));
                        let slot2 = Arc::clone(&slot);
                        spawn_local(async move {
                            if let Some(file) = rfd::AsyncFileDialog::new()
                                .add_filter("Bias files", &["bias"])
                                .pick_file()
                                .await
                            {
                                let bytes = file.read().await;
                                let content = String::from_utf8_lossy(&bytes);
                                if let Ok(offsets) = parse_bias_file(&content) {
                                    *slot2.lock().unwrap() = Some(offsets);
                                }
                            }
                        });
                        v.pending_bias_load = Some(slot);
                    }
                    BiasAction::Reset => {
                        for (i, bi) in BIAS_TABLE.iter().enumerate() {
                            if let Some(o) = v.bias_offsets.get_mut(i) {
                                *o = bi.default_offset;
                            }
                        }
                    }
                    BiasAction::None => {}
                }

                // Pending async bias load
                let mut loaded = None;
                if let Some(ref slot) = v.pending_bias_load {
                    if let Ok(mut g) = slot.try_lock() {
                        loaded = g.take();
                    }
                }
                if let Some(offsets) = loaded {
                    v.bias_offsets = offsets;
                    let biases = v
                        .bias_defaults
                        .iter()
                        .enumerate()
                        .map(|(i, &f)| (i, f, v.bias_offsets[i]))
                        .collect();
                    v.producer.apply_biases(biases);
                    v.pending_bias_load = None;
                }

                if panel_actions.bias_changed {
                    let biases = v
                        .bias_defaults
                        .iter()
                        .enumerate()
                        .map(|(i, &f)| (i, f, v.bias_offsets[i]))
                        .collect();
                    v.producer.apply_biases(biases);
                    if let Some(sid) = v.sensor_id {
                        storage::save_biases(sid, &v.bias_offsets);
                    }
                }

                AppState::Viewing(v)
            }

            AppState::Error(msg) => {
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
                    AppState::Selection
                } else {
                    AppState::Error(msg)
                }
            }
        }
    }
}

// ── Frame upload ──────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn upload_frame(
    ctx: &egui::Context,
    v: &mut ViewerState,
    batch: EventBatch,
    img: RgbImage,
    rate: f64,
    timestamp: u64,
) {
    // ── Normal frame texture ──────────────────────────────────────────────────
    let pixels: Vec<egui::Color32> = img
        .pixels()
        .map(|p| egui::Color32::from_rgb(p[0], p[1], p[2]))
        .collect();
    let ci = egui::ColorImage {
        size: [v.width as usize, v.height as usize],
        pixels,
    };
    match &mut v.frame_texture {
        Some(t) => t.set(ci, egui::TextureOptions::NEAREST),
        None => {
            v.frame_texture = Some(ctx.load_texture("frame", ci, egui::TextureOptions::NEAREST));
        }
    }

    // ── Stats ─────────────────────────────────────────────────────────────────
    v.frame_count += 1;
    if v.first_timestamp.is_none() {
        v.first_timestamp = Some(timestamp);
    }
    v.last_timestamp = Some(timestamp);

    v.fps_tracker.record();
    v.render_fps = v.fps_tracker.fps();

    // ── Event-rate history ────────────────────────────────────────────────────
    v.rate_history.push(RateSample { t: timestamp, rate });

    // ── Time-surface accumulator ──────────────────────────────────────────────
    v.time_surface.update(&batch);

    // Render to texture only when that mode is active (avoids 921K-pixel / 60fps cost).
    if v.display_mode == DisplayMode::TimeSurface {
        let img = v.time_surface.render(timestamp, v.ts_decay_us);
        let pixels: Vec<egui::Color32> = img
            .pixels()
            .map(|p| egui::Color32::from_rgb(p[0], p[1], p[2]))
            .collect();
        let ci = egui::ColorImage {
            size: [v.width as usize, v.height as usize],
            pixels,
        };
        match &mut v.ts_texture {
            Some(t) => t.set(ci, egui::TextureOptions::NEAREST),
            None => {
                v.ts_texture =
                    Some(ctx.load_texture("time_surface", ci, egui::TextureOptions::NEAREST));
            }
        }
    }

    // ── XYT sliding window (always filled, regardless of display mode) ────────
    // TimeWindow handles sensor-time eviction (2 s), clock-reset detection
    // (file loops), and the 60-slice count cap internally.
    v.xyt_buffer.push(batch);
}

// ── Download helper ───────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn download_file(filename: &str, data: &[u8]) {
    use wasm_bindgen::JsCast;
    use web_sys::{Blob, BlobPropertyBag, HtmlAnchorElement, Url};

    let window = match web_sys::window() {
        Some(w) => w,
        None => return,
    };
    let document = match window.document() {
        Some(d) => d,
        None => return,
    };

    let array = js_sys::Uint8Array::from(data);
    let blob_parts = js_sys::Array::new();
    blob_parts.push(&array);
    let props = BlobPropertyBag::new();
    props.set_type("application/octet-stream");

    let blob = match Blob::new_with_u8_array_sequence_and_options(&blob_parts, &props) {
        Ok(b) => b,
        Err(_) => return,
    };
    let url = match Url::create_object_url_with_blob(&blob) {
        Ok(u) => u,
        Err(_) => return,
    };
    if let Ok(a) = document.create_element("a") {
        if let Ok(anchor) = a.dyn_into::<HtmlAnchorElement>() {
            anchor.set_href(&url);
            anchor.set_download(filename);
            anchor.click();
        }
    }
    let _ = Url::revoke_object_url(&url);
}

// ── File opening ──────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn start_open_file_wasm() -> AppState {
    let init: InitSlot = Arc::new(Mutex::new(None));
    let init2 = Arc::clone(&init);
    spawn_local(async move {
        *init2.lock().unwrap() = Some(open_file_wasm_async().await);
    });
    AppState::Opening {
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
        .map(|(k, val)| (k.clone(), val.trim().to_string()))
        .collect();
    header_info.sort_by(|a, b| a.0.cmp(&b.0));

    let producer = Box::new(WasmFileProducer::new(
        Arc::clone(&bytes),
        geometry,
        SliceBy::Time(33_333),
        header,
    ));

    Ok(new_viewer_state(
        geometry,
        label,
        header_info,
        false,
        None,
        vec![],
        vec![],
        producer,
    ))
}

// ── Camera opening ────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn start_open_camera_wasm() -> AppState {
    let init: InitSlot = Arc::new(Mutex::new(None));
    let init2 = Arc::clone(&init);
    spawn_local(async move {
        *init2.lock().unwrap() = Some(open_camera_wasm_async().await);
    });
    AppState::Opening {
        label: "Connecting EVK4… (~3 s)".into(),
        init,
    }
}

#[cfg(target_arch = "wasm32")]
async fn open_camera_wasm_async() -> Result<ViewerState, String> {
    use libreeb::io::camera_web::WasmCameraReader;
    use producers::camera::{RoiCommand, WasmShared};

    let camera = WasmCameraReader::open()
        .await
        .map_err(|e| format!("{e:?}"))?;
    let geometry = CameraGeometry {
        width: camera.width(),
        height: camera.height(),
    };
    let label = format!("EVK4 — {}", camera.sensor_name());
    let sensor_id = camera.sensor_id;
    let bias_defaults = camera.bias_defaults.clone();

    // Restore biases from localStorage, or fall back to defaults.
    let bias_offsets: Vec<i32> = storage::load_biases(sensor_id, BIAS_TABLE.len())
        .unwrap_or_else(|| BIAS_TABLE.iter().map(|bi| bi.default_offset).collect());

    let initial_biases: Vec<(usize, u8, i32)> = bias_defaults
        .iter()
        .enumerate()
        .map(|(i, &f)| (i, f, bias_offsets[i]))
        .collect();

    // Restore ROI from localStorage.
    let (roi_left, roi_top, roi_width, roi_height, roi_enabled) =
        storage::load_roi(sensor_id).unwrap_or((0, 0, geometry.width, geometry.height, false));

    // Build the pending ROI command to apply after the camera loop starts.
    let pending_roi = if roi_enabled {
        let x0 = roi_left;
        let y0 = roi_top;
        let x1 = (roi_left + roi_width).saturating_sub(1);
        let y1 = (roi_top + roi_height).saturating_sub(1);
        Some(RoiCommand::Set(x0, y0, x1, y1))
    } else {
        None
    };

    let shared = Arc::new(Mutex::new(WasmShared {
        latest_result: None,
        pending_slice: None,
        pending_biases: if initial_biases.is_empty() {
            None
        } else {
            Some(initial_biases)
        },
        pending_roi,
        stop: false,
        recorder: None,
    }));
    let shared_task = Arc::clone(&shared);
    let generator = FrameGenerator::new(geometry);

    let header = RawFileHeader {
        header_dict: std::collections::HashMap::new(),
        event_type: libreeb::io::header::RawEventType::Evt3,
        camera_geometry: geometry,
    };
    let mut slice = SliceBy::Time(33_333);

    spawn_local(async move {
        let mut camera = camera;
        let mut acc = libreeb::EventBatch::new();
        loop {
            let (stop, new_slice, biases, roi_cmd) = {
                let mut s = match shared_task.lock() {
                    Ok(g) => g,
                    Err(_) => break,
                };
                (
                    s.stop,
                    s.pending_slice.take(),
                    s.pending_biases.take(),
                    s.pending_roi.take(),
                )
            };
            if stop {
                camera.stop().await;
                break;
            }
            if let Some(sl) = new_slice {
                slice = sl;
                acc = libreeb::EventBatch::new();
            }
            if let Some(bs) = biases {
                for (idx, factory, offset) in bs {
                    let _ = camera
                        .write_bias(BIAS_TABLE[idx].addr, factory, offset)
                        .await;
                }
            }
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

            if let Some((mut batch, raw)) = camera.read_batch_with_bytes().await {
                acc.append(&mut batch);
                if let Ok(s) = shared_task.lock() {
                    if let Some(ref rec) = s.recorder {
                        if let Ok(mut r) = rec.lock() {
                            r.buffer.extend_from_slice(&raw);
                        }
                    }
                }
            } else {
                // Yield to the browser event loop on empty reads to avoid starving egui.
                wasm_bindgen_futures::JsFuture::from(js_sys::Promise::resolve(
                    &wasm_bindgen::JsValue::UNDEFINED,
                ))
                .await
                .ok();
            }

            let emit = match slice {
                SliceBy::Time(us) => {
                    acc.t.len() >= 2 && acc.t.last().unwrap().saturating_sub(acc.t[0]) >= us
                }
                SliceBy::Count(n) => acc.len() >= n,
                SliceBy::Both(us, n) => {
                    (acc.t.len() >= 2 && acc.t.last().unwrap().saturating_sub(acc.t[0]) >= us)
                        || acc.len() >= n
                }
                SliceBy::Bytes => false,
            };
            if emit && !acc.is_empty() {
                let ev = acc.len();
                let dt = acc.t.last().unwrap().saturating_sub(acc.t[0]);
                let rate = if dt > 0 {
                    ev as f64 / (dt as f64 * 1e-6)
                } else {
                    0.0
                };
                let ts = acc.t.last().copied().unwrap_or(0);
                let frame = generator.generate(&acc);
                let batch = std::mem::replace(&mut acc, libreeb::EventBatch::new());
                if let Ok(mut s) = shared_task.lock() {
                    s.latest_result = Some((batch, frame, ev, rate, ts));
                }
            }
        }
    });

    let mut vs = new_viewer_state(
        geometry,
        label,
        vec![],
        true,
        Some(sensor_id),
        bias_defaults,
        bias_offsets,
        Box::new(WasmCameraProducer { shared, header }),
    );
    // Restore persisted ROI into ViewerState.
    vs.roi_left = roi_left;
    vs.roi_top = roi_top;
    vs.roi_width = roi_width;
    vs.roi_height = roi_height;
    vs.roi_enabled = roi_enabled;
    Ok(vs)
}

// ── ViewerState constructor helper ────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn new_viewer_state(
    geometry: CameraGeometry,
    label: String,
    header_info: Vec<(String, String)>,
    is_camera: bool,
    sensor_id: Option<u32>,
    bias_defaults: Vec<u8>,
    bias_offsets: Vec<i32>,
    producer: Box<dyn producers::Producer>,
) -> ViewerState {
    ViewerState {
        width: geometry.width,
        height: geometry.height,
        label,
        header_info,
        is_camera,
        sensor_id,
        producer,

        first_timestamp: None,
        last_timestamp: None,
        frame_count: 0,
        fps_tracker: FpsTracker::new(),
        render_fps: 0.0,

        window_us: 33_333.0,
        playback_speed: 1.0,
        playback_fps: 0.0,

        display_mode: DisplayMode::Normal,
        frame_texture: None,

        time_surface: TimeSurface::new(geometry.width, geometry.height),
        ts_texture: None,
        ts_decay_us: 30_000,

        xyt_buffer: TimeWindow::new(2_000_000, 20),
        xyt_azimuth: 45.0,
        xyt_elevation: 30.0,
        xyt_point_size: 1.5,
        xyt_zoom: 1.0,

        rate_history: TimeWindow::new(300_000_000, 18_000),
        rate_lookback_s: 10.0,

        roi_enabled: false,
        roi_left: 0,
        roi_top: 0,
        roi_width: geometry.width,
        roi_height: geometry.height,

        bias_defaults,
        bias_offsets,
        pending_bias_load: None,

        recorder: Arc::new(Mutex::new(recording::RecordingState::new())),
    }
}

// ── Entry points ──────────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("event_viewer is WASM-only. Use `trunk serve` / `trunk build`.");
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
            .start(
                canvas,
                opts,
                Box::new(|cc| Ok(Box::new(EventViewerApp::new(cc)))),
            )
            .await
            .expect("eframe start failed");
    });
}
