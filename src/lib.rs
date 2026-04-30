use evt_reader::EvtReader;
use facet::Facet;
use pyo3::prelude::*;
use std::{
    collections::HashMap,
    fs::File,
    io::{self, BufRead, BufReader},
    path::{Path, PathBuf},
};
use thiserror::Error;

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

pub use slicer::{SliceBy, SliceIter};

// Error types
#[derive(Error, Debug)]
pub enum RawFileReaderError {
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
/// with `p = 0` for a negative (falling) edge and `p = 1` for a positive
/// (rising) edge.
///
/// `EventCD` is [`Copy`] because all fields are small scalar types.
/// It is yielded by value when iterating over an [`EventBatch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventCD {
    /// Column coordinate of the pixel.
    pub x: u16,
    /// Row coordinate of the pixel.
    pub y: u16,
    /// Timestamp in microseconds.
    pub t: u64,
    /// Polarity: `0` = negative (OFF), `1` = positive (ON).
    pub p: u8,
}

/// Consuming iterator over the [`EventCD`] events in an [`EventBatch`].
///
/// Obtained by calling [`IntoIterator::into_iter`] on an owned `EventBatch`,
/// i.e. via `for evt in batch { … }`.
///
/// The batch is moved into the iterator; no allocation takes place.
/// Implements [`ExactSizeIterator`] so `.len()` is O(1).
pub struct EventBatchIter {
    batch: EventBatch,
    index: usize,
}

impl Iterator for EventBatchIter {
    type Item = EventCD;

    #[inline]
    fn next(&mut self) -> Option<EventCD> {
        let i = self.index;
        if i < self.batch.len() {
            self.index += 1;
            Some(EventCD {
                x: self.batch.x[i],
                y: self.batch.y[i],
                t: self.batch.t[i],
                p: self.batch.p[i],
            })
        } else {
            None
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.batch.len() - self.index;
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for EventBatchIter {}

impl IntoIterator for EventBatch {
    type Item = EventCD;
    type IntoIter = EventBatchIter;

    /// Consumes the batch and returns an iterator over its [`EventCD`] events.
    fn into_iter(self) -> EventBatchIter {
        EventBatchIter { batch: self, index: 0 }
    }
}

/// Borrowing iterator over the [`EventCD`] events in an [`EventBatch`].
///
/// Obtained by calling [`IntoIterator::into_iter`] on a reference to an
/// `EventBatch`, i.e. via `for evt in &batch { … }`.
///
/// Each event is copied out of the underlying SoA arrays — this is cheap
/// because [`EventCD`] is only 13 bytes.
/// Implements [`ExactSizeIterator`] so `.len()` is O(1).
pub struct EventBatchRefIter<'a> {
    batch: &'a EventBatch,
    index: usize,
}

impl<'a> Iterator for EventBatchRefIter<'a> {
    type Item = EventCD;

    #[inline]
    fn next(&mut self) -> Option<EventCD> {
        let i = self.index;
        if i < self.batch.len() {
            self.index += 1;
            Some(EventCD {
                x: self.batch.x[i],
                y: self.batch.y[i],
                t: self.batch.t[i],
                p: self.batch.p[i],
            })
        } else {
            None
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.batch.len() - self.index;
        (remaining, Some(remaining))
    }
}

impl<'a> ExactSizeIterator for EventBatchRefIter<'a> {}

impl<'a> IntoIterator for &'a EventBatch {
    type Item = EventCD;
    type IntoIter = EventBatchRefIter<'a>;

    /// Borrows the batch and returns an iterator over its [`EventCD`] events.
    fn into_iter(self) -> EventBatchRefIter<'a> {
        EventBatchRefIter { batch: self, index: 0 }
    }
}

/// Structure-of-Arrays batch of CD events.
///
/// All four vectors are guaranteed to be the same length at all times.
/// Timestamps (`t`) are non-decreasing within a batch.
#[pyclass]
#[derive(Debug, Clone, Facet)]
pub struct EventBatch {
    #[pyo3(get)]
    pub x: Vec<u16>,
    #[pyo3(get)]
    pub y: Vec<u16>,
    #[pyo3(get)]
    pub t: Vec<u64>,
    #[pyo3(get)]
    pub p: Vec<u8>,
}

impl EventBatch {
    pub fn new() -> Self {
        EventBatch {
            x: Vec::new(),
            y: Vec::new(),
            t: Vec::new(),
            p: Vec::new(),
        }
    }

    pub fn with_capacity(n: usize) -> Self {
        EventBatch {
            x: Vec::with_capacity(n),
            y: Vec::with_capacity(n),
            t: Vec::with_capacity(n),
            p: Vec::with_capacity(n),
        }
    }

    #[inline]
    pub fn push(&mut self, x: u16, y: u16, t: u64, p: u8) {
        self.x.push(x);
        self.y.push(y);
        self.t.push(t);
        self.p.push(p);
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.x.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
    }

    /// Append all events from `other`, consuming it.
    pub fn append(&mut self, mut other: EventBatch) {
        self.x.append(&mut other.x);
        self.y.append(&mut other.y);
        self.t.append(&mut other.t);
        self.p.append(&mut other.p);
    }

    /// Split at `at`: `self` keeps `[0..at]`, returns the `[at..]` tail.
    pub fn split_off(&mut self, at: usize) -> EventBatch {
        EventBatch {
            x: self.x.split_off(at),
            y: self.y.split_off(at),
            t: self.t.split_off(at),
            p: self.p.split_off(at),
        }
    }
}

impl Default for EventBatch {
    fn default() -> Self {
        Self::new()
    }
}

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

#[pyclass]
pub struct RawFileReader {
    pub header: RawFileHeader,
    path: Box<Path>,
    event_iterator: Box<dyn Iterator<Item = EventBatch> + Send + Sync>,
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
        // Look at the next char without consuming it
        let buffer = reader
            .fill_buf()
            .map_err(|_e| RawFileReaderError::ReadBytesFailed)?; // TODO: Propagate the error

        let next_char = buffer.first().ok_or(RawFileReaderError::ReadBytesFailed)?;

        if *next_char != b'%' {
            // if next char is not a % it's the end of header section
            break;
        }

        // Read the line
        let mut header_line = String::new();
        reader
            .read_line(&mut header_line)
            .map_err(|_e| RawFileReaderError::ReadBytesFailed)?; // TODO: Propagate the error

        let mut parts = header_line.trim_start_matches('%').trim().splitn(2, ' ');
        let key = parts.next().ok_or(RawFileReaderError::ParseHeaderFailed)?;
        let maybe_value = parts.next();
        if let Some(value) = maybe_value {
            match key {
                "evt" => {
                    event_type_string = Some(value.to_string());
                }
                // Format: "1280x720"
                "geometry" => {
                    if let Some((w, h)) = value.trim().split_once('x') {
                        if let (Ok(w), Ok(h)) = (w.parse(), h.parse()) {
                            cam_width = w;
                            cam_height = h;
                        }
                    }
                }
                "format" => {
                    event_format_string = Some(value.to_string());
                }
                "endianness" => {}
                _ => {}
            }
            header_dict.insert(key.to_string(), value.to_string());
        }
    }

    // Find the format
    let mut evt_format_str = match (event_format_string, event_type_string) {
        (Some(format), Some(_)) => Ok(format),
        (Some(format), None) => Ok(format),
        (None, Some(evt_type)) => Ok(evt_type),
        (None, None) => Err(RawFileReaderError::EventTypeNotFound),
    }?;

    // For some reason, some header have a different formating where the
    // format field looks like that: "EVT21;endianness=little;height=320;width=320"
    // in this case we parse that and it takes precedence over other other fields
    if evt_format_str.contains(";") {
        let parts: Vec<String> = evt_format_str.split(";").map(|x| x.to_owned()).collect();
        evt_format_str = parts
            .first()
            .ok_or(RawFileReaderError::ParseHeaderFailed)?
            .to_string();

        // Also extract geometry from the semicolon-delimited format string if not yet found
        if cam_width == 0 {
            for part in evt_format_str.split(';').skip(1) {
                if let Some(v) = part.strip_prefix("width=") {
                    cam_width = v.trim().parse().unwrap_or(0);
                } else if let Some(v) = part.strip_prefix("height=") {
                    cam_height = v.trim().parse().unwrap_or(0);
                }
            }
        }
    }

    let event_type = match evt_format_str.as_str() {
        "2.0" | "EVT2" => Ok(RawEventType::Evt2),
        "2.1" | "EVT21" => Ok(RawEventType::Evt21),
        "3.0" | "EVT3" => Ok(RawEventType::Evt3),
        "4.0" | "EVT4" => Ok(RawEventType::Evt4),
        unkown_type => Err(RawFileReaderError::UnknownEventType(
            unkown_type.to_string(),
        )),
    }?;

    let header = RawFileHeader {
        header_dict,
        event_type,
        camera_geometry: CameraGeometry {
            width: cam_width,
            height: cam_height,
        },
    };
    Ok(header)
}

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
                RawEventType::Evt21 => {
                    let decoder = Evt21Decoder::new();
                    Box::new(EvtReader::new(reader, decoder))
                }
                RawEventType::Evt3 => {
                    let decoder = Evt3Decoder::new();
                    Box::new(EvtReader::new(reader, decoder))
                }
                _ => {
                    return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                        "Unsupported event type",
                    ))
                }
            };

        Ok(EventIterator {
            inner: event_iterator,
        })
    }
}

impl RawFileReader {
    pub fn new(path: &Path) -> Result<Self, RawFileReaderError> {
        let file =
            File::open(path).map_err(|e| RawFileReaderError::FileOpenError(path.into(), e))?;

        let mut reader = BufReader::with_capacity(64 * 1024, file);

        let header = parse_header(&mut reader)?;

        let event_iterator: Box<dyn Iterator<Item = EventBatch> + Send + Sync> =
            match header.event_type {
                RawEventType::Evt2 => {
                    let decoder = Evt2Decoder::new();
                    Box::new(EvtReader::new(reader, decoder))
                }
                RawEventType::Evt21 => {
                    let decoder = Evt21Decoder::new();
                    Box::new(EvtReader::new(reader, decoder))
                }
                RawEventType::Evt3 => {
                    let decoder = Evt3Decoder::new();
                    Box::new(EvtReader::new(reader, decoder))
                }
                _ => return Err(RawFileReaderError::DecoderNotImplemented(header.event_type)),
            };

        Ok(RawFileReader {
            path: path.into(),
            event_iterator,
            header,
        })
    }

    pub fn read_events<'a>(&'a mut self) -> Box<dyn std::iter::Iterator<Item = EventBatch> + 'a> {
        Box::new(&mut self.event_iterator)
    }

    /// Resets the file reader
    pub fn reset(&mut self) {
        let decoder = Self::new(&self.path).unwrap();
        *self = decoder;
    }
}

// Python bindings
#[pyclass]
pub struct EventIterator {
    inner: Box<dyn Iterator<Item = EventBatch> + Send + Sync>,
}

#[pymethods]
impl EventIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(mut slf: PyRefMut<'_, Self>) -> Option<EventBatch> {
        slf.inner.next()
    }

    /// Consume this iterator and return a SliceIterator that yields one EventBatch per
    /// time window of `micros` microseconds.
    pub fn slice_by_time(mut slf: PyRefMut<'_, Self>, micros: u64) -> SliceIterator {
        let inner = std::mem::replace(&mut slf.inner, Box::new(std::iter::empty()));
        SliceIterator {
            inner: Box::new(SliceIter::new(inner, SliceBy::Time(micros))),
        }
    }

    /// Consume this iterator and return a SliceIterator that yields one EventBatch per
    /// `count` events.
    pub fn slice_by_count(mut slf: PyRefMut<'_, Self>, count: usize) -> SliceIterator {
        let inner = std::mem::replace(&mut slf.inner, Box::new(std::iter::empty()));
        SliceIterator {
            inner: Box::new(SliceIter::new(inner, SliceBy::Count(count))),
        }
    }

    /// Consume this iterator and return a SliceIterator that yields one EventBatch per
    /// time window of `micros` microseconds or `count` events, whichever comes first.
    pub fn slice_by_both(mut slf: PyRefMut<'_, Self>, micros: u64, count: usize) -> SliceIterator {
        let inner = std::mem::replace(&mut slf.inner, Box::new(std::iter::empty()));
        SliceIterator {
            inner: Box::new(SliceIter::new(inner, SliceBy::Both(micros, count))),
        }
    }
}

#[pyclass]
pub struct SliceIterator {
    inner: Box<dyn Iterator<Item = EventBatch> + Send + Sync>,
}

#[pymethods]
impl SliceIterator {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(mut slf: PyRefMut<'_, Self>) -> Option<EventBatch> {
        slf.inner.next()
    }
}

#[pymodule]
fn libreeb(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<EventBatch>()?;
    m.add_class::<EventIterator>()?;
    m.add_class::<SliceIterator>()?;
    m.add_class::<RawFileReader>()?;
    Ok(())
}

// Test module
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
            for evt in batch {
                hasher.write_u16(evt.x);
                hasher.write_u16(evt.y);
                hasher.write_u8(evt.p);
                hasher.write_u64(evt.t);
            }
        }
        hasher.finish()
    }

    #[rstest]
    #[case::evt3(
        "data/gen4_evt3_hand.raw",
        "references/gen4_evt3_hand.raw.no_time_shift.xxh"
    )]
    #[case::evt21(
        "data/claque_doigt_evt21.raw",
        "references/claque_doigt_evt21.raw.no_time_shift.xxh"
    )]
    #[case::evt2(
        "data/blinking_leds.raw",
        "references/blinking_leds.raw.no_time_shift.xxh"
    )]
    fn test_decoder(#[case] raw_path: &str, #[case] ref_path: &str) {
        let mut reader =
            RawFileReader::new(Path::new(raw_path)).expect("Failed to open input file");

        let hash = compute_hash(reader.read_events());

        let reference_content =
            fs::read_to_string(ref_path).expect("Failed to read reference file");

        let reference: u64 = reference_content
            .trim()
            .parse()
            .expect("Failed to parse reference hash");

        assert_eq!(hash, reference, "Hash mismatch for {}", raw_path);
    }
}
