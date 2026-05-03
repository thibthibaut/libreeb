use eframe::egui;
use image::RgbImage;
use libreeb::{processors::FrameGenerator, RawFileReader, SliceBy, SliceIter};
use std::sync::Arc;

#[cfg(not(target_arch = "wasm32"))]
use libreeb::camera::{list_cameras, CameraDeviceInfo, CameraReader, BIAS_TABLE};

#[cfg(not(target_arch = "wasm32"))]
use {
    std::path::PathBuf,
    std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Mutex,
    },
    std::thread,
};

#[cfg(target_arch = "wasm32")]
use {libreeb::camera_web::BIAS_TABLE, wasm_bindgen_futures::spawn_local};

// ── Embedded test file (WASM only) ────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
static TEST_BYTES: &[u8] = include_bytes!("../../data/test.raw");

// ── Producer trait ────────────────────────────────────────────────────────────

trait Producer: Send + 'static {
    fn poll_frame(&mut self) -> Option<(RgbImage, usize, f64)>; // (image, event_count, rate_ev_s)
    fn set_slice(&mut self, slice: SliceBy);
    fn apply_biases(&mut self, biases: Vec<(usize, u8, i32)>);
    fn stop(&mut self);
}

// ── Native producer ───────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
enum ProducerCmd {
    SetSliceBy(SliceBy),
    ApplyBiases(Vec<(usize, u8, i32)>),
    Stop,
}

#[cfg(not(target_arch = "wasm32"))]
struct NativeProducer {
    frame_rx:  mpsc::Receiver<(RgbImage, usize, f64)>,
    cmd_tx:    mpsc::Sender<ProducerCmd>,
    stop_flag: Arc<AtomicBool>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Producer for NativeProducer {
    fn poll_frame(&mut self) -> Option<(RgbImage, usize, f64)> {
        let mut latest = None;
        while let Ok(item) = self.frame_rx.try_recv() { latest = Some(item); }
        latest
    }
    fn set_slice(&mut self, s: SliceBy) { let _ = self.cmd_tx.send(ProducerCmd::SetSliceBy(s)); }
    fn apply_biases(&mut self, b: Vec<(usize, u8, i32)>) { let _ = self.cmd_tx.send(ProducerCmd::ApplyBiases(b)); }
    fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        let _ = self.cmd_tx.send(ProducerCmd::Stop);
    }
}

// ── WASM file producer ────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
type BoxBatchIter = Box<dyn Iterator<Item = libreeb::EventBatch> + Send + Sync + 'static>;

#[cfg(target_arch = "wasm32")]
struct WasmFileProducer {
    bytes:     &'static [u8],
    generator: FrameGenerator,
    slicer:    SliceIter<BoxBatchIter>,
    slice:     SliceBy,
}

#[cfg(target_arch = "wasm32")]
impl WasmFileProducer {
    fn new(bytes: &'static [u8], width: u32, height: u32, slice: SliceBy) -> Self {
        let slicer = Self::make_slicer(bytes, slice);
        WasmFileProducer { bytes, generator: FrameGenerator::new(width, height), slicer, slice }
    }

    fn make_slicer(bytes: &'static [u8], slice: SliceBy) -> SliceIter<BoxBatchIter> {
        match RawFileReader::from_reader(std::io::Cursor::new(bytes)) {
            Ok(r)  => SliceIter::new(r.into_event_iter(), slice),
            Err(e) => {
                web_sys::console::error_1(&format!("reader error: {e}").into());
                SliceIter::new(
                    Box::new(std::iter::empty::<libreeb::EventBatch>()) as BoxBatchIter,
                    slice,
                )
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl Producer for WasmFileProducer {
    fn poll_frame(&mut self) -> Option<(RgbImage, usize, f64)> {
        loop {
            match self.slicer.next() {
                Some(batch) if !batch.is_empty() => {
                    let n = batch.len();
                    // Compute rate from the timestamps in this slice.
                    let rate = if n >= 2 {
                        let dt_us = batch.t.last().unwrap().saturating_sub(batch.t[0]);
                        if dt_us > 0 { n as f64 / (dt_us as f64 * 1e-6) } else { 0.0 }
                    } else { 0.0 };
                    return Some((self.generator.generate(&batch), n, rate));
                }
                Some(_) => {}
                None => {
                    self.slicer = Self::make_slicer(self.bytes, self.slice);
                    return None;
                }
            }
        }
    }
    fn set_slice(&mut self, slice: SliceBy) {
        self.slice = slice;
        self.slicer = Self::make_slicer(self.bytes, self.slice);
    }
    fn apply_biases(&mut self, _: Vec<(usize, u8, i32)>) {}
    fn stop(&mut self) {}
}

// ── WASM camera producer ──────────────────────────────────────────────────────

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
    fn poll_frame(&mut self) -> Option<(RgbImage, usize, f64)> { self.shared.lock().ok()?.latest_frame.take() }
    fn set_slice(&mut self, s: SliceBy) { if let Ok(mut g) = self.shared.lock() { g.pending_slice = Some(s); } }
    fn apply_biases(&mut self, b: Vec<(usize, u8, i32)>) { if let Ok(mut g) = self.shared.lock() { g.pending_biases = Some(b); } }
    fn stop(&mut self) { if let Ok(mut g) = self.shared.lock() { g.stop = true; } }
}

// ── Shared viewer state ───────────────────────────────────────────────────────

struct ViewerState {
    width:         u32,
    height:        u32,
    label:         String,
    header_info:   Vec<(String, String)>,   // sorted key/value pairs from the file header
    producer:      Box<dyn Producer>,
    texture:       Option<egui::TextureHandle>,
    frame_count:   u64,
    window_us:     f32,
    last_ev_count: usize,
    last_rate:     f64,
}

// ── WASM App ──────────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
struct EventViewerApp { viewer: ViewerState }

#[cfg(target_arch = "wasm32")]
impl EventViewerApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let slice = SliceBy::Time(33_333);

        // First pass: parse header and auto-detect geometry if missing.
        let mut probe = RawFileReader::from_reader(std::io::Cursor::new(TEST_BYTES))
            .expect("failed to parse embedded test.raw header");
        if probe.needs_geometry_scan() {
            probe.scan_geometry(8);
        }
        let width  = probe.header.camera_geometry.width.max(1);
        let height = probe.header.camera_geometry.height.max(1);
        let mut header_info: Vec<(String, String)> = probe.header.header_dict
            .iter()
            .map(|(k, v)| (k.clone(), v.trim().to_string()))
            .collect();
        header_info.sort_by(|a, b| a.0.cmp(&b.0));

        web_sys::console::log_1(&format!("test.raw: {width}x{height}").into());

        // Second pass: real producer using the full bytes.
        let producer = Box::new(WasmFileProducer::new(TEST_BYTES, width, height, slice));

        EventViewerApp {
            viewer: ViewerState {
                width, height, label: "test.raw".into(), header_info,
                producer, texture: None, frame_count: 0,
                window_us: 33_333.0, last_ev_count: 0, last_rate: 0.0,
            },
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl eframe::App for EventViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint();
        let v = &mut self.viewer;

        if let Some((img, ev_count, rate)) = v.producer.poll_frame() {
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

        // ── Left info panel ───────────────────────────────────────────────────
        egui::SidePanel::left("info").min_width(200.0).show(ctx, |ui| {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(&v.label).strong());
            ui.label(format!("{}×{}", v.width, v.height));
            ui.separator();

            ui.label(egui::RichText::new("Playback").strong());
            ui.label(format!("Frame:       #{}", v.frame_count));
            ui.label(format!("Events/slice: {}", v.last_ev_count));
            let rate = v.last_rate;
            if rate >= 1_000_000.0 {
                ui.label(format!("Event rate:   {:.1} Mev/s", rate / 1_000_000.0));
            } else if rate >= 1_000.0 {
                ui.label(format!("Event rate:   {:.1} kev/s", rate / 1_000.0));
            } else {
                ui.label(format!("Event rate:   {:.0} ev/s", rate));
            }
            ui.add_space(4.0);
            ui.label("Slice window (µs):");
            if ui.add(egui::Slider::new(&mut v.window_us, 1_000.0..=200_000.0)
                .step_by(1_000.0).integer()).changed()
            {
                v.producer.set_slice(SliceBy::Time(v.window_us as u64));
            }

            ui.separator();
            ui.label(egui::RichText::new("File header").strong());
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("hdr").num_columns(2).striped(true).show(ui, |ui| {
                    for (k, val) in &v.header_info {
                        ui.label(egui::RichText::new(k).monospace().small());
                        ui.label(egui::RichText::new(val).monospace().small());
                        ui.end_row();
                    }
                });
            });
        });

        // ── Main canvas ───────────────────────────────────────────────────────
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
}

// ── Native App ────────────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
enum NativeState {
    Selection { cameras: Vec<CameraDeviceInfo> },
    Opening   { label: String, is_camera: bool, init: InitSlot },
    Viewing   (ViewerState),
    Error     (String),
}

#[cfg(not(target_arch = "wasm32"))]
type InitSlot = Arc<std::sync::Mutex<Option<Result<ViewerState, String>>>>;

#[cfg(not(target_arch = "wasm32"))]
struct EventViewerApp { state: NativeState }

#[cfg(not(target_arch = "wasm32"))]
impl EventViewerApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        EventViewerApp { state: NativeState::Selection { cameras: list_cameras() } }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl eframe::App for EventViewerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint();
        let old = std::mem::replace(&mut self.state, NativeState::Selection { cameras: vec![] });
        self.state = self.process(ctx, old);
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl EventViewerApp {
    fn process(&mut self, ctx: &egui::Context, state: NativeState) -> NativeState {
        match state {
            NativeState::Selection { cameras } => {
                let mut next = None;
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        ui.heading("libreeb Event Viewer");
                        ui.add_space(20.0);
                        if ui.button("  Open RAW file…  ").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("RAW event files", &["raw"])
                                .pick_file()
                            {
                                next = Some(self.start_open_file(path));
                            }
                        }
                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(10.0);
                        ui.label("USB Cameras:");
                        if cameras.is_empty() {
                            ui.label("(none detected)");
                        } else {
                            for cam in &cameras {
                                if ui.button(format!("  EVK4 #{} — bus {}  ", cam.index, cam.bus)).clicked() {
                                    next = Some(self.start_open_camera(cam.index));
                                }
                            }
                        }
                        if ui.small_button("Refresh").clicked() {
                            next = Some(NativeState::Selection { cameras: list_cameras() });
                        }
                    });
                });
                next.unwrap_or(NativeState::Selection { cameras })
            }

            NativeState::Opening { label, is_camera, init } => {
                let mut cancel = false;
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(60.0);
                        ui.heading("Opening…");
                        ui.label(&label);
                        if is_camera { ui.label("Initialising sensor (~3 s)"); }
                        if ui.button("Cancel").clicked() { cancel = true; }
                    });
                });
                if cancel { return NativeState::Selection { cameras: list_cameras() }; }
                match init.lock().ok().and_then(|mut g| g.take()) {
                    Some(Ok(vs))  => NativeState::Viewing(vs),
                    Some(Err(e))  => NativeState::Error(e),
                    None          => NativeState::Opening { label, is_camera, init },
                }
            }

            NativeState::Viewing(mut v) => {
                if let Some((img, ev_count, rate)) = v.producer.poll_frame() {
                    let pixels: Vec<egui::Color32> = img.pixels()
                        .map(|p| egui::Color32::from_rgb(p[0], p[1], p[2]))
                        .collect();
                    let ci = egui::ColorImage { size: [v.width as usize, v.height as usize], pixels };
                    match &mut v.texture {
                        Some(t) => t.set(ci, egui::TextureOptions::NEAREST),
                        None => { v.texture = Some(ctx.load_texture("frame", ci, egui::TextureOptions::NEAREST)); }
                    }
                    v.frame_count  += 1;
                    v.last_ev_count = ev_count;
                    v.last_rate     = rate;
                }

                let mut close = false;
                let mut slice_changed = false;

                egui::SidePanel::left("info").min_width(200.0).show(ctx, |ui| {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(&v.label).strong());
                    ui.label(format!("{}×{}", v.width, v.height));
                    ui.separator();

                    ui.label(egui::RichText::new("Playback").strong());
                    ui.label(format!("Frame:        #{}", v.frame_count));
                    ui.label(format!("Events/slice:  {}", v.last_ev_count));
                    let rate = v.last_rate;
                    if rate >= 1_000_000.0 {
                        ui.label(format!("Event rate:    {:.1} Mev/s", rate / 1_000_000.0));
                    } else if rate >= 1_000.0 {
                        ui.label(format!("Event rate:    {:.1} kev/s", rate / 1_000.0));
                    } else {
                        ui.label(format!("Event rate:    {:.0} ev/s", rate));
                    }
                    ui.add_space(4.0);
                    ui.label("Slice window (µs):");
                    if ui.add(egui::Slider::new(&mut v.window_us, 1_000.0..=200_000.0)
                        .step_by(1_000.0).integer()).changed()
                    {
                        slice_changed = true;
                    }

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

                    ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                        if ui.button("Close").clicked() { close = true; }
                    });
                });

                egui::CentralPanel::default().show(ctx, |ui| {
                    if let Some(tex) = &v.texture {
                        let avail  = ui.available_size();
                        let aspect = v.width as f32 / v.height as f32;
                        let size   = if avail.x / avail.y > aspect {
                            egui::vec2(avail.y * aspect, avail.y)
                        } else { egui::vec2(avail.x, avail.x / aspect) };
                        ui.centered_and_justified(|ui| { ui.image((tex.id(), size)); });
                    } else {
                        ui.centered_and_justified(|ui| { ui.label("Waiting for first frame…"); });
                    }
                });

                if close { v.producer.stop(); return NativeState::Selection { cameras: list_cameras() }; }
                if slice_changed { v.producer.set_slice(SliceBy::Time(v.window_us as u64)); }
                NativeState::Viewing(v)
            }

            NativeState::Error(msg) => {
                let mut back = false;
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.label(egui::RichText::new("Error").color(egui::Color32::RED).heading());
                        ui.label(&msg);
                        if ui.button("Back").clicked() { back = true; }
                    });
                });
                if back { NativeState::Selection { cameras: list_cameras() } } else { NativeState::Error(msg) }
            }
        }
    }

    fn start_open_file(&self, path: PathBuf) -> NativeState {
        let label = path.file_name().map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let init: InitSlot = Arc::new(std::sync::Mutex::new(None));
        let init2 = Arc::clone(&init);
        thread::spawn(move || { *init2.lock().unwrap() = Some(open_file_native(path)); });
        NativeState::Opening { label, is_camera: false, init }
    }

    fn start_open_camera(&self, index: usize) -> NativeState {
        let label = format!("EVK4 #{index}");
        let init: InitSlot = Arc::new(std::sync::Mutex::new(None));
        let init2 = Arc::clone(&init);
        thread::spawn(move || { *init2.lock().unwrap() = Some(open_camera_native(index)); });
        NativeState::Opening { label, is_camera: true, init }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn open_file_native(path: PathBuf) -> Result<ViewerState, String> {
    let mut reader = RawFileReader::new(&path).map_err(|e| e.to_string())?;
    if reader.needs_geometry_scan() { reader.scan_geometry(8); }
    let width  = reader.header.camera_geometry.width.max(1);
    let height = reader.header.camera_geometry.height.max(1);
    let label  = path.file_name().map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let mut header_info: Vec<(String, String)> = reader.header.header_dict
        .iter().map(|(k, v)| (k.clone(), v.trim().to_string())).collect();
    header_info.sort_by(|a, b| a.0.cmp(&b.0));

    let (frame_tx, frame_rx) = mpsc::sync_channel::<(RgbImage, usize, f64)>(2);
    let (cmd_tx, cmd_rx)     = mpsc::channel::<ProducerCmd>();
    let stop_flag  = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop_flag);
    let slice      = SliceBy::Time(33_333);

    thread::spawn(move || {
        let generator = FrameGenerator::new(width, height);
        let mut current_slice = slice;
        'outer: loop {
            if stop_clone.load(Ordering::Relaxed) { break; }
            let mut r = match RawFileReader::new(&path) {
                Ok(r) => r, Err(_) => break,
            };
            let mut iter = SliceIter::new(r.read_events(), current_slice);
            loop {
                while let Ok(cmd) = cmd_rx.try_recv() {
                    match cmd {
                        ProducerCmd::SetSliceBy(s) => { current_slice = s; iter.set_slice_by(s); }
                        ProducerCmd::Stop => break 'outer,
                        ProducerCmd::ApplyBiases(_) => {}
                    }
                }
                if stop_clone.load(Ordering::Relaxed) { break 'outer; }
                match iter.next() {
                    Some(b) if !b.t.is_empty() => {
                        let n = b.len();
                        let rate = if n >= 2 {
                            let dt = b.t.last().unwrap().saturating_sub(b.t[0]);
                            if dt > 0 { n as f64 / (dt as f64 * 1e-6) } else { 0.0 }
                        } else { 0.0 };
                        if frame_tx.send((generator.generate(&b), n, rate)).is_err() { break 'outer; }
                    }
                    Some(_) => continue,
                    None    => break,
                }
            }
        }
    });

    Ok(ViewerState {
        width, height, label, header_info,
        producer: Box::new(NativeProducer { frame_rx, cmd_tx, stop_flag }),
        texture: None, frame_count: 0,
        window_us: 33_333.0, last_ev_count: 0, last_rate: 0.0,
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn open_camera_native(index: usize) -> Result<ViewerState, String> {
    let camera = CameraReader::open_nth(index).map_err(|e| e.to_string())?;
    let width  = camera.width();
    let height = camera.height();
    let label  = format!("EVK4 #{index} — {}", camera.sensor_name());

    let (frame_tx, frame_rx) = mpsc::sync_channel::<(RgbImage, usize, f64)>(2);
    let (cmd_tx, cmd_rx)     = mpsc::channel::<ProducerCmd>();
    let stop_flag  = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop_flag);
    let slice      = SliceBy::Time(33_333);

    thread::spawn(move || {
        let generator = FrameGenerator::new(width, height);
        let mut iter  = SliceIter::new(camera, slice);
        loop {
            while let Ok(cmd) = cmd_rx.try_recv() {
                match cmd {
                    ProducerCmd::SetSliceBy(s) => iter.set_slice_by(s),
                    ProducerCmd::ApplyBiases(biases) => {
                        for (idx, factory, offset) in biases {
                            let addr = BIAS_TABLE[idx].addr;
                            if let Err(e) = iter.inner_mut().write_bias(addr, factory, offset) {
                                eprintln!("bias error: {e}");
                            }
                        }
                    }
                    ProducerCmd::Stop => { iter.inner_mut().stop(); return; }
                }
            }
            if stop_clone.load(Ordering::Relaxed) { iter.inner_mut().stop(); return; }
            match iter.next() {
                Some(b) if !b.t.is_empty() => {
                    let n = b.len();
                    let rate = if n >= 2 {
                        let dt = b.t.last().unwrap().saturating_sub(b.t[0]);
                        if dt > 0 { n as f64 / (dt as f64 * 1e-6) } else { 0.0 }
                    } else { 0.0 };
                    if frame_tx.send((generator.generate(&b), n, rate)).is_err() { iter.inner_mut().stop(); return; }
                }
                Some(_) => continue,
                None    => { iter.inner_mut().stop(); return; }
            }
        }
    });

    Ok(ViewerState {
        width, height, label, header_info: vec![],
        producer: Box::new(NativeProducer { frame_rx, cmd_tx, stop_flag }),
        texture: None, frame_count: 0,
        window_us: 33_333.0, last_ev_count: 0, last_rate: 0.0,
    })
}

// ── Entry points ──────────────────────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    eframe::run_native(
        "libreeb Event Viewer",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("libreeb Event Viewer")
                .with_inner_size([1280.0, 760.0]),
            ..Default::default()
        },
        Box::new(|cc| Ok(Box::new(EventViewerApp::new(cc)))),
    )
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
            .start(canvas, eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(EventViewerApp::new(cc)))))
            .await
            .expect("failed to start eframe");
    });
}
