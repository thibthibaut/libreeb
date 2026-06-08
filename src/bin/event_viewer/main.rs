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
use libreeb::{
    io::biases::BIAS_TABLE,
    io::header::{CameraGeometry, RawEventType, RawFileHeader},
    processing::{FrameGenerator, TimeSurface},
    EventBatch, RawFileReader,
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
use utils::{RateSample, TimeWindow, Timestamped};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::spawn_local;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

#[cfg(target_arch = "wasm32")]
const EVENT_WINDOW_US: u64 = 4_000_000;
#[cfg(target_arch = "wasm32")]
const EVENT_WINDOW_BATCH_CAP: usize = 30_000;

// ── App ───────────────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
struct EventViewerApp {
    state: AppState,
    camera_event_type: RawEventType,
}

#[cfg(target_arch = "wasm32")]
impl EventViewerApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        EventViewerApp {
            state: AppState::Selection,
            camera_event_type: RawEventType::Evt21,
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
            AppState::Selection => match show_selection_screen(ctx, &mut self.camera_event_type) {
                SelectionAction::OpenFile => start_open_file_wasm(),
                SelectionAction::OpenCamera(event_type) => start_open_camera_wasm(event_type),
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
                // ── Detect display-mode change before showing UI ───────────────
                let prev_mode = v.display_mode;

                // ── UI ────────────────────────────────────────────────────────
                let panel_actions = show_info_panel(ctx, &mut v);
                show_timeline_panel(ctx, &mut v);

                if panel_actions.close {
                    v.producer.stop();
                    return AppState::Selection;
                }
                if panel_actions.speed_changed {
                    v.playback_speed = v.playback_speed.clamp(0.001, 3.0);
                    v.producer.set_playback_speed(v.playback_speed);
                }
                if panel_actions.pause_changed {
                    v.producer.set_paused(v.playback_paused);
                }

                // ── Handle mode switch TO TimeSurface ─────────────────────────
                let entered_ts_mode = prev_mode != DisplayMode::TimeSurface
                    && v.display_mode == DisplayMode::TimeSurface;
                if entered_ts_mode {
                    // Reset accumulator and replay whole buffer for catch-up.
                    v.time_surface = TimeSurface::new(v.width, v.height);
                    v.ts_processed_t = 0;
                    // (replay happens below inside the buffer lock)
                }

                // ── Consume new events from shared buffer ─────────────────────
                {
                    let buffer = v.event_buffer.lock().unwrap();

                    if let Some(newest) = buffer.newest_t() {
                        if v.last_processed_ts != 0
                            && newest < v.last_processed_ts.saturating_sub(1_000_000)
                        {
                            v.rate_history.clear();
                            v.first_timestamp = None;
                            v.last_timestamp = None;
                            v.frame_count = 0;
                            v.last_processed_ts = 0;
                            v.render_t = None;
                            v.time_surface = TimeSurface::new(v.width, v.height);
                            v.ts_processed_t = 0;
                        }
                    }

                    let ts_before = v.last_processed_ts;

                    // Update dropped count.
                    v.dropped_count = buffer.dropped_count();

                    // Stats: process all batches newer than last_processed_ts.
                    for batch in buffer
                        .iter()
                        .filter(|b| b.timestamp_us() > ts_before && !b.is_empty())
                    {
                        let t = batch.timestamp_us();
                        let rate = batch_rate(batch);
                        v.rate_history.push(RateSample { t, rate });
                        if v.first_timestamp.is_none() {
                            v.first_timestamp = Some(t);
                        }
                        v.last_timestamp = Some(batch.t.last().copied().unwrap_or(0));
                        v.frame_count += 1;
                        v.last_processed_ts = t;
                    }

                    let oldest_t = buffer.oldest_t();
                    let newest_t = buffer.newest_t();
                    let is_camera = v.is_camera;
                    let playback_paused = v.playback_paused;
                    let playback_speed = v.playback_speed;
                    let render_t = update_render_time(
                        &mut v.render_t,
                        &mut v.last_render_instant,
                        is_camera,
                        playback_paused,
                        playback_speed,
                        oldest_t,
                        newest_t,
                    );

                    // ── TimeSurface: catch-up or incremental update ────────────
                    if v.display_mode == DisplayMode::TimeSurface {
                        if let Some(render_t) = render_t {
                            if v.ts_processed_t > render_t {
                                v.time_surface = TimeSurface::new(v.width, v.height);
                                v.ts_processed_t = 0;
                            }
                            let ts_start = if entered_ts_mode {
                                0u64
                            } else {
                                v.ts_processed_t
                            };
                            for batch in buffer.iter().filter(|b| {
                                let t = b.timestamp_us();
                                t > ts_start && t <= render_t && !b.is_empty()
                            }) {
                                v.time_surface.update(batch);
                                v.ts_processed_t = batch.timestamp_us();
                            }
                            let img = v.time_surface.render(render_t, v.ts_decay_us);
                            upload_texture(
                                ctx,
                                &mut v.ts_texture,
                                "time_surface",
                                &img,
                                v.width,
                                v.height,
                            );
                        }
                    }

                    // ── Normal mode: render from window_us slice ──────────────
                    if v.display_mode == DisplayMode::Normal {
                        if let Some(render_t) = render_t {
                            let cutoff = render_t.saturating_sub(v.window_us as u64);
                            let img =
                                v.frame_generator
                                    .generate_from_batches(buffer.iter().filter(|b| {
                                        let t = b.timestamp_us();
                                        t >= cutoff && t <= render_t
                                    }));
                            upload_texture(
                                ctx,
                                &mut v.frame_texture,
                                "frame",
                                &img,
                                v.width,
                                v.height,
                            );
                        }
                    }
                } // ← buffer lock released

                show_canvas(ctx, &mut v);

                // ── Render FPS ────────────────────────────────────────────────
                v.fps_tracker.record();
                v.render_fps = v.fps_tracker.fps();

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

                // Pending async bias load.
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

// ── Texture upload helper ─────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn upload_texture(
    ctx: &egui::Context,
    slot: &mut Option<egui::TextureHandle>,
    name: &str,
    img: &image::RgbImage,
    width: u32,
    height: u32,
) {
    let pixels: Vec<egui::Color32> = img
        .pixels()
        .map(|p| egui::Color32::from_rgb(p[0], p[1], p[2]))
        .collect();
    let ci = egui::ColorImage {
        size: [width as usize, height as usize],
        pixels,
    };
    match slot {
        Some(t) => t.set(ci, egui::TextureOptions::NEAREST),
        None => *slot = Some(ctx.load_texture(name, ci, egui::TextureOptions::NEAREST)),
    }
}

// ── Batch rate helper ─────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn batch_rate(batch: &EventBatch) -> f64 {
    let n = batch.len();
    if n >= 2 {
        let dt_us = batch.t.last().unwrap().saturating_sub(batch.t[0]);
        if dt_us > 0 {
            n as f64 / (dt_us as f64 * 1e-6)
        } else {
            0.0
        }
    } else {
        0.0
    }
}

#[cfg(target_arch = "wasm32")]
fn update_render_time(
    render_t: &mut Option<u64>,
    last_render_instant: &mut Option<Instant>,
    is_camera: bool,
    playback_paused: bool,
    playback_speed: f32,
    oldest_t: Option<u64>,
    newest_t: Option<u64>,
) -> Option<u64> {
    let newest = newest_t?;
    let oldest = oldest_t.unwrap_or(newest);
    let now = Instant::now();
    let dt_us = last_render_instant
        .replace(now)
        .map(|prev| now.duration_since(prev).as_secs_f64() * 1_000_000.0)
        .unwrap_or(0.0);

    let current = (*render_t).unwrap_or(newest).clamp(oldest, newest);
    let next = if !is_camera {
        // File playback speed is handled by the file producer's slice duration.
        newest
    } else if playback_paused {
        current
    } else if playback_speed >= 0.999 && newest.saturating_sub(current) <= 50_000 {
        newest
    } else {
        let advance_us = (dt_us * playback_speed.clamp(0.001, 3.0) as f64)
            .round()
            .max(1.0) as u64;
        current.saturating_add(advance_us).min(newest)
    };

    let next = next.clamp(oldest, newest);
    *render_t = Some(next);
    Some(next)
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

    let event_buffer = Arc::new(Mutex::new(TimeWindow::<EventBatch>::new(
        EVENT_WINDOW_US,
        EVENT_WINDOW_BATCH_CAP,
    )));
    let producer = Box::new(WasmFileProducer::new(
        Arc::clone(&bytes),
        geometry,
        Arc::clone(&event_buffer),
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
        event_buffer,
        producer,
    ))
}

// ── Camera opening ────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn start_open_camera_wasm(event_type: RawEventType) -> AppState {
    let init: InitSlot = Arc::new(Mutex::new(None));
    let init2 = Arc::clone(&init);
    spawn_local(async move {
        *init2.lock().unwrap() = Some(open_camera_wasm_async(event_type).await);
    });
    AppState::Opening {
        label: format!("Connecting EVK4 ({})… (~3 s)", event_type_label(event_type)),
        init,
    }
}

#[cfg(target_arch = "wasm32")]
async fn open_camera_wasm_async(event_type: RawEventType) -> Result<ViewerState, String> {
    use libreeb::io::camera_web::WasmCameraReader;
    use producers::camera::RoiCommand;

    let camera = WasmCameraReader::open(event_type)
        .await
        .map_err(|e| format!("{e:?}"))?;
    let geometry = CameraGeometry {
        width: camera.width(),
        height: camera.height(),
    };
    let label = format!(
        "EVK4 — {} ({})",
        camera.sensor_name(),
        event_type_label(event_type)
    );
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

    let initial_roi: Option<RoiCommand> = if roi_enabled {
        let x1 = (roi_left + roi_width).saturating_sub(1);
        let y1 = (roi_top + roi_height).saturating_sub(1);
        Some(RoiCommand::Set(roi_left, roi_top, x1, y1))
    } else {
        None
    };

    let header = RawFileHeader {
        header_dict: std::collections::HashMap::new(),
        event_type,
        camera_geometry: geometry,
    };

    let event_buffer = Arc::new(Mutex::new(TimeWindow::<EventBatch>::new(
        EVENT_WINDOW_US,
        EVENT_WINDOW_BATCH_CAP,
    )));
    let producer = Box::new(WasmCameraProducer::new(
        camera,
        geometry,
        Arc::clone(&event_buffer),
        header,
        if initial_biases.is_empty() {
            None
        } else {
            Some(initial_biases)
        },
        initial_roi,
    ));

    let mut vs = new_viewer_state(
        geometry,
        label,
        vec![],
        true,
        Some(sensor_id),
        bias_defaults,
        bias_offsets,
        event_buffer,
        producer,
    );
    // Restore persisted ROI into ViewerState.
    vs.roi_left = roi_left;
    vs.roi_top = roi_top;
    vs.roi_width = roi_width;
    vs.roi_height = roi_height;
    vs.roi_enabled = roi_enabled;
    Ok(vs)
}

#[cfg(target_arch = "wasm32")]
fn event_type_label(event_type: RawEventType) -> &'static str {
    match event_type {
        RawEventType::Evt21 => "EVT21",
        RawEventType::Evt3 => "EVT3",
        RawEventType::Evt2 => "EVT2",
        RawEventType::Evt4 => "EVT4",
    }
}

// ── ViewerState constructor helper ────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
#[allow(clippy::too_many_arguments)]
fn new_viewer_state(
    geometry: CameraGeometry,
    label: String,
    header_info: Vec<(String, String)>,
    is_camera: bool,
    sensor_id: Option<u32>,
    bias_defaults: Vec<u8>,
    bias_offsets: Vec<i32>,
    event_buffer: Arc<Mutex<TimeWindow<EventBatch>>>,
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

        event_buffer,

        first_timestamp: None,
        last_timestamp: None,
        frame_count: 0,
        fps_tracker: FpsTracker::new(),
        render_fps: 0.0,
        last_processed_ts: 0,
        dropped_count: 0,

        window_us: 33_333.0,
        playback_speed: 1.0,
        playback_paused: false,
        render_t: None,
        last_render_instant: None,

        display_mode: DisplayMode::Normal,
        frame_texture: None,
        frame_generator: FrameGenerator::new(geometry),

        time_surface: TimeSurface::new(geometry.width, geometry.height),
        ts_texture: None,
        ts_decay_us: 30_000,
        ts_processed_t: 0,

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
