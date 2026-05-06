//! Terminal-based event viewer using ratatui.
//!
//! Usage:
//!   tui_viewer <file.raw> [--slice-duration <us>] [--filter <on|off|both>]
//!
//! Controls:
//!   q/ESC - Quit
//!   Space - Pause/Resume
//!   f - Toggle fullscreen stats

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use libreeb::{
    header::CameraGeometry, 
    processors::FrameGenerator, 
    RawFileReader
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{canvas::{Canvas, Points}, Block, Borders, Paragraph, Wrap},
    Frame, Terminal,
};
use std::{
    error::Error,
    io,
    path::Path,
    time::{Duration, Instant},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventFilter {
    Both,
    OnOnly,
    OffOnly,
}

impl EventFilter {
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "both" => Ok(EventFilter::Both),
            "on" => Ok(EventFilter::OnOnly),
            "off" => Ok(EventFilter::OffOnly),
            _ => Err(format!("Invalid filter: {}. Use 'on', 'off', or 'both'", s)),
        }
    }

    fn as_str(&self) -> &str {
        match self {
            EventFilter::Both => "Both",
            EventFilter::OnOnly => "ON only",
            EventFilter::OffOnly => "OFF only",
        }
    }
}

struct AppState {
    reader: RawFileReader,
    generator: FrameGenerator,
    slice_duration: u64,
    filter: EventFilter,
    filename: String,
    
    // Slicing state
    overflow: libreeb::EventBatch,
    
    // State
    frame_count: u64,
    last_event_count: usize,
    last_rate: f64,
    first_timestamp: Option<u64>,
    last_timestamp: Option<u64>,
    paused: bool,
    current_image: Option<image::RgbImage>,
    
    // Timing
    start_time: Instant,
    last_frame_time: Instant,
}

impl AppState {
    fn new(file_path: &Path, slice_duration: u64, filter: EventFilter) -> Result<Self, Box<dyn Error>> {
        let mut reader = RawFileReader::new(file_path)?;
        
        // Scan geometry if needed
        if reader.needs_geometry_scan() {
            reader.scan_geometry(8);
        }
        
        let geometry = CameraGeometry {
            width: reader.header.camera_geometry.width.max(1),
            height: reader.header.camera_geometry.height.max(1),
        };
        
        let generator = FrameGenerator::new(geometry);
        let filename = file_path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        
        Ok(AppState {
            reader,
            generator,
            slice_duration,
            filter,
            filename,
            overflow: libreeb::EventBatch::new(),
            frame_count: 0,
            last_event_count: 0,
            last_rate: 0.0,
            first_timestamp: None,
            last_timestamp: None,
            paused: false,
            current_image: None,
            start_time: Instant::now(),
            last_frame_time: Instant::now(),
        })
    }
    
    fn process_next_frame(&mut self) -> Result<bool, Box<dyn Error>> {
        if self.paused {
            return Ok(true);
        }
        
        // Implement time-based slicing (same algorithm as SliceIter::Time)
        let mut acc = std::mem::take(&mut self.overflow);
        let mut need_reset = false;
        
        // Ensure we have at least one event to anchor the window
        while acc.is_empty() {
            match self.reader.read_events().next() {
                Some(batch) => acc = batch,
                None => {
                    // End of file - need to loop back to start
                    need_reset = true;
                    break;
                }
            }
        }
        
        if need_reset {
            self.reader.reset();
            self.frame_count = 0;
            self.first_timestamp = None;
            self.last_timestamp = None;
            self.overflow = libreeb::EventBatch::new();
            return Ok(true); // Continue from start
        }
        
        let end_t = acc.t[0] + self.slice_duration;
        
        loop {
            // Binary search: timestamps are non-decreasing
            let split = acc.t.partition_point(|&t| t < end_t);
            if split < acc.len() {
                self.overflow = acc.split_off(split);
                break;
            }
            match self.reader.read_events().next() {
                Some(batch) => acc.append(batch),
                None => {
                    // End of file with remaining events
                    break;
                }
            }
        }
        
        if acc.is_empty() {
            return Ok(false);
        }
        
        // Update statistics
        self.frame_count += 1;
        self.last_event_count = acc.len();
        
        if acc.t.len() >= 2 {
            let dt_us = acc.t.last().unwrap().saturating_sub(acc.t[0]);
            if dt_us > 0 {
                self.last_rate = acc.len() as f64 / (dt_us as f64 * 1e-6);
            }
        }
        
        if self.first_timestamp.is_none() {
            self.first_timestamp = acc.t.first().copied();
        }
        self.last_timestamp = acc.t.last().copied();
        
        // Generate frame
        let img = self.generator.generate(&acc);
        self.current_image = Some(img);
        
        self.last_frame_time = Instant::now();
        
        Ok(true)
    }
    
    fn adjust_slice_duration(&mut self, delta: i64) {
        let new_duration = (self.slice_duration as i64 + delta).max(1).min(200_000) as u64;
        self.slice_duration = new_duration;
        // When slice duration changes, we let the next frame use the new duration naturally
    }
    
    fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }
    
    fn fps(&self) -> f64 {
        if self.frame_count == 0 {
            return 0.0;
        }
        self.frame_count as f64 / self.elapsed().as_secs_f64()
    }
}

fn format_timestamp(us: u64) -> String {
    let total_ms = us / 1000;
    let ms = total_ms % 1000;
    let total_s = total_ms / 1000;
    let s = total_s % 60;
    let total_m = total_s / 60;
    let m = total_m % 60;
    let h = total_m / 60;
    format!("{:02}:{:02}:{:02}.{:03}", h, m, s, ms)
}

fn format_rate(rate: f64) -> String {
    if rate >= 1_000_000.0 {
        format!("{:.2} Mev/s", rate / 1_000_000.0)
    } else if rate >= 1_000.0 {
        format!("{:.2} kev/s", rate / 1_000.0)
    } else {
        format!("{:.0} ev/s", rate)
    }
}

fn ui(f: &mut Frame, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(35)])
        .split(f.area());
    
    // Canvas area
    render_canvas(f, chunks[0], state);
    
    // Sidebar
    render_sidebar(f, chunks[1], state);
}

fn render_canvas(f: &mut Frame, area: Rect, state: &AppState) {
    let width = state.reader.header.camera_geometry.width as f64;
    let height = state.reader.header.camera_geometry.height as f64;
    
    // Calculate available canvas space (account for borders)
    let canvas_width = (area.width.saturating_sub(2)) as f64;
    let canvas_height = (area.height.saturating_sub(2)) as f64 * 2.0; // Account for half-blocks
    
    // Calculate scale to fit while preserving aspect ratio
    let scale_x = canvas_width / width;
    let scale_y = canvas_height / height;
    let scale = scale_x.min(scale_y);
    
    // Calculate actual display dimensions
    let display_width = width * scale;
    let display_height = height * scale;
    
    // Set canvas bounds to match display size exactly (no margins!)
    let x_bound = display_width / 2.0;
    let y_bound = display_height / 2.0;
    
    let canvas = Canvas::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Event Viewer - Frame #{} ", state.frame_count))
                .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        )
        .paint(|ctx| {
            // Get current frame image
            let img = match &state.current_image {
                Some(img) => img,
                None => return, // No frame yet
            };
            
            // Draw events as points
            for y in 0..(height as usize) {
                for x in 0..(width as usize) {
                    let pixel = img.get_pixel(x as u32, y as u32);
                    
                    // FrameGenerator colors:
                    // BACKGROUND: [30, 30, 46]
                    // POSITIVE (p=1): [245, 194, 231] - pinkish
                    // NEGATIVE (p=0): [137, 220, 235] - cyan
                    
                    // Skip background
                    if pixel[0] < 50 && pixel[1] < 50 && pixel[2] < 50 {
                        continue;
                    }
                    
                    // Distinguish ON vs OFF by green channel
                    // POSITIVE has G=194, NEGATIVE has G=220
                    let is_on = pixel[1] < 200; // ON events have lower green
                    let is_off = pixel[1] > 200; // OFF events have higher green
                    
                    let show = match state.filter {
                        EventFilter::Both => true,
                        EventFilter::OnOnly => is_on,
                        EventFilter::OffOnly => is_off,
                    };
                    
                    if !show {
                        continue;
                    }
                    
                    // Scale coordinates to fit display, centered at origin
                    let sx = (x as f64 * scale) - x_bound;
                    let sy = y_bound - (y as f64 * scale);
                    
                    // Use very distinct colors
                    let color = if is_on {
                        Color::LightRed  // Bright red for ON events
                    } else {
                        Color::LightCyan  // Bright cyan for OFF events
                    };
                    
                    // Draw point
                    ctx.draw(&Points {
                        coords: &[(sx, sy)],
                        color,
                    });
                }
            }
        })
        .x_bounds([-x_bound, x_bound])
        .y_bounds([-y_bound, y_bound])
        .marker(ratatui::symbols::Marker::Braille);
    
    f.render_widget(canvas, area);
}

fn render_sidebar(f: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(13), // File info (increased from 11 to 13)
            Constraint::Length(10), // Stats
            Constraint::Min(5),     // Header
            Constraint::Length(4),  // Controls (increased from 3 to 4)
        ])
        .split(area);
    
    // File info
    let file_info = vec![
        Line::from(vec![
            Span::styled("File Info", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("File:    "),
            Span::styled(
                &state.filename,
                Style::default().fg(Color::Cyan)
            ),
        ]),
        Line::from(vec![
            Span::raw("Format:  "),
            Span::styled(
                format!("{:?}", state.reader.header.event_type),
                Style::default().fg(Color::Green)
            ),
        ]),
        Line::from(vec![
            Span::raw("Size:    "),
            Span::styled(
                format!("{}×{}", 
                    state.reader.header.camera_geometry.width,
                    state.reader.header.camera_geometry.height
                ),
                Style::default().fg(Color::Green)
            ),
        ]),
        Line::from(vec![
            Span::raw("Filter:  "),
            Span::styled(state.filter.as_str(), Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::raw("Slice:   "),
            Span::styled(format!("{} µs", state.slice_duration), Style::default().fg(Color::Green)),
        ]),
    ];
    
    let file_block = Paragraph::new(file_info)
        .block(Block::default().borders(Borders::ALL).title(" File "));
    f.render_widget(file_block, chunks[0]);
    
    // Stats
    let status = if state.paused { "PAUSED" } else { "Running" };
    let status_color = if state.paused { Color::Yellow } else { Color::Green };
    
    let stats = vec![
        Line::from(vec![
            Span::styled("Stats", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("Status:  "),
            Span::styled(status, Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::raw("Frame:   "),
            Span::styled(format!("#{}", state.frame_count), Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::raw("Rate:    "),
            Span::styled(format_rate(state.last_rate), Style::default().fg(Color::Green)),
        ]),
        Line::from(vec![
            Span::raw("FPS:     "),
            Span::styled(format!("{:.1}", state.fps()), Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::raw("Time:    "),
            Span::styled(
                state.last_timestamp.map(format_timestamp).unwrap_or_else(|| "N/A".to_string()),
                Style::default().fg(Color::Green)
            ),
        ]),
        Line::from(vec![
            Span::raw("Elapsed: "),
            Span::styled(
                if let (Some(first), Some(last)) = (state.first_timestamp, state.last_timestamp) {
                    format_timestamp(last.saturating_sub(first))
                } else {
                    "N/A".to_string()
                },
                Style::default().fg(Color::Green)
            ),
        ]),
    ];
    
    let stats_block = Paragraph::new(stats)
        .block(Block::default().borders(Borders::ALL).title(" Stats "));
    f.render_widget(stats_block, chunks[1]);
    
    // Header
    let header_lines: Vec<Line> = state.reader.header.header_dict
        .iter()
        .take(10)
        .map(|(k, v)| {
            Line::from(vec![
                Span::styled(format!("{}: ", k), Style::default().fg(Color::DarkGray)),
                Span::raw(v.chars().take(20).collect::<String>()),
            ])
        })
        .collect();
    
    let header_block = Paragraph::new(header_lines)
        .block(Block::default().borders(Borders::ALL).title(" Header "))
        .wrap(Wrap { trim: true });
    f.render_widget(header_block, chunks[2]);
    
    // Controls
    let controls = vec![
        Line::from("q/ESC: Quit  Space: Pause"),
        Line::from("←/→: Adjust slice duration"),
    ];
    
    let controls_block = Paragraph::new(controls)
        .block(Block::default().borders(Borders::ALL).title(" Controls "))
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(controls_block, chunks[3]);
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    mut state: AppState,
) -> Result<(), Box<dyn Error>> {
    let target_fps = 30.0;
    let frame_duration = Duration::from_secs_f64(1.0 / target_fps);
    
    loop {
        terminal.draw(|f| ui(f, &state))?;
        
        // Check for key events
        if event::poll(Duration::from_millis(16))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        return Ok(());
                    }
                    KeyCode::Char(' ') => {
                        state.paused = !state.paused;
                    }
                    KeyCode::Left => {
                        // Decrease slice duration
                        state.adjust_slice_duration(-5000);
                    }
                    KeyCode::Right => {
                        // Increase slice duration
                        state.adjust_slice_duration(5000);
                    }
                    _ => {}
                }
            }
        }
        
        // Process next frame
        if !state.paused {
            state.process_next_frame()?;
        }
        
        // Frame rate limiting
        std::thread::sleep(frame_duration);
    }
}

fn parse_args() -> Result<(String, u64, EventFilter), String> {
    let args: Vec<String> = std::env::args().collect();
    
    if args.len() < 2 {
        return Err(format!(
            "Usage: {} <file.raw> [--slice-duration <us>] [--filter <on|off|both>]\n\
             \n\
             Arguments:\n\
             - file.raw: Path to the .raw event file\n\
             - --slice-duration: Duration of each frame slice in microseconds (default: 33333)\n\
             - --filter: Event filter - 'on', 'off', or 'both' (default: both)",
            args[0]
        ));
    }
    
    let filename = args[1].clone();
    let mut slice_duration = 33_333u64;
    let mut filter = EventFilter::Both;
    
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--slice-duration" | "-s" => {
                if i + 1 >= args.len() {
                    return Err("--slice-duration requires a value".to_string());
                }
                slice_duration = args[i + 1].parse()
                    .map_err(|e| format!("Invalid slice duration: {}", e))?;
                i += 2;
            }
            "--filter" | "-f" => {
                if i + 1 >= args.len() {
                    return Err("--filter requires a value".to_string());
                }
                filter = EventFilter::from_str(&args[i + 1])?;
                i += 2;
            }
            _ => {
                return Err(format!("Unknown argument: {}", args[i]));
            }
        }
    }
    
    Ok((filename, slice_duration, filter))
}

fn main() -> Result<(), Box<dyn Error>> {
    // Parse arguments
    let (filename, slice_duration, filter) = match parse_args() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };
    
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    
    // Create app state
    let state = AppState::new(Path::new(&filename), slice_duration, filter)?;
    
    // Run app
    let res = run_app(&mut terminal, state);
    
    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    
    if let Err(err) = res {
        eprintln!("Error: {:?}", err);
    }
    
    Ok(())
}
