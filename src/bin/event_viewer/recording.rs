//! Recording state for capturing raw event bytes to file.

use libreeb::io::header::RawFileHeader;

/// Recording state for capturing raw bytes to a .raw file.
pub struct RecordingState {
    pub is_recording: bool,
    pub filename: String,
    pub buffer: Vec<u8>,
    pub start_timestamp: Option<u64>,
    pub event_count: usize,
}

impl RecordingState {
    pub fn new() -> Self {
        RecordingState {
            is_recording: false,
            filename: Self::default_filename(),
            buffer: Vec::new(),
            start_timestamp: None,
            event_count: 0,
        }
    }

    /// Generate default filename with current timestamp.
    fn default_filename() -> String {
        #[cfg(target_arch = "wasm32")]
        {
            let date = js_sys::Date::new_0();
            let year = date.get_full_year() as i32;
            let month = date.get_month() + 1;
            let day = date.get_date();
            let hour = date.get_hours();
            let minute = date.get_minutes();
            let second = date.get_seconds();
            format!(
                "recording-{:04}-{:02}-{:02}_{:02}-{:02}-{:02}.raw",
                year, month, day, hour, minute, second
            )
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            "recording.raw".to_string()
        }
    }

    /// Start recording with the given header.
    pub fn start(&mut self, header: &RawFileHeader) {
        self.is_recording = true;
        self.buffer.clear();
        self.event_count = 0;
        self.start_timestamp = None;

        // Write header to buffer
        self.write_header(header);
    }

    /// Write header in .raw format.
    fn write_header(&mut self, header: &RawFileHeader) {
        // Generate current timestamp
        #[cfg(target_arch = "wasm32")]
        let date_str = {
            let date = js_sys::Date::new_0();
            date.to_iso_string().as_string().unwrap_or_default()
        };
        #[cfg(not(target_arch = "wasm32"))]
        let date_str = "2026-05-05T00:00:00.000Z".to_string();

        // Write date
        self.buffer.extend_from_slice(b"% Date ");
        self.buffer.extend_from_slice(date_str.as_bytes());
        self.buffer.push(b'\n');

        // Write event type
        let event_type_str = match header.event_type {
            libreeb::io::header::RawEventType::Evt2 => "EVT2",
            libreeb::io::header::RawEventType::Evt21 => "EVT2.1\n% endianness little",
            libreeb::io::header::RawEventType::Evt21Legacy => "EVT2.1\n% endianness legacy",
            libreeb::io::header::RawEventType::Evt3 => "EVT3",
            libreeb::io::header::RawEventType::Evt4 => "EVT4",
        };
        self.buffer.extend_from_slice(b"% evt ");
        self.buffer.extend_from_slice(event_type_str.as_bytes());
        self.buffer.push(b'\n');

        // Write geometry
        let geo = &header.camera_geometry;
        let geo_str = format!("% geometry {}x{}\n", geo.width, geo.height);
        self.buffer.extend_from_slice(geo_str.as_bytes());

        // Copy other header fields (excluding Date, evt, geometry to avoid duplicates)
        let skip_keys = ["Date", "date", "evt", "geometry", "format", "endianness"];
        for (k, v) in &header.header_dict {
            if !skip_keys.contains(&k.as_str()) {
                let line = format!("% {} {}\n", k, v);
                self.buffer.extend_from_slice(line.as_bytes());
            }
        }

        // No blank line needed - .raw format has event data immediately after last header field
    }

    /// Stop recording and return the buffer.
    pub fn stop(&mut self) -> Vec<u8> {
        self.is_recording = false;
        std::mem::take(&mut self.buffer)
    }

    /// Reset recording state.
    pub fn reset(&mut self) {
        self.is_recording = false;
        self.buffer.clear();
        self.event_count = 0;
        self.start_timestamp = None;
        self.filename = Self::default_filename();
    }

    /// Get buffer size in MB.
    pub fn buffer_size_mb(&self) -> f64 {
        self.buffer.len() as f64 / 1_000_000.0
    }
}

impl Default for RecordingState {
    fn default() -> Self {
        Self::new()
    }
}
