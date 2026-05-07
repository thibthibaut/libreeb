// event_viewer is a WASM-only binary served via trunk.
// For native builds only a stub main is compiled so `cargo check` succeeds.

#[cfg(target_arch = "wasm32")]
use eframe::egui;
#[cfg(target_arch = "wasm32")]
use image::RgbImage;
#[cfg(target_arch = "wasm32")]
use libreeb::{biases::{BiasGroup, BIAS_TABLE}, processors::FrameGenerator, RawFileReader, SliceBy, EventDecoder, EventBatch};
#[cfg(target_arch = "wasm32")]
use std::sync::Arc;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::spawn_local;

// ── Producer trait ────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
trait Producer: Send + 'static {
    /// Returns (frame, event_count, event_rate_ev_per_s) for the latest slice.
    fn poll_frame(&mut self) -> Option<(RgbImage, usize, f64)>;
    fn set_slice(&mut self, slice: SliceBy);
    fn apply_biases(&mut self, biases: Vec<(usize, u8, i32)>);
    fn stop(&mut self);
}

// ── WASM file producer (poll-based, no background task) ──────────────────────
//
// One slice is decoded per egui frame (requestAnimationFrame), so the browser
// is never blocked.

#[cfg(target_arch = "wasm32")]
struct WasmFileProducer {
    bytes:     Arc<[u8]>,
    generator: FrameGenerator,
    decoder:   EventDecoder<EventBatch>,
    slice:     SliceBy,
}

#[cfg(target_arch = "wasm32")]
impl WasmFileProducer {
    fn new(bytes: Arc<[u8]>, width: u32, height: u32, slice: SliceBy) -> Self {
        let decoder = Self::make_decoder(Arc::clone(&bytes), slice);
        WasmFileProducer {
            bytes,
            generator: FrameGenerator::new(width, height),
            decoder,
            slice,
        }
    }

    fn make_decoder(bytes: Arc<[u8]>, slice: SliceBy) -> EventDecoder<EventBatch> {
        use std::io::Cursor;
        match RawFileReader::from_reader(Cursor::new(bytes)) {
            Ok(r) => {
                let mut dec = r.into_decoder();
                dec.set_slice_by(slice);
                dec
            }
            Err(e) => {
                web_sys::console::error_1(&format!("WasmFileProducer reader error: {e}").into());
                // Return a decoder that will yield nothing
                let empty = RawFileReader::from_reader(Cursor::new(Arc::from([] as [u8; 0])));
                match empty {
                    Ok(r) => r.into_decoder(),
                    Err(_) => {
                        // Truly impossible to create — panic is acceptable here
                        panic!("Cannot create empty decoder");
                    }
                }
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl Producer for WasmFileProducer {
    fn poll_frame(&mut self) -> Option<(RgbImage, usize, f64)> {
        loop {
            match self.decoder.next() {
                Some(batch) if !batch.is_empty() => {
                    let n = batch.len();
                    let rate = if n >= 2 {
                        let dt_us = batch.t.last().unwrap().saturating_sub(batch.t[0]);
                        if dt_us > 0 { n as f64 / (dt_us as f64 * 1e-6) } else { 0.0 }
                    } else { 0.0 };
                    return Some((self.generator.generate(&batch), n, rate));
                }
                Some(_) => {} // empty batch — keep going
                None => {
                    // EOF: restart on next call
                    self.decoder = Self::make_decoder(Arc::clone(&self.bytes), self.slice);
                    return None;
                }
            }
        }
    }
    fn set_slice(&mut self, slice: SliceBy) {
        self.slice = slice;
        self.decoder = Self::make_decoder(Arc::clone(&self.bytes), self.slice);
    }
    fn apply_biases(&mut self, _: Vec<(usize, u8, i32)>) {}
    fn stop(&mut self) {}
}

// ── WASM camera producer (async USB via spawn_local) ─────────────────────────

#[cfg(target_arch = "wasm32")]
struct WasmShared {
    latest_frame:   Option<(RgbImage, usize, f64)>,
    pending_slice:  Option<SliceBy>,
    pending_biases: Option<Vec<(usize, u8, i32)>>,
    stop:           bool,
}

#[cfg(target_arch = "wasm32")]
struct WasmCameraProducer {
    shared: std::sync::Arc<std::sync::Mutex<WasmShared>>,
}

#[cfg(target_arch = "wasm32")]
impl Producer for WasmCameraProducer {
    fn poll_frame(&mut self) -> Option<(RgbImage, usize, f64)> {
        self.shared.lock().ok()?.latest_frame.take()
    }
    fn set_slice(&mut self, s: SliceBy) {
        if let Ok(mut g) = self.shared.lock() { g.pending_slice = Some(s); }
    }
    fn apply_biases(&mut self, b: Vec<(usize, u8, i32)>) {
        if let Ok(mut g) = self.shared.lock() { g.pending_biases = Some(b); }
    }
    fn stop(&mut self) {
        if let Ok(mut g) = self.shared.lock() { g.stop = true; }
    }
}

// ── Shared viewer state ───────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
struct ViewerState {
    width:         u32,
    height:        u32,
    label:         String,
    header_info:   Vec<(String, String)>,
    producer:      Box<dyn Producer>,
    texture:       Option<egui::TextureHandle>,
    frame_count:   u64,
    window_us:     f32,
    last_ev_count: usize,
    last_rate:     f64,
    // Camera-mode bias controls (empty for file mode)
    is_camera:     bool,
    bias_defaults: Vec<u8>,   // factory idac_ctl per BIAS_TABLE entry
    bias_offsets:  Vec<i32>,  // current offset per entry, starts at 0
}

/// Async init result slot.
#[cfg(target_arch = "wasm32")]
type InitSlot = Arc<std::sync::Mutex<Option<Result<ViewerState, String>>>>;

// ── WASM App ──────────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
enum WasmState {
    Selection,
    Opening { label: String, init: InitSlot },
    Viewing(ViewerState),
    Error(String),
}

#[cfg(target_arch = "wasm32")]
struct EventViewerApp { state: WasmState }

#[cfg(target_arch = "wasm32")]
impl EventViewerApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        EventViewerApp { state: WasmState::Selection }
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
                let mut next = None;
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        ui.heading("libreeb Event Viewer");
                        ui.add_space(20.0);
                        if ui.button("  Open RAW file…  ").clicked() {
                            next = Some(start_open_file_wasm());
                        }
                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(10.0);
                        ui.label("Live camera (Chromium + WebUSB required):");
                        ui.add_space(6.0);
                        if ui.button("  Connect EVK4…  ").clicked() {
                            next = Some(start_open_camera_wasm());
                        }
                    });
                });
                next.unwrap_or(WasmState::Selection)
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
                        if ui.button("Cancel").clicked() { cancel = true; }
                    });
                });
                if cancel { return WasmState::Selection; }
                match init.lock().ok().and_then(|mut g| g.take()) {
                    Some(Ok(vs))  => WasmState::Viewing(vs),
                    Some(Err(e))  => WasmState::Error(e),
                    None          => WasmState::Opening { label, init },
                }
            }

            // ── Viewing ───────────────────────────────────────────────────────
             WasmState::Viewing(mut v) => {
                 if let Some((img, n, rate)) = v.producer.poll_frame() {
                     upload_frame(ctx, &mut v, img, n, rate);
                 }
                 let mut close = false;
                 let mut slice_changed = false;
                 let mut bias_changed  = false;
                 show_info_panel(ctx, &mut v, &mut close, &mut slice_changed, &mut bias_changed);
                 show_canvas(ctx, &v);
                 if close { v.producer.stop(); return WasmState::Selection; }
                 if slice_changed { v.producer.set_slice(SliceBy::Time(v.window_us as u64)); }
                 if bias_changed {
                     let biases = v.bias_defaults.iter().enumerate()
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
                        ui.label(egui::RichText::new("Error").color(egui::Color32::RED).heading());
                        ui.add_space(8.0);
                        ui.label(&msg);
                        ui.add_space(16.0);
                        if ui.button("Back").clicked() { back = true; }
                    });
                });
                if back { WasmState::Selection } else { WasmState::Error(msg) }
            }
        }
    }
}

/// Spawn an async task that shows a file picker, reads bytes, detects geometry,
/// and deposits the result into an InitSlot.  Returns the Opening state.
#[cfg(target_arch = "wasm32")]
fn start_open_file_wasm() -> WasmState {
    let init: InitSlot = Arc::new(std::sync::Mutex::new(None));
    let init2 = Arc::clone(&init);
    spawn_local(async move {
        *init2.lock().unwrap() = Some(open_file_wasm_async().await);
    });
    WasmState::Opening { label: "Picking file…".into(), init }
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
    if probe.needs_geometry_scan() { probe.scan_geometry(8); }
    let width  = probe.header.camera_geometry.width.max(1);
    let height = probe.header.camera_geometry.height.max(1);
    let mut header_info: Vec<(String, String)> = probe.header.header_dict
        .iter().map(|(k, v)| (k.clone(), v.trim().to_string())).collect();
    header_info.sort_by(|a, b| a.0.cmp(&b.0));

    let producer = Box::new(WasmFileProducer::new(
        Arc::clone(&bytes), width, height, SliceBy::Time(33_333),
    ));
    Ok(ViewerState {
        width, height, label, header_info, producer,
        texture: None, frame_count: 0,
        window_us: 33_333.0, last_ev_count: 0, last_rate: 0.0,
        is_camera: false, bias_defaults: vec![], bias_offsets: vec![],
    })
}

/// Spawn an async task that opens the EVK4 via WebUSB.
#[cfg(target_arch = "wasm32")]
fn start_open_camera_wasm() -> WasmState {
    let init: InitSlot = Arc::new(std::sync::Mutex::new(None));
    let init2 = Arc::clone(&init);
    spawn_local(async move {
        *init2.lock().unwrap() = Some(open_camera_wasm_async().await);
    });
    WasmState::Opening { label: "Connecting EVK4… (~3 s)".into(), init }
}

#[cfg(target_arch = "wasm32")]
async fn open_camera_wasm_async() -> Result<ViewerState, String> {
    use libreeb::camera_web::WasmCameraReader;

    let camera = WasmCameraReader::open().await.map_err(|e| format!("{e:?}"))?;
    let width  = camera.width();
    let height = camera.height();
    let label  = format!("EVK4 — {}", camera.sensor_name());
    let bias_defaults = camera.bias_defaults.clone();

    // Pre-compute the less-noisy default offsets and apply them on first loop tick.
    let bias_offsets: Vec<i32> = BIAS_TABLE.iter().map(|bi| bi.default_offset).collect();
    let initial_biases: Vec<(usize, u8, i32)> = bias_defaults.iter().enumerate()
        .filter(|(i, _)| BIAS_TABLE[*i].default_offset != 0)
        .map(|(i, &factory)| (i, factory, BIAS_TABLE[i].default_offset))
        .collect();

    let shared = Arc::new(std::sync::Mutex::new(WasmShared {
        latest_frame: None, pending_slice: None,
        pending_biases: if initial_biases.is_empty() { None } else { Some(initial_biases) },
        stop: false,
    }));
    let shared_task = Arc::clone(&shared);
    let generator   = FrameGenerator::new(width, height);
    let mut slice   = SliceBy::Time(33_333);

    spawn_local(async move {
        let mut camera = camera;
        let mut acc = libreeb::EventBatch::new();
        loop {
            // Extract all pending commands and release the lock *before* any await.
            let (should_stop, new_slice, pending_biases) = {
                let mut s = match shared_task.lock() { Ok(g) => g, Err(_) => break };
                (s.stop, s.pending_slice.take(), s.pending_biases.take())
            };
            if should_stop { camera.stop().await; break; }
            if let Some(sl) = new_slice {
                slice = sl;
                acc = libreeb::EventBatch::new();
            }
            // Apply biases outside the lock — each write_bias is an await point.
            if let Some(biases) = pending_biases {
                for (idx, factory, offset) in biases {
                    let _ = camera.write_bias(BIAS_TABLE[idx].addr, factory, offset).await;
                }
            }
            if let Some(batch) = camera.read_batch().await { acc.append(batch); }
            // Emit a frame when the accumulator satisfies the slice condition.
            let emit = match slice {
                SliceBy::Time(us) => acc.t.len() >= 2 &&
                    acc.t.last().unwrap().saturating_sub(acc.t[0]) >= us,
                SliceBy::Count(n) => acc.len() >= n,
                SliceBy::Both(us, n) => {
                    (acc.t.len() >= 2 && acc.t.last().unwrap().saturating_sub(acc.t[0]) >= us)
                        || acc.len() >= n
                }
            };
            if emit && !acc.is_empty() {
                let ev = acc.len();
                let dt = acc.t.last().unwrap().saturating_sub(acc.t[0]);
                let rate = if dt > 0 { ev as f64 / (dt as f64 * 1e-6) } else { 0.0 };
                let frame = generator.generate(&acc);
                acc = libreeb::EventBatch::new();
                if let Ok(mut s) = shared_task.lock() {
                    s.latest_frame = Some((frame, ev, rate));
                }
            }
        }
    });

    Ok(ViewerState {
        width, height, label,
        header_info: vec![],
        producer: Box::new(WasmCameraProducer { shared }),
        texture: None, frame_count: 0,
        window_us: 33_333.0, last_ev_count: 0, last_rate: 0.0,
        is_camera: true,
        bias_offsets,
        bias_defaults,
    })
}

// ── Shared UI helpers ─────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn upload_frame(
    ctx: &egui::Context,
    v: &mut ViewerState,
    img: RgbImage,
    ev_count: usize,
    rate: f64,
) {
    let pixels: Vec<egui::Color32> = img.pixels()
        .map(|p| egui::Color32::from_rgb(p[0], p[1], p[2]))
        .collect();
    let ci = egui::ColorImage { size: [v.width as usize, v.height as usize], pixels };
    match &mut v.texture {
        Some(t) => t.set(ci, egui::TextureOptions::NEAREST),
        None    => { v.texture = Some(ctx.load_texture("frame", ci, egui::TextureOptions::NEAREST)); }
    }
    v.frame_count  += 1;
    v.last_ev_count = ev_count;
    v.last_rate     = rate;
}

#[cfg(target_arch = "wasm32")]
fn show_info_panel(
    ctx: &egui::Context,
    v: &mut ViewerState,
    close: &mut bool,
    slice_changed: &mut bool,
    bias_changed: &mut bool,
) {
    egui::SidePanel::left("info").min_width(220.0).show(ctx, |ui| {
        ui.add_space(6.0);
        ui.label(egui::RichText::new(&v.label).strong());
        ui.label(format!("{}×{}", v.width, v.height));
        ui.separator();

        ui.label(egui::RichText::new("Playback").strong());
        ui.label(format!("Frame:        #{}", v.frame_count));
        ui.label(format!("Events/slice:  {}", v.last_ev_count));
        let rate = v.last_rate;
        ui.label(if rate >= 1_000_000.0 {
            format!("Event rate:    {:.1} Mev/s", rate / 1_000_000.0)
        } else if rate >= 1_000.0 {
            format!("Event rate:    {:.1} kev/s", rate / 1_000.0)
        } else {
            format!("Event rate:    {:.0} ev/s", rate)
        });

        ui.add_space(4.0);
        ui.label("Slice window (µs):");
        if ui.add(
            egui::Slider::new(&mut v.window_us, 1_000.0..=200_000.0)
                .step_by(1_000.0)
                .integer(),
        ).changed() {
            *slice_changed = true;
        }

        if v.is_camera && !v.bias_defaults.is_empty() {
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Biases").strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Reset").on_hover_text("Reset all offsets to defaults").clicked() {
                        for (i, bi) in BIAS_TABLE.iter().enumerate() {
                            if let Some(o) = v.bias_offsets.get_mut(i) { *o = bi.default_offset; }
                        }
                        *bias_changed = true;
                    }
                });
            });
            egui::ScrollArea::vertical().id_salt("bias_scroll").show(ui, |ui| {
                for (group_label, group_variant) in &[
                    ("Bandwidth",  BiasGroup::Bandwidth),
                    ("Contrast",   BiasGroup::Contrast),
                    ("Advanced",   BiasGroup::Advanced),
                ] {
                    egui::CollapsingHeader::new(
                        egui::RichText::new(*group_label).small().strong()
                    )
                    .default_open(true)
                    .show(ui, |ui| {
                        egui::Grid::new(format!("bias_{group_label}"))
                            .num_columns(2)
                            .spacing([6.0, 3.0])
                            .show(ui, |ui| {
                                for (i, bi) in BIAS_TABLE.iter().enumerate() {
                                    if bi.group != *group_variant { continue; }
                                    if let Some(offset) = v.bias_offsets.get_mut(i) {
                                        let factory = v.bias_defaults.get(i).copied().unwrap_or(128);
                                        let abs_val = (factory as i32 + *offset).clamp(0, 255);
                                        // Name + absolute value
                                        ui.label(
                                            egui::RichText::new(bi.name).monospace().small()
                                        ).on_hover_text(bi.hint);
                                        ui.label(
                                            egui::RichText::new(format!("abs {abs_val}"))
                                                .small()
                                                .color(egui::Color32::DARK_GRAY)
                                        );
                                        ui.end_row();
                                        // Full-width slider on next row
                                        ui.scope(|_| {});
                                        let resp = ui.add(
                                            egui::Slider::new(offset, bi.rec_min..=bi.rec_max)
                                                .step_by(1.0)
                                                .integer()
                                                .text("offset"),
                                        ).on_hover_text(bi.hint);
                                        if resp.changed() { *bias_changed = true; }
                                        ui.end_row();
                                    }
                                }
                            });
                    });
                }
            });
        } else if !v.header_info.is_empty() {
            ui.separator();
            ui.label(egui::RichText::new("File header").strong());
            egui::ScrollArea::vertical().id_salt("hdr_scroll").show(ui, |ui| {
                egui::Grid::new("hdr").num_columns(2).striped(true).show(ui, |ui| {
                    for (k, val) in &v.header_info {
                        ui.label(egui::RichText::new(k).monospace().small());
                        ui.label(egui::RichText::new(val).monospace().small());
                        ui.end_row();
                    }
                });
            });
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.add_space(6.0);
            if ui.button("Close source").clicked() { *close = true; }
        });
    });
}

#[cfg(target_arch = "wasm32")]
fn show_canvas(ctx: &egui::Context, v: &ViewerState) {
    egui::CentralPanel::default().show(ctx, |ui| {
        if let Some(tex) = &v.texture {
            let avail  = ui.available_size();
            let aspect = v.width as f32 / v.height as f32;
            let size   = if avail.x / avail.y > aspect {
                egui::vec2(avail.y * aspect, avail.y)
            } else {
                egui::vec2(avail.x, avail.x / aspect)
            };
            ui.centered_and_justified(|ui| { ui.image((tex.id(), size)); });
        } else {
            ui.centered_and_justified(|ui| { ui.label("Waiting for first frame…"); });
        }
    });
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
        eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(EventViewerApp::new(cc)))),
            )
            .await
            .expect("failed to start eframe");
    });
}
