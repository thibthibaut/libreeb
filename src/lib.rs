use evt_reader::BufferedDecoder;
use std::io::{self, BufReader, Read};
use thiserror::Error;

// pyo3 — native only with python feature
#[cfg(all(not(target_arch = "wasm32"), feature = "python"))]
use pyo3::prelude::*;
#[cfg(not(target_arch = "wasm32"))]
use std::{
    fs::File,
    path::{Path, PathBuf},
};

// Re-export decoder raw event types as public
pub use evt2::Evt2;
pub use evt2_1::Evt21;
pub use evt3::Evt3;

// Re-export the public EventDecoder (the user-facing API)
pub use evt_reader::EventDecoder;

// Re-export event representation types
pub use evt_repr::{EventAoS, EventBatch, EventCD, EventSoA};

pub mod biases;
pub mod evt2;
pub mod evt2_1;
pub mod evt3;
pub mod evt_reader;
pub mod evt_repr;
pub mod header;
mod macros;
pub mod processors;
pub mod tee_reader;

// Camera module: native uses rusb, WASM uses WebUSB
#[cfg(all(not(target_arch = "wasm32"), feature = "camera"))]
pub mod camera;
#[cfg(target_arch = "wasm32")]
pub mod camera_web;

#[cfg(all(not(target_arch = "wasm32"), feature = "camera"))]
pub use camera::CameraReader;
#[cfg(target_arch = "wasm32")]
pub use camera_web::WasmCameraReader;

pub use header::{CameraGeometry, RawEventType, RawFileHeader};

// ── SliceBy ──────────────────────────────────────────────────────────────────

/// Slicing mode for `EventDecoder`.
///
/// Controls how the decoder batches events when iterating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliceBy {
    /// Slice by time in microseconds. The decoder will yield a batch when
    /// it has accumulated events spanning the given time window.
    Time(u64),
    /// Slice by count of CD events. The decoder will yield a batch when
    /// it has accumulated the given number of events.
    Count(usize),
    /// Slice by whichever limit is reached first: time or count.
    Both(u64, usize),
    /// Decode events in fixed-size chunks from the internal buffer.
    /// This is the "chunked" mode — it processes data in buffer-sized chunks
    /// (currently 256 KB) without enforcing specific time or count constraints.
    /// The actual number of events per batch will vary based on event density
    /// and buffer boundaries.
    ///
    /// This is the default and fastest mode for streaming through data when
    /// you don't need precise batch boundaries.
    Bytes,
}

// ── Error types ──────────────────────────────────────────────────────────────

#[derive(Error, Debug)]
pub enum RawFileReaderError {
    #[cfg(not(target_arch = "wasm32"))]
    #[error("Failed to open file at path {0}")]
    FileOpenError(PathBuf, #[source] io::Error),

    #[error("Failed to read the content of the file")]
    ReadBytesFailed,

    #[error("Failed to parse header")]
    ParseHeaderFailed,

    #[error("Unsupported event type: {0}")]
    UnsupportedEventType(String),

    #[error("Unknown event type: {0}")]
    UnknownEventType(String),

    #[error("No decoder are implemented for event type: {0:?}")]
    DecoderNotImplemented(RawEventType),

    #[error("Unable to find the event type in file header")]
    EventTypeNotFound,

    #[error("An unknown error occurred")]
    Unknown,
}

// ── EventsRepr trait ─────────────────────────────────────────────────────────

/// Trait for decoder output types.
///
/// Defines how decoded CD events are accumulated. `EventBatch` (SoA layout) is
/// the primary implementation. Future implementations could include:
/// - `Vec<EventCD>` (AoS layout)
/// - Events + external triggers bundle
/// - Histograms / time-surface accumulators
///
/// The trait provides default implementations for bulk methods that delegate to
/// `push_cd`. Performance-critical types (like `EventBatch`) override these
/// with optimized bulk operations.
/// Trait for compile-time generic event output representations.
///
/// This trait allows the decoder to output events in different formats at compile time.
/// The default implementation is `EventBatch` (SoA format), but users can implement
/// custom output types for different use cases (e.g., AoS, histograms, triggers+events).
///
/// # Examples
///
/// Using the default `EventBatch` representation:
/// ```no_run
/// # use libreeb::{RawFileReader, EventBatch};
/// let mut reader = RawFileReader::new("data.raw")?;
/// while let Some(batch) = reader.decoder().next() {
///     // batch is EventBatch (default)
///     println!("Got {} events", batch.len());
/// }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// Advanced: using a custom output type with `make_decoder_generic`:
/// ```no_run
/// # use std::io::BufReader;
/// # use std::fs::File;
/// # use libreeb::{make_decoder_generic, header::RawEventType, EventsRepr};
/// # struct MyCustomOutput;
/// # impl Default for MyCustomOutput { fn default() -> Self { MyCustomOutput } }
/// # impl EventsRepr for MyCustomOutput {
/// #     fn push_cd(&mut self, x: u16, y: u16, p: u8, t: u64) {}
/// #     fn len(&self) -> usize { 0 }
/// #     fn last_t(&self) -> Option<u64> { None }
/// # }
/// let file = File::open("data.raw")?;
/// let mut decoder = make_decoder_generic::<_, MyCustomOutput>(
///     BufReader::new(file),
///     RawEventType::Evt3
/// )?;
/// while let Some(output) = decoder.next() {
///     // output is MyCustomOutput
/// }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub trait EventsRepr: Default + Send {
    /// Push a single CD event.
    fn push_cd(&mut self, x: u16, y: u16, p: u8, t: u64);

    /// Bulk push from SoA slices (all slices must be the same length).
    ///
    /// Default iterates and calls `push_cd`. `EventBatch` overrides with memcpy.
    #[inline]
    fn extend_from_soa(&mut self, x: &[u16], y: &[u16], p: &[u8], t: &[u64]) {
        for i in 0..x.len() {
            self.push_cd(x[i], y[i], p[i], t[i]);
        }
    }

    /// Bulk push where only x varies; y, p, t are constant across the group.
    ///
    /// Common pattern in EVT21 (mask expansion) and vectorized EVT3 events.
    /// Default iterates; `EventBatch` overrides with `extend_from_slice` + `resize`.
    #[inline]
    fn extend_constant_ypt(&mut self, xs: &[u16], y: u16, p: u8, t: u64) {
        for &x in xs {
            self.push_cd(x, y, p, t);
        }
    }

    // ── Optional event types (default: discard) ────────────────────────

    /// Push an external trigger event.
    ///
    /// - `channel`: trigger channel/ID (0–15 in EVT3, 0–31 in EVT2/EVT21)
    /// - `value`: trigger value/polarity (0 or 1)
    /// - `t`: timestamp in microseconds
    ///
    /// Default implementation discards the event. Override in output types
    /// that need trigger correlation (e.g. `EventsWithTriggers`).
    #[inline]
    fn push_trigger(&mut self, _channel: u8, _value: u8, _t: u64) {}

    // ── Bookkeeping ──────────────────────────────────────────────────

    /// Number of CD events accumulated so far.
    ///
    /// Triggers and other non-CD events are NOT counted. This value drives
    /// slice budget decisions (max_events).
    fn len(&self) -> usize;

    /// Whether the output contains no CD events.
    #[inline]
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Timestamp of the last CD event, or `None` if empty.
    ///
    /// Used by `EventDecoder` for time-slice anchoring.
    fn last_t(&self) -> Option<u64>;

    /// Hint to reserve capacity for approximately `n` more CD events.
    fn reserve(&mut self, n: usize);
}

// ── Stage: fixed-size stack buffer for batching events ───────────────────────

pub(crate) const STAGE_CAP: usize = 256;

/// Stack-local staging buffer for event accumulation.
///
/// Reduces per-event overhead by batching writes into bulk `extend_from_soa`.
/// The buffer stores events in SoA layout internally and flushes via the
/// generic `EventsRepr::extend_from_soa` method.
pub(crate) struct Stage {
    x: [u16; STAGE_CAP],
    y: [u16; STAGE_CAP],
    p: [u8; STAGE_CAP],
    t: [u64; STAGE_CAP],
    len: usize,
}

impl Stage {
    #[inline(always)]
    pub fn new() -> Self {
        Stage {
            x: [0; STAGE_CAP],
            y: [0; STAGE_CAP],
            p: [0; STAGE_CAP],
            t: [0; STAGE_CAP],
            len: 0,
        }
    }

    /// Push a single event without bounds checking.
    /// Caller must ensure buffer is not full.
    #[inline(always)]
    pub fn push_unchecked(&mut self, x: u16, y: u16, p: u8, t: u64) {
        unsafe {
            *self.x.get_unchecked_mut(self.len) = x;
            *self.y.get_unchecked_mut(self.len) = y;
            *self.p.get_unchecked_mut(self.len) = p;
            *self.t.get_unchecked_mut(self.len) = t;
        }
        self.len += 1;
    }

    /// Flush accumulated events to any `EventsRepr` via bulk extend.
    #[inline(always)]
    pub fn flush<O: EventsRepr>(&mut self, output: &mut O) {
        if self.len > 0 {
            output.extend_from_soa(
                &self.x[..self.len],
                &self.y[..self.len],
                &self.p[..self.len],
                &self.t[..self.len],
            );
            self.len = 0;
        }
    }

    #[inline(always)]
    pub fn is_full(&self) -> bool {
        self.len >= STAGE_CAP
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Push 8 events at once (from SIMD-extracted arrays).
    /// y and t are constant across all 8 events.
    #[inline(always)]
    pub fn push_8(&mut self, x: [u16; 8], y: u16, p: [u8; 8], t: u64) {
        let base = self.len;
        self.x[base..base + 8].copy_from_slice(&x);
        self.y[base..base + 8].fill(y);
        self.p[base..base + 8].copy_from_slice(&p);
        self.t[base..base + 8].fill(t);
        self.len += 8;
    }
}

// ── RawFileReader ─────────────────────────────────────────────────────────────

/// Convenience wrapper that opens a `.raw` file, parses its header, and
/// provides an [`EventDecoder`] for iterating over decoded events.
#[cfg_attr(all(not(target_arch = "wasm32"), feature = "python"), pyclass)]
pub struct RawFileReader {
    pub header: RawFileHeader,
    #[cfg(not(target_arch = "wasm32"))]
    path: Box<Path>,
    decoder: EventDecoder,
}

fn make_decoder<R: Read + Send + Sync + 'static>(
    reader: BufReader<R>,
    event_type: RawEventType,
) -> Result<EventDecoder, RawFileReaderError> {
    make_decoder_generic::<R, EventBatch>(reader, event_type)
}

/// Create an `EventDecoder` with a custom output type.
///
/// This is the generic version of `make_decoder` for advanced users who want
/// a different output format (e.g., events + triggers, histograms, etc.).
pub fn make_decoder_generic<R: Read + Send + Sync + 'static, O: EventsRepr + 'static>(
    reader: BufReader<R>,
    event_type: RawEventType,
) -> Result<EventDecoder<O>, RawFileReaderError> {
    use evt2::Evt2Decoder;
    use evt2_1::Evt21Decoder;
    use evt3::Evt3Decoder;
    use evt_reader::RawFormatDecoder;

    let stream: Box<dyn evt_reader::DecoderStream<O>> = match event_type {
        RawEventType::Evt2 => Box::new(BufferedDecoder::new(reader, Evt2Decoder::new())),
        RawEventType::Evt21 => Box::new(BufferedDecoder::new(reader, Evt21Decoder::new())),
        RawEventType::Evt3 => Box::new(BufferedDecoder::new(reader, Evt3Decoder::new())),
        _ => return Err(RawFileReaderError::DecoderNotImplemented(event_type)),
    };

    Ok(EventDecoder::from_stream(stream))
}

impl RawFileReader {
    /// Open a `.raw` file from the filesystem (native only).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(path: &Path) -> Result<Self, RawFileReaderError> {
        let file =
            File::open(path).map_err(|e| RawFileReaderError::FileOpenError(path.into(), e))?;
        let mut reader = BufReader::with_capacity(64 * 1024, file);
        let header = header::parse_header(&mut reader)?;
        let decoder = make_decoder(reader, header.event_type)?;
        Ok(RawFileReader {
            header,
            path: path.into(),
            decoder,
        })
    }

    /// Create a `RawFileReader` from any `Read` source.
    pub fn from_reader<R: Read + Send + Sync + 'static>(
        reader: R,
    ) -> Result<Self, RawFileReaderError> {
        let mut buf_reader = BufReader::with_capacity(64 * 1024, reader);
        let header = header::parse_header(&mut buf_reader)?;
        let decoder = make_decoder(buf_reader, header.event_type)?;
        Ok(RawFileReader {
            header,
            #[cfg(not(target_arch = "wasm32"))]
            path: std::path::PathBuf::new().into_boxed_path(),
            decoder,
        })
    }

    /// Get a mutable reference to the event decoder for iteration and slice control.
    pub fn decoder(&mut self) -> &mut EventDecoder {
        &mut self.decoder
    }

    /// Consume the reader, returning the owned event decoder.
    pub fn into_decoder(self) -> EventDecoder {
        self.decoder
    }

    /// Return true if the geometry in the header was not found.
    pub fn needs_geometry_scan(&self) -> bool {
        self.header.camera_geometry.width == 0 || self.header.camera_geometry.height == 0
    }

    /// Scan up to `max_batches` event batches to determine geometry from event coordinates.
    pub fn scan_geometry(&mut self, max_batches: usize) {
        let mut max_x: u32 = 0;
        let mut max_y: u32 = 0;
        let mut seen = 0;
        while seen < max_batches {
            match self.decoder.next() {
                Some(batch) => {
                    for evt in &batch {
                        if evt.x as u32 > max_x {
                            max_x = evt.x as u32;
                        }
                        if evt.y as u32 > max_y {
                            max_y = evt.y as u32;
                        }
                    }
                    if !batch.is_empty() {
                        seen += 1;
                    }
                }
                None => break,
            }
        }
        if max_x > 0 || max_y > 0 {
            self.header.camera_geometry.width = max_x + 1;
            self.header.camera_geometry.height = max_y + 1;
        }
    }

    /// Reset by re-opening the file from disk (native only).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn reset(&mut self) {
        if let Ok(fresh) = Self::new(&self.path) {
            *self = fresh;
        }
    }
}

// ── Python bindings (native only) ─────────────────────────────────────────────

#[cfg(all(not(target_arch = "wasm32"), feature = "python"))]
#[pymethods]
impl RawFileReader {
    #[new]
    pub fn py_new(path: &str) -> PyResult<Self> {
        let path = Path::new(path);
        Self::new(path)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("{}", e)))
    }

    /// Get an event decoder for this file. Supports optional slicing.
    pub fn get_decoder(&self) -> PyResult<PyEventDecoder> {
        let file = File::open(&self.path).map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Failed to open file: {}", e))
        })?;
        let mut reader = BufReader::with_capacity(64 * 1024, file);
        let _header = header::parse_header(&mut reader).map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                "Failed to parse header: {}",
                e
            ))
        })?;

        let decoder = make_decoder(reader, self.header.event_type).map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("{}", e))
        })?;

        Ok(PyEventDecoder { decoder })
    }
}

#[cfg(all(not(target_arch = "wasm32"), feature = "python"))]
#[pyclass]
pub struct PyEventDecoder {
    decoder: EventDecoder,
}

#[cfg(all(not(target_arch = "wasm32"), feature = "python"))]
#[pymethods]
impl PyEventDecoder {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(mut slf: PyRefMut<'_, Self>) -> Option<EventBatch> {
        slf.decoder.next()
    }

    /// Set slicing by time (microseconds).
    pub fn set_slice_time(&mut self, micros: u64) {
        self.decoder.set_slice_by(SliceBy::Time(micros));
    }

    /// Set slicing by event count.
    pub fn set_slice_count(&mut self, count: usize) {
        self.decoder.set_slice_by(SliceBy::Count(count));
    }

    /// Set slicing by both time and count (whichever is reached first).
    pub fn set_slice_both(&mut self, micros: u64, count: usize) {
        self.decoder.set_slice_by(SliceBy::Both(micros, count));
    }

    /// Reset to chunked decoding mode (decode in fixed 256 KB buffer-sized chunks).
    ///
    /// This is the default and fastest mode. Each batch will contain whatever events
    /// fit in one buffer read, with no time or count constraints.
    pub fn set_slice_bytes(&mut self) {
        self.decoder.set_slice_by(SliceBy::Bytes);
    }
}

#[cfg(all(not(target_arch = "wasm32"), feature = "python"))]
#[pymodule]
fn libreeb(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<EventBatch>()?;
    m.add_class::<PyEventDecoder>()?;
    m.add_class::<RawFileReader>()?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::fs;
    use std::hash::Hasher;
    use xxhash_rust::xxh64::Xxh64;

    fn compute_hash(decoder: &mut EventDecoder) -> u64 {
        let mut hasher = Xxh64::new(0);
        while let Some(batch) = decoder.next() {
            for evt in &batch {
                hasher.write_u16(evt.x);
                hasher.write_u16(evt.y);
                hasher.write_u8(evt.p);
                hasher.write_u64(evt.t);
            }
        }
        hasher.finish()
    }

    #[rstest]
    #[case::evt3("data/gen4_evt3_hand.raw", "references/gen4_evt3_hand.raw.no_time_shift.xxh")]
    #[case::evt21("data/claque_doigt_evt21.raw", "references/claque_doigt_evt21.raw.no_time_shift.xxh")]
    #[case::evt2("data/blinking_leds.raw", "references/blinking_leds.raw.no_time_shift.xxh")]
    fn test_decoder(#[case] raw_path: &str, #[case] ref_path: &str) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut reader = RawFileReader::new(std::path::Path::new(raw_path))
                .expect("Failed to open input file");
            let hash = compute_hash(reader.decoder());
            let reference_content =
                fs::read_to_string(ref_path).expect("Failed to read reference file");
            let reference: u64 = reference_content
                .trim()
                .parse()
                .expect("Failed to parse reference hash");
            assert_eq!(hash, reference, "Hash mismatch for {}", raw_path);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[rstest]
    #[case::evt3("data/gen4_evt3_hand.raw", 50_000)]
    #[case::evt21("data/claque_doigt_evt21.raw", 50_000)]
    #[case::evt2("data/blinking_leds.raw", 50_000)]
    fn test_slice_by_count(#[case] path: &str, #[case] count: usize) {
        // Count total events without slicing
        let mut reader = RawFileReader::new(std::path::Path::new(path)).unwrap();
        let total: usize = std::iter::from_fn(|| reader.decoder().next())
            .map(|b| b.len())
            .sum();

        // Count with slicing — must produce same total
        let mut reader = RawFileReader::new(std::path::Path::new(path)).unwrap();
        reader.decoder().set_slice_by(SliceBy::Count(count));
        let mut sliced_total = 0;
        let mut prev_len = None;
        while let Some(batch) = reader.decoder().next() {
            assert!(!batch.is_empty());
            // Every slice except possibly the last must be exactly `count`
            if let Some(prev) = prev_len {
                assert_eq!(prev, count, "mid-stream slice has wrong length");
            }
            prev_len = Some(batch.len());
            sliced_total += batch.len();
        }
        assert_eq!(sliced_total, total, "event count changed after slicing");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[rstest]
    #[case::evt3("data/gen4_evt3_hand.raw", 50_000)]
    #[case::evt21("data/claque_doigt_evt21.raw", 50_000)]
    #[case::evt2("data/blinking_leds.raw", 50_000)]
    fn test_slice_by_time(#[case] path: &str, #[case] window_us: u64) {
        // Count total events without slicing
        let mut reader = RawFileReader::new(std::path::Path::new(path)).unwrap();
        let total: usize = std::iter::from_fn(|| reader.decoder().next())
            .map(|b| b.len())
            .sum();

        // Count with time slicing
        let mut reader = RawFileReader::new(std::path::Path::new(path)).unwrap();
        reader.decoder().set_slice_by(SliceBy::Time(window_us));
        let mut sliced_total = 0;
        while let Some(batch) = reader.decoder().next() {
            assert!(!batch.is_empty());
            let first_t = batch.t[0];
            let last_t = *batch.t.last().unwrap();
            assert!(
                last_t < first_t + window_us,
                "slice spans {} us, exceeds window {window_us}",
                last_t - first_t,
            );
            sliced_total += batch.len();
        }
        assert_eq!(sliced_total, total, "event count changed after time-slicing");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn test_set_slice_by_mid_stream() {
        let path = "data/gen4_evt3_hand.raw";

        // Count total
        let mut reader = RawFileReader::new(std::path::Path::new(path)).unwrap();
        let total: usize = std::iter::from_fn(|| reader.decoder().next())
            .map(|b| b.len())
            .sum();

        // Slice with mid-stream change
        let mut reader = RawFileReader::new(std::path::Path::new(path)).unwrap();
        let dec = reader.decoder();
        dec.set_slice_by(SliceBy::Time(50_000));

        let mut sliced_total = 0;
        let mut frame_nr = 0usize;
        while let Some(batch) = dec.next() {
            sliced_total += batch.len();
            frame_nr += 1;
            if frame_nr == 1 {
                dec.set_slice_by(SliceBy::Time(10_000));
            }
        }
        assert_eq!(
            sliced_total, total,
            "event count changed after mid-stream set_slice_by"
        );
    }
}
