//! RAW file header parsing for Prophesee `.raw` files.
//!
//! The header is a sequence of `% key value` lines terminated by the first
//! line that does not start with `%`.  Two keys carry geometry and format
//! information used by the rest of the library:
//!
//! - `geometry` — `<width>x<height>` (may be absent; use `scan_geometry` then)
//! - `format` / `evt` — event encoding identifier (see [`RawEventType`])
//!
//! Some newer files encode width/height inside the `format` value separated
//! by semicolons: `EVT3;width=1280;height=720`.

use std::collections::HashMap;
use std::io::BufRead;

use crate::RawFileReaderError;

/// Supported event encoding formats.
#[derive(Debug, Clone, Copy)]
pub enum RawEventType {
    Evt2,
    Evt21,
    Evt3,
    Evt4,
}

/// Width × height of the sensor in pixels.
#[derive(Debug)]
pub struct CameraGeometry {
    pub width:  u32,
    pub height: u32,
}

/// Parsed contents of a `.raw` file header.
#[derive(Debug)]
pub struct RawFileHeader {
    /// All key/value pairs found in the header (for display purposes).
    pub header_dict:     HashMap<String, String>,
    pub event_type:      RawEventType,
    pub camera_geometry: CameraGeometry,
}

/// Read and parse the `%`-prefixed header lines from `reader`.
///
/// Stops at the first byte that is not `%` (that byte is left unconsumed).
/// Returns an error if no event format can be determined.
pub fn parse_header(reader: &mut impl BufRead) -> Result<RawFileHeader, RawFileReaderError> {
    let mut header_dict: HashMap<String, String> = HashMap::new();
    let mut event_type_string:   Option<String> = None;
    let mut event_format_string: Option<String> = None;
    let mut cam_width:  u32 = 0;
    let mut cam_height: u32 = 0;

    loop {
        let buffer = reader.fill_buf()
            .map_err(|_| RawFileReaderError::ReadBytesFailed)?;

        match buffer.first() {
            None => break,
            Some(&b) if b != b'%' => break,
            _ => {}
        }

        let mut line = String::new();
        reader.read_line(&mut line)
            .map_err(|_| RawFileReaderError::ReadBytesFailed)?;

        let mut parts = line.trim_start_matches('%').trim().splitn(2, ' ');
        let key = parts.next().ok_or(RawFileReaderError::ParseHeaderFailed)?;
        if let Some(value) = parts.next() {
            match key {
                "evt"      => event_type_string   = Some(value.to_string()),
                "format"   => event_format_string = Some(value.to_string()),
                "geometry" => {
                    if let Some((w, h)) = value.trim().split_once('x') {
                        if let (Ok(w), Ok(h)) = (w.parse(), h.parse()) {
                            cam_width  = w;
                            cam_height = h;
                        }
                    }
                }
                _ => {}
            }
            header_dict.insert(key.to_string(), value.to_string());
        }
    }

    // Prefer `format` over the older `evt` key.
    let mut fmt = match (event_format_string, event_type_string) {
        (Some(f), _) | (None, Some(f)) => Ok(f),
        (None, None)                   => Err(RawFileReaderError::EventTypeNotFound),
    }?;

    // Some files use `EVT3;width=1280;height=720` inside the format field.
    if fmt.contains(';') {
        let mut parts = fmt.splitn(2, ';');
        let base = parts.next().unwrap_or("").to_string();
        for segment in fmt.splitn(99, ';').skip(1) {
            if let Some(v) = segment.strip_prefix("width=") {
                cam_width  = v.trim().parse().unwrap_or(cam_width);
            } else if let Some(v) = segment.strip_prefix("height=") {
                cam_height = v.trim().parse().unwrap_or(cam_height);
            }
        }
        fmt = base;
    }

    let event_type = match fmt.trim().to_uppercase().as_str() {
        "EVT2"  | "2.0"        => RawEventType::Evt2,
        "EVT21" | "EVT2.1" | "2.1" => RawEventType::Evt21,
        "EVT3"  | "3.0"        => RawEventType::Evt3,
        "EVT4"  | "4.0"        => RawEventType::Evt4,
        other => return Err(RawFileReaderError::UnknownEventType(other.to_string())),
    };

    Ok(RawFileHeader {
        header_dict,
        event_type,
        camera_geometry: CameraGeometry { width: cam_width, height: cam_height },
    })
}
