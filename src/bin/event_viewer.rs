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

// ── Producer trait ────────────────────────────────────────────────────────────

trait Producer: Send + 'static {
    /// Returns (frame, event_count, event_rate_ev_per_s) for the latest slice.
    fn poll_frame(&mut self) -> Option<(RgbImage, usize, f64)>;
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

// ── WASM file producer (poll-based, no background task) ──────────────────────
//
// One slice is decoded per egui frame (requestAnimationFrame), so the browser
// is never blocked.

#[cfg(target_arch = "wasm32")]
type BoxBatchIter = Box<dyn Iterator<Item = libreeb::EventBatch> + Send + Sync + 'static>;

#[cfg(target_arch = "wasm32")]
struct WasmFileProducer {
    bytes:     Arc<[u8]>,
    generator: FrameGenerator,
    slicer:    SliceIter<BoxBatchIter>,
    slice:     SliceBy,
}

#[cfg(target_arch = "wasm32")]
impl WasmFileProducer {
    fn new(bytes: Arc<[u8]>, width: u32, height: u32, slice: SliceBy) -> Self {
        let slicer = Self::make_slicer(Arc::clone(&bytes), slice);
        WasmFileProducer { bytes, generator: FrameGenerator::new(width, height), slicer, slice }
    }

    fn make_slicer(bytes: Arc<[u8]>, slice: SliceBy) -> SliceIter<BoxBatchIter> {
        match RawFileReader::from_reader(std::io::Cursor::new(bytes)) {
            Ok(r)  => SliceIter::new(r.into_event_iter(), slice),
            Err(e) => {
                web_sys::console::error_1(&format!("WasmFileProducer reader error: {e}").into());
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
                    let rate = if n >= 2 {
                        let dt_us = batch.t.last().unwrap().saturating_sub(batch.t[0]);
                        if dt_us > 0 { n as f64 / (dt_us as f64 * 1e-6) } else { 0.0 }
                    } else { 0.0 };
                    return Some((self.generator.generate(&batch), n, rate));
                }
                Some(_) => {} // empty batch (all TIME_HIGH) — keep going
                None => {
                    // EOF: restart on next call
                    self.slicer = Self::make_slicer(Arc::clone(&self.bytes), self.slice);
                    return None;
                }
            }
        }
    }
    fn set_slice(&mut self, slice: SliceBy) {
        self.slice = slice;
        self.slicer = Self::make_slicer(Arc::clone(&self.bytes), self.slice);
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
}

/// Async init result slot.
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
                show_info_panel(ctx, &mut v, &mut close, &mut slice_changed);
                show_canvas(ctx, &v);
                if close { v.producer.stop(); return WasmState::Selection; }
                if slice_changed { v.producer.set_slice(SliceBy::Time(v.window_us as u64)); }
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

    let shared = Arc::new(std::sync::Mutex::new(WasmShared {
        latest_frame: None, pending_slice: None, pending_biases: None, stop: false,
    }));
    let shared_task = Arc::clone(&shared);
    let generator   = FrameGenerator::new(width, height);
    let mut slice   = SliceBy::Time(33_333);

    spawn_local(async move {
        let mut camera = camera;
        let mut acc = libreeb::EventBatch::new();
        loop {
            {
                let mut s = match shared_task.lock() { Ok(g) => g, Err(_) => break };
                if s.stop { camera.stop().await; break; }
                if let Some(sl) = s.pending_slice.take() {
                    slice = sl;
                    acc = libreeb::EventBatch::new();
                }
                if let Some(biases) = s.pending_biases.take() {
                    for (idx, factory, offset) in biases {
                        let _ = camera.write_bias(BIAS_TABLE[idx].addr, factory, offset).await;
                    }
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
        header_info: bias_defaults.iter().enumerate()
            .map(|(i, v)| (BIAS_TABLE[i].name.to_string(), format!("{v}")))
            .collect(),
        producer: Box::new(WasmCameraProducer { shared }),
        texture: None, frame_count: 0,
        window_us: 33_333.0, last_ev_count: 0, last_rate: 0.0,
    })
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
                                if ui.button(format!(
                                    "  EVK4 #{} — bus {}, addr {}  ",
                                    cam.index, cam.bus, cam.address
                                )).clicked() {
                                    next = Some(self.start_open_camera(cam.index));
                                }
                            }
                        }
                        ui.add_space(6.0);
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
                        ui.add_space(8.0);
                        ui.label(&label);
                        if is_camera { ui.add_space(4.0); ui.label("Initialising sensor (~3 s)"); }
                        ui.add_space(16.0);
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
                if let Some((img, n, rate)) = v.producer.poll_frame() {
                    upload_frame(ctx, &mut v, img, n, rate);
                }
                let mut close = false;
                let mut slice_changed = false;
                show_info_panel(ctx, &mut v, &mut close, &mut slice_changed);
                show_canvas(ctx, &v);
                if close {
                    v.producer.stop();
                    return NativeState::Selection { cameras: list_cameras() };
                }
                if slice_changed { v.producer.set_slice(SliceBy::Time(v.window_us as u64)); }
                NativeState::Viewing(v)
            }

            NativeState::Error(msg) => {
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
                if back { NativeState::Selection { cameras: list_cameras() } }
                else    { NativeState::Error(msg) }
            }
        }
    }

    fn start_open_file(&self, path: PathBuf) -> NativeState {
        let label = path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
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
    let label  = path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let mut header_info: Vec<(String, String)> = reader.header.header_dict
        .iter().map(|(k, v)| (k.clone(), v.trim().to_string())).collect();
    header_info.sort_by(|a, b| a.0.cmp(&b.0));

    let (frame_tx, frame_rx) = mpsc::sync_channel::<(RgbImage, usize, f64)>(2);
    let (cmd_tx,   cmd_rx)   = mpsc::channel::<ProducerCmd>();
    let stop_flag  = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop_flag);
    let slice      = SliceBy::Time(33_333);

    thread::spawn(move || {
        let generator = FrameGenerator::new(width, height);
        let mut cur_slice = slice;
        'outer: loop {
            if stop_clone.load(Ordering::Relaxed) { break; }
            let mut r = match RawFileReader::new(&path) { Ok(r) => r, Err(_) => break };
            let mut iter = SliceIter::new(r.read_events(), cur_slice);
            loop {
                while let Ok(cmd) = cmd_rx.try_recv() {
                    match cmd {
                        ProducerCmd::SetSliceBy(s) => { cur_slice = s; iter.set_slice_by(s); }
                        ProducerCmd::Stop            => break 'outer,
                        ProducerCmd::ApplyBiases(_)  => {}
                    }
                }
                if stop_clone.load(Ordering::Relaxed) { break 'outer; }
                match iter.next() {
                    Some(b) if !b.t.is_empty() => {
                        let n = b.len();
                        let dt = b.t.last().unwrap().saturating_sub(b.t[0]);
                        let rate = if dt > 0 { n as f64 / (dt as f64 * 1e-6) } else { 0.0 };
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
    let (cmd_tx,   cmd_rx)   = mpsc::channel::<ProducerCmd>();
    let stop_flag  = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop_flag);
    let slice      = SliceBy::Time(33_333);

    thread::spawn(move || {
        let generator = FrameGenerator::new(width, height);
        let mut iter  = SliceIter::new(camera, slice);
        loop {
            while let Ok(cmd) = cmd_rx.try_recv() {
                match cmd {
                    ProducerCmd::SetSliceBy(s)      => iter.set_slice_by(s),
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
                    let dt = b.t.last().unwrap().saturating_sub(b.t[0]);
                    let rate = if dt > 0 { n as f64 / (dt as f64 * 1e-6) } else { 0.0 };
                    if frame_tx.send((generator.generate(&b), n, rate)).is_err() {
                        iter.inner_mut().stop(); return;
                    }
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

// ── Shared UI helpers ─────────────────────────────────────────────────────────

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

fn show_info_panel(
    ctx: &egui::Context,
    v: &mut ViewerState,
    close: &mut bool,
    slice_changed: &mut bool,
) {
    egui::SidePanel::left("info").min_width(200.0).show(ctx, |ui| {
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

        if !v.header_info.is_empty() {
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
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(EventViewerApp::new(cc)))),
            )
            .await
            .expect("failed to start eframe");
    });
}
