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

pub fn parse_header(reader: &mut impl BufRead) -> Result<RawFileHeader, RawFileReaderError> {
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
