#![feature(impl_trait_in_assoc_type)]

use std::{
    collections::HashMap,
    fs::File,
    io::{self, BufRead, BufReader, Read},
    path::{Path, PathBuf},
};
use thiserror::Error;

// Re-export decoders as public
pub use evt2_1::*;
pub use evt3::*;

pub mod evt2_1;
pub mod evt3;
mod macros;

enum Decoder {
    Evt21(Evt21Decoder),
    Evt3(Evt3Decoder),
}

impl EventDecoder for Decoder {
    fn decode(&mut self, reader: &mut impl Read) -> impl Iterator<Item = EventCD> {
        std::iter::from_fn(move || match self {
            Decoder::Evt21(decoder) => decoder.decode(reader).next(),
            Decoder::Evt3(decoder) => decoder.decode(reader).next(),
        })
    }
}

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

    #[error("Wasn't able to find the event type in file header")]
    EventTypeNotFound,

    #[error("An unknown error occurred")]
    Unknown,
}

#[derive(Debug, PartialEq, Eq, Default)]
pub struct EventCD {
    pub x: u16,
    pub y: u16,
    pub p: u8,
    pub t: u64,
}

pub trait EventDecoder {
    fn decode(&mut self, reader: &mut impl Read) -> impl Iterator<Item = EventCD>;
}

pub struct RawFileReader {
    path: Box<Path>,
    reader: BufReader<File>,
    decoder: Decoder,
    header: RawFileHeader,
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

// Implement the parse_header function
fn parse_header(reader: &mut impl BufRead) -> Result<RawFileHeader, RawFileReaderError> {
    let mut header_dict: HashMap<String, String> = HashMap::new();
    let mut event_type_string = None;
    let mut event_format_string = None;

    loop {
        // Look at the next char without consuming it
        let buffer = reader
            .fill_buf()
            .map_err(|e| RawFileReaderError::ReadBytesFailed)?;

        let next_char = buffer.first().ok_or(RawFileReaderError::ReadBytesFailed)?;

        if *next_char != b'%' {
            // if next char is not a % it's the end of header section
            break;
        }

        // Read the line
        let mut header_line = String::new();
        reader
            .read_line(&mut header_line)
            .map_err(|e| RawFileReaderError::ReadBytesFailed)?;

        let mut parts = header_line.trim_start_matches('%').trim().splitn(2, ' ');
        let key = parts.next().ok_or(RawFileReaderError::ParseHeaderFailed)?;
        let maybe_value = parts.next();
        if let Some(value) = maybe_value {
            match key {
                "evt" => {
                    event_type_string = Some(value.to_string());
                }
                "geometry" => {}
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
    let evt_format_str = match (event_format_string, event_type_string) {
        (Some(format), Some(_)) => {
            Ok(format) // TODO Handle the ugly ;;
        }
        (Some(format), None) => {
            Ok(format) // TODO Handle the ugly ;;
        }
        (None, Some(evt_type)) => Ok(evt_type),
        (None, None) => Err(RawFileReaderError::EventTypeNotFound),
    }?;

    let event_type = match evt_format_str.as_str() {
        "2.0" | "EVT2" => Ok(RawEventType::Evt21),
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
            width: 0,
            height: 0,
        },
    };
    Ok(header)
}

impl RawFileReader {
    pub fn new(path: &Path) -> Result<Self, RawFileReaderError> {
        let file =
            File::open(path).map_err(|e| RawFileReaderError::FileOpenError(path.into(), e))?;

        let mut reader = BufReader::with_capacity(32 * 1024, file);

        let header = parse_header(&mut reader)?;

        let decoder = match header.event_type {
            RawEventType::Evt2 => Err(RawFileReaderError::DecoderNotImplemented(header.event_type)),
            RawEventType::Evt21 => Ok(Decoder::Evt21(Evt21Decoder::default())),
            RawEventType::Evt3 => Ok(Decoder::Evt3(Evt3Decoder::default())),
            RawEventType::Evt4 => Err(RawFileReaderError::DecoderNotImplemented(header.event_type)),
        }?;

        Ok(RawFileReader {
            path: path.into(),
            reader,
            decoder,
            header,
        })
    }

    pub fn read_events(&mut self) -> impl Iterator<Item = EventCD> + use<'_> {
        self.decoder.decode(&mut self.reader)
    }

    /// Resets the file reader
    pub fn reset(&mut self) {
        let file = File::open(&self.path).expect("Cannot read file");
        self.reader = BufReader::with_capacity(32 * 1024, file);
        // self.decoder = Evt21Decoder::default();
    }
}

/// Slice configuration options
pub enum SliceBy {
    /// Slice by time in microseconds
    Time(u64),
    /// Slice by count of events
    Count(usize),
    /// Slice by whichever comes first: time or count
    Both(u64, usize),
}

pub fn slice_events<I>(events: I, slice_by: SliceBy) -> impl Iterator<Item = Vec<EventCD>>
where
    I: Iterator<Item = EventCD>,
{
    let mut iter = events.peekable();

    // Estimate capacity based on slice configuration
    let estimated_capacity = match &slice_by {
        SliceBy::Count(count) => *count,
        SliceBy::Time(_) => 100_000, // Assuming up to 100,000 events per millisecond
        SliceBy::Both(_, count) => *count,
    };

    std::iter::from_fn(move || {
        let first = iter.next()?;

        let (slice_end_time, max_count) = match slice_by {
            SliceBy::Time(micros) => (Some(first.t + micros), None),
            SliceBy::Count(count) => (None, Some(count)),
            SliceBy::Both(micros, count) => (Some(first.t + micros), Some(count)),
        };

        // Pre-allocate with estimated capacity
        let mut slice = Vec::with_capacity(estimated_capacity);
        slice.push(first);

        // If we're slicing by count only
        if slice_end_time.is_none() {
            let count = max_count.unwrap();
            slice.extend(iter.by_ref().take(count - 1));
            return Some(slice);
        }

        // If we're slicing by time or both
        let end_time = slice_end_time.unwrap();
        if let Some(count) = max_count {
            // Both time and count
            slice.extend(
                iter.by_ref()
                    .take_while(|event| event.t < end_time)
                    .take(count - 1),
            );
        } else {
            // Time only
            slice.extend(iter.by_ref().take_while(|event| event.t < end_time));
        }

        Some(slice)
    })
}
