use evt_reader::EvtReader;
use facet::Facet;
use std::{
    collections::HashMap,
    io::{self, BufRead, BufReader, Cursor, Read},
};
use thiserror::Error;

// pyo3 — native only
#[cfg(not(target_arch = "wasm32"))]
use pyo3::prelude::*;
#[cfg(not(target_arch = "wasm32"))]
use std::{
    fs::File,
    path::{Path, PathBuf},
};

// Re-export decoders as public
pub use evt2::*;
pub use evt2_1::*;
pub use evt3::*;

pub mod evt2;
pub mod evt2_1;
pub mod evt3;
mod evt_reader;
mod macros;
pub mod processors;
pub mod slicer;

// Camera module: native uses rusb, WASM uses WebUSB
#[cfg(not(target_arch = "wasm32"))]
pub mod camera;
#[cfg(target_arch = "wasm32")]
pub mod camera_web;

#[cfg(not(target_arch = "wasm32"))]
pub use camera::CameraReader;
#[cfg(target_arch = "wasm32")]
pub use camera_web::WasmCameraReader;

pub use slicer::{SliceBy, SliceIter};

// Error types
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

/// A single decoded Change Detection (CD) event.
///
/// CD events are the primary output of an event-based camera: each one records
/// that a pixel at (`x`, `y`) crossed a log-intensity threshold at time `t`,
/// with polarity `p` (1 = brighter, 0 = darker).
///
/// Timestamps are in microseconds relative to the start of the recording.
#[cfg_attr(not(target_arch = "wasm32"), pyclass)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
pub struct EventCD {
    pub x: u16,
    pub y: u16,
    pub p: u8,
    pub t: u64,
}

/// A batch of [`EventCD`] events in Structure-of-Arrays layout.
///
/// This is the fundamental unit returned by all event iterators.  The four
/// parallel vectors are always the same length.
#[cfg_attr(not(target_arch = "wasm32"), pyclass)]
#[derive(Debug, Clone, Facet)]
pub struct EventBatch {
    pub x: Vec<u16>,
    pub y: Vec<u16>,
    pub p: Vec<u8>,
    pub t: Vec<u64>,
}

impl EventBatch {
    pub fn new() -> Self {
        EventBatch {
            x: Vec::new(),
            y: Vec::new(),
            p: Vec::new(),
            t: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.x.len()
    }

    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
    }

    pub fn with_capacity(cap: usize) -> Self {
        EventBatch {
            x: Vec::with_capacity(cap),
            y: Vec::with_capacity(cap),
            p: Vec::with_capacity(cap),
            t: Vec::with_capacity(cap),
        }
    }

    /// Push from separate field values (used by decoders for maximum efficiency).
    #[inline]
    pub fn push(&mut self, x: u16, y: u16, t: u64, p: u8) {
        self.x.push(x);
        self.y.push(y);
        self.p.push(p);
        self.t.push(t);
    }

    /// Push a single [`EventCD`] (convenience wrapper).
    #[inline]
    pub fn push_event(&mut self, evt: EventCD) {
        self.push(evt.x, evt.y, evt.t, evt.p);
    }

    /// Append all events from `other` into `self`.
    pub fn append(&mut self, mut other: EventBatch) {
        self.x.append(&mut other.x);
        self.y.append(&mut other.y);
        self.p.append(&mut other.p);
        self.t.append(&mut other.t);
    }

    /// Split at index `at`.  `self` retains `[0..at)`, returns `[at..)`.
    pub fn split_off(&mut self, at: usize) -> EventBatch {
        EventBatch {
            x: self.x.split_off(at),
            y: self.y.split_off(at),
            p: self.p.split_off(at),
            t: self.t.split_off(at),
        }
    }
}

pub struct EventBatchIter<'a> {
    batch: &'a EventBatch,
    index: usize,
}

impl<'a> Iterator for EventBatchIter<'a> {
    type Item = EventCD;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.batch.len() {
            let i = self.index;
            self.index += 1;
            Some(EventCD {
                x: self.batch.x[i],
                y: self.batch.y[i],
                p: self.batch.p[i],
                t: self.batch.t[i],
            })
        } else {
            None
        }
    }
}

impl<'a> IntoIterator for &'a EventBatch {
    type Item = EventCD;
    type IntoIter = EventBatchIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        EventBatchIter {
            batch: self,
            index: 0,
        }
    }
}

impl Default for EventBatch {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[pymethods]
impl EventBatch {
    fn __len__(&self) -> usize {
        self.len()
    }

    fn __repr__(&self) -> String {
        format!("EventBatch(len={})", self.len())
    }
}

pub trait EventDecoder {
    type RawEventType: zerocopy::FromBytes + zerocopy::Immutable + zerocopy::KnownLayout + Copy;
    fn new() -> Self;
    fn decode(&mut self, raw_event: &[Self::RawEventType], batch: &mut EventBatch);
}

#[derive(Debug)]
pub enum Endianness {
    Big,
    Little,
}

#[derive(Debug, Clone, Copy)]
pub enum RawEventType {
    Evt2,
    Evt21,
    Evt3,
    Evt4,
}

#[derive(Debug)]
pub struct CameraGeometry {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug)]
pub struct RawFileHeader {
    pub header_dict: HashMap<String, String>,
    pub event_type: RawEventType,
    pub camera_geometry: CameraGeometry,
}

fn parse_header(reader: &mut impl BufRead) -> Result<RawFileHeader, RawFileReaderError> {
    let mut header_dict: HashMap<String, String> = HashMap::new();
    let mut event_type_string = None;
    let mut event_format_string = None;
    let mut cam_width: u32 = 0;
    let mut cam_height: u32 = 0;

    loop {
        let buffer = reader
            .fill_buf()
            .map_err(|_e| RawFileReaderError::ReadBytesFailed)?;

        let next_char = buffer.first().ok_or(RawFileReaderError::ReadBytesFailed)?;

        if *next_char != b'%' {
            break;
        }

        let mut header_line = String::new();
        reader
            .read_line(&mut header_line)
            .map_err(|_e| RawFileReaderError::ReadBytesFailed)?;

        let mut parts = header_line.trim_start_matches('%').trim().splitn(2, ' ');
        let key = parts.next().ok_or(RawFileReaderError::ParseHeaderFailed)?;
        let maybe_value = parts.next();
        if let Some(value) = maybe_value {
            match key {
                "evt" => event_type_string = Some(value.to_string()),
                "geometry" => {
                    if let Some((w, h)) = value.trim().split_once('x') {
                        if let (Ok(w), Ok(h)) = (w.parse(), h.parse()) {
                            cam_width = w;
                            cam_height = h;
                        }
                    }
                }
                "format" => event_format_string = Some(value.to_string()),
                _ => {}
            }
            header_dict.insert(key.to_string(), value.to_string());
        }
    }

    let mut evt_format_str = match (event_format_string, event_type_string) {
        (Some(format), _) => Ok(format),
        (None, Some(evt_type)) => Ok(evt_type),
        (None, None) => Err(RawFileReaderError::EventTypeNotFound),
    }?;

    if evt_format_str.contains(";") {
        let parts: Vec<String> = evt_format_str.split(";").map(|x| x.to_owned()).collect();
        evt_format_str = parts
            .first()
            .ok_or(RawFileReaderError::ParseHeaderFailed)?
            .to_string();

        for part in &parts[1..] {
            if let Some(v) = part.strip_prefix("width=") {
                cam_width = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = part.strip_prefix("height=") {
                cam_height = v.trim().parse().unwrap_or(0);
            }
        }
    }

    let evt_format_normalized = evt_format_str.trim().to_uppercase();
    let event_type = match evt_format_normalized.as_str() {
        "EVT2"  | "2.0" => RawEventType::Evt2,
        "EVT21" | "EVT2.1" | "2.1" => RawEventType::Evt21,
        "EVT3"  | "3.0" => RawEventType::Evt3,
        "EVT4"  | "4.0" => RawEventType::Evt4,
        other   => return Err(RawFileReaderError::UnknownEventType(other.to_string())),
    };

    let header = RawFileHeader {
        header_dict,
        event_type,
        camera_geometry: CameraGeometry { width: cam_width, height: cam_height },
    };
    Ok(header)
}

// ── RawFileReader ─────────────────────────────────────────────────────────────

#[cfg_attr(not(target_arch = "wasm32"), pyclass)]
pub struct RawFileReader {
    pub header: RawFileHeader,
    /// Path is only meaningful on native; empty on WASM.
    #[cfg(not(target_arch = "wasm32"))]
    path: Box<Path>,
    event_iterator: Box<dyn Iterator<Item = EventBatch> + Send + Sync>,
}

fn make_event_iterator<R: Read + Send + Sync + 'static>(
    reader: BufReader<R>,
    event_type: RawEventType,
) -> Result<Box<dyn Iterator<Item = EventBatch> + Send + Sync>, RawFileReaderError> {
    Ok(match event_type {
        RawEventType::Evt2  => Box::new(EvtReader::new(reader, Evt2Decoder::new())),
        RawEventType::Evt21 => Box::new(EvtReader::new(reader, Evt21Decoder::new())),
        RawEventType::Evt3  => Box::new(EvtReader::new(reader, Evt3Decoder::new())),
        _                   => return Err(RawFileReaderError::DecoderNotImplemented(event_type)),
    })
}

impl RawFileReader {
    /// Open a `.raw` file from the filesystem (native only).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(path: &Path) -> Result<Self, RawFileReaderError> {
        let file = File::open(path)
            .map_err(|e| RawFileReaderError::FileOpenError(path.into(), e))?;
        let mut reader = BufReader::with_capacity(64 * 1024, file);
        let header = parse_header(&mut reader)?;
        let event_iterator = make_event_iterator(reader, header.event_type)?;
        Ok(RawFileReader { header, path: path.into(), event_iterator })
    }

    /// Create a `RawFileReader` from any `Read` source.
    ///
    /// On WASM the source is typically a `Cursor<Arc<[u8]>>` built from
    /// browser file bytes.  On native it can be any reader.
    pub fn from_reader<R: Read + Send + Sync + 'static>(
        reader: R,
    ) -> Result<Self, RawFileReaderError> {
        let mut buf_reader = BufReader::with_capacity(64 * 1024, reader);
        let header = parse_header(&mut buf_reader)?;
        let event_iterator = make_event_iterator(buf_reader, header.event_type)?;
        Ok(RawFileReader {
            header,
            #[cfg(not(target_arch = "wasm32"))]
            path: std::path::PathBuf::new().into_boxed_path(),
            event_iterator,
        })
    }

    pub fn read_events<'a>(&'a mut self) -> Box<dyn std::iter::Iterator<Item = EventBatch> + 'a> {
        Box::new(&mut self.event_iterator)
    }

    /// Consume the reader, yielding an owned iterator over all event batches.
    pub fn into_event_iter(self) -> Box<dyn Iterator<Item = EventBatch> + Send + Sync + 'static> {
        self.event_iterator
    }

    /// Return true if the geometry in the header was not found (width or height is 0).
    pub fn needs_geometry_scan(&self) -> bool {
        self.header.camera_geometry.width == 0 || self.header.camera_geometry.height == 0
    }

    /// Scan up to `max_batches` event batches to determine geometry from event coordinates.
    /// Updates `header.camera_geometry` in place.
    pub fn scan_geometry(&mut self, max_batches: usize) {
        let mut max_x: u32 = 0;
        let mut max_y: u32 = 0;
        let mut seen = 0;
        // We iterate the inner iterator directly so we can stop early.
        // NOTE: these events are consumed and lost; call this only when you intend
        // to recreate the reader afterwards (e.g. on WASM from in-memory bytes).
        while seen < max_batches {
            match self.event_iterator.next() {
                Some(batch) => {
                    for evt in &batch {
                        if evt.x as u32 > max_x { max_x = evt.x as u32; }
                        if evt.y as u32 > max_y { max_y = evt.y as u32; }
                    }
                    if !batch.is_empty() { seen += 1; }
                }
                None => break,
            }
        }
        if max_x > 0 || max_y > 0 {
            self.header.camera_geometry.width  = max_x + 1;
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

#[cfg(not(target_arch = "wasm32"))]
#[pymethods]
impl RawFileReader {
    #[new]
    pub fn py_new(path: &str) -> PyResult<Self> {
        let path = Path::new(path);
        Self::new(path)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("{}", e)))
    }

    pub fn get_event_iterator(&self) -> PyResult<EventIterator> {
        let file = File::open(&self.path).map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Failed to open file: {}", e))
        })?;
        let mut reader = BufReader::with_capacity(64 * 1024, file);
        let _header = parse_header(&mut reader).map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                "Failed to parse header: {}",
                e
            ))
        })?;

        let event_iterator: Box<dyn Iterator<Item = EventBatch> + Send + Sync> =
            match self.header.event_type {
                RawEventType::Evt21 => Box::new(EvtReader::new(reader, Evt21Decoder::new())),
                RawEventType::Evt3  => Box::new(EvtReader::new(reader, Evt3Decoder::new())),
                _ => return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                    "Unsupported event type",
                )),
            };

        Ok(EventIterator { inner: event_iterator })
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[pyclass]
pub struct EventIterator {
    inner: Box<dyn Iterator<Item = EventBatch> + Send + Sync>,
}

#[cfg(not(target_arch = "wasm32"))]
#[pymethods]
impl EventIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> { slf }

    fn __next__(mut slf: PyRefMut<'_, Self>) -> Option<EventBatch> { slf.inner.next() }

    pub fn slice_by_time(mut slf: PyRefMut<'_, Self>, micros: u64) -> SliceIterator {
        let inner = std::mem::replace(&mut slf.inner, Box::new(std::iter::empty()));
        SliceIterator { inner: Box::new(SliceIter::new(inner, SliceBy::Time(micros))) }
    }

    pub fn slice_by_count(mut slf: PyRefMut<'_, Self>, count: usize) -> SliceIterator {
        let inner = std::mem::replace(&mut slf.inner, Box::new(std::iter::empty()));
        SliceIterator { inner: Box::new(SliceIter::new(inner, SliceBy::Count(count))) }
    }

    pub fn slice_by_both(mut slf: PyRefMut<'_, Self>, micros: u64, count: usize) -> SliceIterator {
        let inner = std::mem::replace(&mut slf.inner, Box::new(std::iter::empty()));
        SliceIterator { inner: Box::new(SliceIter::new(inner, SliceBy::Both(micros, count))) }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[pyclass]
pub struct SliceIterator {
    inner: Box<dyn Iterator<Item = EventBatch> + Send + Sync>,
}

#[cfg(not(target_arch = "wasm32"))]
#[pymethods]
impl SliceIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> { slf }
    fn __next__(mut slf: PyRefMut<'_, Self>) -> Option<EventBatch> { slf.inner.next() }
}

#[cfg(not(target_arch = "wasm32"))]
#[pymodule]
fn libreeb(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<EventBatch>()?;
    m.add_class::<EventIterator>()?;
    m.add_class::<SliceIterator>()?;
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

    fn compute_hash<I>(batches: I) -> u64
    where
        I: Iterator<Item = EventBatch>,
    {
        let mut hasher = Xxh64::new(0);
        for batch in batches {
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
    #[case::evt3("data/gen4_evt3_hand.raw",      "references/gen4_evt3_hand.raw.no_time_shift.xxh")]
    #[case::evt21("data/claque_doigt_evt21.raw", "references/claque_doigt_evt21.raw.no_time_shift.xxh")]
    #[case::evt2("data/blinking_leds.raw",       "references/blinking_leds.raw.no_time_shift.xxh")]
    fn test_decoder(#[case] raw_path: &str, #[case] ref_path: &str) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut reader = RawFileReader::new(std::path::Path::new(raw_path))
                .expect("Failed to open input file");
            let hash = compute_hash(reader.read_events());
            let reference_content = fs::read_to_string(ref_path)
                .expect("Failed to read reference file");
            let reference: u64 = reference_content.trim().parse()
                .expect("Failed to parse reference hash");
            assert_eq!(hash, reference, "Hash mismatch for {}", raw_path);
        }
    }
}
