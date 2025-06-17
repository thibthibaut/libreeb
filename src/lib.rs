use evt_reader::EvtReader;
use std::{
    collections::{HashMap, VecDeque},
    fs::File,
    io::{self, BufRead, BufReader, Read},
    path::{Path, PathBuf},
};
use thiserror::Error;

// Re-export decoders as public
// pub use evt2::*;
pub use evt2_1::*;
// pub use evt3::*;

// pub mod evt2;
pub mod evt2_1;
// pub mod evt3;
mod evt_reader;
mod macros;

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

#[derive(Default, Debug, PartialEq, Eq, Clone, Copy)]
pub enum Event {
    CD {
        x: u16,
        y: u16,
        p: u8,
        t: u64,
    },
    ExternalTrigger {
        id: u8,
        p: u8,
    },
    #[default]
    Unknown,
}

impl Event {
    pub fn timestamp(&self) -> Option<u64> {
        if let Event::CD { t, .. } = self {
            Some(*t)
        } else {
            None
        }
    }

    pub fn polarity(&self) -> Option<u8> {
        match self {
            Event::CD { p, .. } => Some(*p),
            Event::ExternalTrigger { p, .. } => Some(*p),
            _ => None,
        }
    }

    pub fn x(&self) -> Option<u16> {
        if let Event::CD { x, .. } = self {
            Some(*x)
        } else {
            None
        }
    }

    pub fn y(&self) -> Option<u16> {
        if let Event::CD { y, .. } = self {
            Some(*y)
        } else {
            None
        }
    }
}

trait EventDecoder {
    type RawEventType: zerocopy::FromBytes + zerocopy::Immutable + zerocopy::KnownLayout + Copy;
    fn new() -> Self;
    fn decode(&mut self, raw_event: &[Self::RawEventType], event_queue: &mut VecDeque<Event>);
}

pub struct RawFileReader {
    pub header: RawFileHeader,
    path: Box<Path>,
    event_iterator: Box<dyn Iterator<Item = Event>>,
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
        // TODO: deal with other parts of this ;-separated header
    }

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

        let mut reader = BufReader::with_capacity(64 * 1024, file);

        let header = parse_header(&mut reader)?;

        let decoder = Evt21Decoder::new();
        let event_iterator = EvtReader::new(reader, decoder);
        let event_iterator = Box::new(event_iterator);

        Ok(RawFileReader {
            path: path.into(),
            event_iterator,
            header,
        })
    }

    // TODO: rename this function
    pub fn read_events<'a>(&'a mut self) -> Box<dyn std::iter::Iterator<Item = Event> + 'a> {
        Box::new(&mut self.event_iterator)
    }

    /// Resets the file reader
    pub fn reset(&mut self) {
        let decoder = Self::new(&self.path).unwrap();
        *self = decoder;
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

pub fn slice_events<I>(events: I, slice_by: SliceBy) -> impl Iterator<Item = Vec<Event>>
where
    I: Iterator<Item = Event>,
{
    let mut iter = events.peekable();

    // Estimate capacity
    let estimated_capacity = match &slice_by {
        SliceBy::Count(count) => *count,
        SliceBy::Time(_) => 100_000,
        SliceBy::Both(_, count) => *count,
    };

    std::iter::from_fn(move || {
        // Find the first event that has a timestamp
        let first = loop {
            match iter.next() {
                Some(e) if e.timestamp().is_some() => break e,
                Some(_) => continue,
                None => return None,
            }
        };

        let first_ts = first.timestamp().unwrap();

        let (slice_end_time, max_count) = match slice_by {
            SliceBy::Time(micros) => (Some(first_ts + micros), None),
            SliceBy::Count(count) => (None, Some(count)),
            SliceBy::Both(micros, count) => (Some(first_ts + micros), Some(count)),
        };

        let mut slice = Vec::with_capacity(estimated_capacity);
        slice.push(first);

        if slice_end_time.is_none() {
            let count = max_count.unwrap();
            slice.extend(iter.by_ref().take(count - 1));
            return Some(slice);
        }

        let end_time = slice_end_time.unwrap();
        if let Some(count) = max_count {
            slice.extend(
                iter.by_ref()
                    .filter(|e| e.timestamp().is_some())
                    .take_while(|e| e.timestamp().unwrap() < end_time)
                    .take(count - 1),
            );
        } else {
            slice.extend(
                iter.by_ref()
                    .filter(|e| e.timestamp().is_some())
                    .take_while(|e| e.timestamp().unwrap() < end_time),
            );
        }

        Some(slice)
    })
}

// Test module
#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::Hasher;
    use xxhash_rust::xxh64::Xxh64;
    fn compute_hash<I>(events: I) -> u64
    where
        I: Iterator<Item = Event>,
    {
        let mut hasher = Xxh64::new(0);
        for e in events {
            if let Event::CD { x, y, p, t } = e {
                // println!("{},{},{},{}", x, y, p, t);
                hasher.write_u16(x);
                hasher.write_u16(y);
                hasher.write_u8(p);
                hasher.write_u64(t);
            }
        }
        hasher.finish()
    }

    #[test]
    #[ignore]
    fn test_evt3_decoder() {
        let path = Path::new("data/openeb/gen4_evt3_hand.raw");
        let mut reader = RawFileReader::new(Path::new(&path)).expect("Failed to open test file");
        let event_iterator = reader.read_events();
        let hash = compute_hash(event_iterator);
        assert_eq!(hash, 0xeb46994708e41cb9);
    }

    #[test]
    fn test_evt21_decoder() {
        let path = Path::new("data/openeb/claque_doigt_evt21.raw");
        let mut reader = RawFileReader::new(Path::new(&path)).expect("Failed to open test file");
        let event_iterator = reader.read_events();
        let hash = compute_hash(event_iterator);
        assert_eq!(hash, 0x1bf31f5b25480a8a);
    }

    #[test]
    #[ignore]
    fn test_evt2_decoder() {
        let path = Path::new("data/openeb/gen4_evt2_hand.raw");
        let mut reader = RawFileReader::new(Path::new(&path)).expect("Failed to open test file");
        let event_iterator = reader.read_events();
        let hash = compute_hash(event_iterator);
        assert_eq!(hash, 0xbd1b3ff8ddb1c91b);
    }
}
