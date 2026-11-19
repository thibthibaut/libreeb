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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawEventType {
    Evt2,
    Evt21,
    /// EVT2.1 with 32-bit halves swapped (`endianness=legacy`, IMX636/EVK4).
    Evt21Legacy,
    Evt3,
    Evt4,
}

/// Width × height of the sensor in pixels.
#[derive(Debug, Clone, Copy)]
pub struct CameraGeometry {
    pub width: u32,
    pub height: u32,
}

/// Parsed contents of a `.raw` file header.
#[derive(Debug)]
pub struct RawFileHeader {
    /// All key/value pairs found in the header (for display purposes).
    pub header_dict: HashMap<String, String>,
    pub event_type: RawEventType,
    pub camera_geometry: CameraGeometry,
}

/// Read and parse the `%`-prefixed header lines from `reader`.
///
/// Stops at the first byte that is not `%` (that byte is left unconsumed).
/// Returns an error if no event format can be determined.
pub fn parse_header(reader: &mut impl BufRead) -> Result<RawFileHeader, RawFileReaderError> {
    let mut header_dict: HashMap<String, String> = HashMap::new();
    let mut event_type_string: Option<String> = None;
    let mut event_format_string: Option<String> = None;
    let mut cam_width: u32 = 0;
    let mut cam_height: u32 = 0;
    let mut endianness: Option<String> = None;

    loop {
        let buffer = reader
            .fill_buf()
            .map_err(|_| RawFileReaderError::ReadBytesFailed)?;

        match buffer.first() {
            None => break,
            Some(&b) if b != b'%' => break,
            _ => {}
        }

        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|_| RawFileReaderError::ReadBytesFailed)?;

        let mut parts = line.trim_start_matches('%').trim().splitn(2, ' ');
        let key = parts.next().ok_or(RawFileReaderError::ParseHeaderFailed)?;
        if let Some(value) = parts.next() {
            match key {
                "evt" => event_type_string = Some(value.to_string()),
                "format" => event_format_string = Some(value.to_string()),
                "endianness" => endianness = Some(value.trim().to_string()),
                "geometry" => {
                    if let Some((w, h)) = value.trim().split_once('x') {
                        if let (Ok(w), Ok(h)) = (w.parse(), h.parse()) {
                            cam_width = w;
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
        (None, None) => Err(RawFileReaderError::EventTypeNotFound),
    }?;

    // Some files use `EVT3;width=1280;height=720` inside the format field.
    if fmt.contains(';') {
        let mut parts = fmt.splitn(2, ';');
        let base = parts.next().unwrap_or("").to_string();
        for segment in fmt.splitn(99, ';').skip(1) {
            if let Some(v) = segment.strip_prefix("width=") {
                cam_width = v.trim().parse().unwrap_or(cam_width);
            } else if let Some(v) = segment.strip_prefix("height=") {
                cam_height = v.trim().parse().unwrap_or(cam_height);
            } else if let Some(v) = segment.strip_prefix("endianness=") {
                // As in OpenEB, the format field wins over the `endianness` key.
                endianness = Some(v.trim().to_string());
            }
        }
        fmt = base;
    }

    let event_type = match fmt.trim().to_uppercase().as_str() {
        "EVT2" | "2.0" => RawEventType::Evt2,
        "EVT21" | "EVT2.1" | "2.1" if endianness.as_deref() == Some("legacy") => {
            RawEventType::Evt21Legacy
        }
        "EVT21" | "EVT2.1" | "2.1" => RawEventType::Evt21,
        "EVT3" | "3.0" => RawEventType::Evt3,
        "EVT4" | "4.0" => RawEventType::Evt4,
        other => return Err(RawFileReaderError::UnknownEventType(other.to_string())),
    };

    Ok(RawFileHeader {
        header_dict,
        event_type,
        camera_geometry: CameraGeometry {
            width: cam_width,
            height: cam_height,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parse_evt3_with_geometry_field() {
        let header_text = "\
% Date 2024-01-01 12:00:00
% geometry 1280x720
% format EVT3
% evt 3.0
";
        let mut cursor = Cursor::new(header_text.as_bytes());
        let header = parse_header(&mut cursor).unwrap();

        assert_eq!(header.camera_geometry.width, 1280);
        assert_eq!(header.camera_geometry.height, 720);
        match header.event_type {
            RawEventType::Evt3 => {}
            _ => panic!("Expected EVT3"),
        }
    }

    #[test]
    fn parse_evt3_with_format_dimensions() {
        let header_text = "\
% Date 2024-01-01 12:00:00
% format EVT3;width=640;height=480
% evt 3.0
";
        let mut cursor = Cursor::new(header_text.as_bytes());
        let header = parse_header(&mut cursor).unwrap();

        assert_eq!(header.camera_geometry.width, 640);
        assert_eq!(header.camera_geometry.height, 480);
    }

    #[test]
    fn parse_evt21_header() {
        let header_text = "\
% Date 2023-12-01 10:30:45
% geometry 320x320
% evt 2.1
";
        let mut cursor = Cursor::new(header_text.as_bytes());
        let header = parse_header(&mut cursor).unwrap();

        match header.event_type {
            RawEventType::Evt21 => {}
            _ => panic!("Expected EVT2.1"),
        }
        assert_eq!(header.camera_geometry.width, 320);
        assert_eq!(header.camera_geometry.height, 320);
    }

    #[test]
    fn parse_evt21_endianness() {
        let parse = |text: &str| {
            parse_header(&mut Cursor::new(text.as_bytes()))
                .unwrap()
                .event_type
        };
        assert_eq!(
            parse("% format EVT21;height=720;width=1280;endianness=legacy\n"),
            RawEventType::Evt21Legacy
        );
        assert_eq!(
            parse("% evt 2.1\n% endianness legacy\n"),
            RawEventType::Evt21Legacy
        );
        assert_eq!(
            parse("% format EVT21\n% endianness little\n"),
            RawEventType::Evt21
        );
    }

    #[test]
    fn parse_integrator_name() {
        let header_text = "\
% Date 2024-01-01 12:00:00
% integrator_name Prophesee
% geometry 1280x720
% format EVT3
";
        let mut cursor = Cursor::new(header_text.as_bytes());
        let header = parse_header(&mut cursor).unwrap();

        assert_eq!(
            header.header_dict.get("integrator_name"),
            Some(&"Prophesee".to_string())
        );
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn parse_real_evt3_file() {
        use crate::RawFileReader;
        use std::path::Path;
        let reader = RawFileReader::new(Path::new("data/gen4_evt3_hand.raw")).unwrap();
        assert_eq!(reader.header.camera_geometry.width, 1280);
        assert_eq!(reader.header.camera_geometry.height, 720);
        match reader.header.event_type {
            RawEventType::Evt3 => {}
            _ => panic!("Expected EVT3"),
        }
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn parse_real_evt21_file() {
        use crate::RawFileReader;
        use std::path::Path;
        let reader = RawFileReader::new(Path::new("data/claque_doigt_evt21.raw")).unwrap();
        assert_eq!(reader.header.camera_geometry.width, 320);
        assert_eq!(reader.header.camera_geometry.height, 320);
        match reader.header.event_type {
            RawEventType::Evt21 => {}
            _ => panic!("Expected EVT2.1"),
        }
    }
}
