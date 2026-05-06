# TUI Event Viewer

A simple terminal-based event viewer for Prophesee `.raw` files using ratatui.

## Features

- **Terminal UI** - Runs in your terminal using ratatui and crossterm
- **Event Visualization** - Display events using canvas widget with proper aspect ratio
- **Event Filtering** - View ON events, OFF events, or both
- **Distinct Colors** - Bright red for ON events, bright cyan for OFF events
- **Dynamic Slicing** - Adjust slice duration in real-time with arrow keys
- **Real-time Stats** - Shows frame count, event rate, FPS, timestamps
- **File Info** - Displays file header information
- **Interactive Controls** - Pause/resume playback, adjust settings with keyboard
- **Auto-resize** - Maintains aspect ratio when terminal is resized

## Building

### Regular Build

```bash
cargo build --release --bin tui_viewer --no-default-features
```

The binary will be in `target/release/tui_viewer` (~829KB unstripped).

**Note:** The `--no-default-features` flag is required to exclude USB camera and Python bindings, which are not needed for the TUI viewer.

### Static Build with musl (Linux only)

For a fully static binary that can be easily shared:

```bash
# Add musl target
rustup target add x86_64-unknown-linux-musl

# Build static binary (no camera or python dependencies)
cargo build --release --target x86_64-unknown-linux-musl --bin tui_viewer --no-default-features

# Strip debug symbols (optional)
strip target/x86_64-unknown-linux-musl/release/tui_viewer
```

The static binary will be in `target/x86_64-unknown-linux-musl/release/tui_viewer`.

**Benefits of static build:**
- No external dependencies (libusb, python, etc.)
- Can be copied to any Linux system without installation
- Self-contained executable

## Usage

```bash
tui_viewer <file.raw> [OPTIONS]

OPTIONS:
    --slice-duration <us>    Frame slice duration in microseconds (default: 33333)
    --filter <mode>          Event filter: 'on', 'off', or 'both' (default: both)

EXAMPLES:
    # View all events with default settings
    tui_viewer recording.raw

    # View only ON events with 50ms slices
    tui_viewer recording.raw --slice-duration 50000 --filter on

    # View only OFF events
    tui_viewer recording.raw --filter off
```

## Controls

| Key         | Action                          |
|-------------|---------------------------------|
| `q` / `ESC` | Quit                            |
| `Space`     | Pause/Resume                    |
| `←` Left    | Decrease slice duration (-5ms)  |
| `→` Right   | Increase slice duration (+5ms)  |

**Slice Duration Range:** 1,000 µs to 200,000 µs (1ms to 200ms)

## Display Layout

```
┌──────────────────────────┬───────────────────┐
│                          │ ┌─ File Info ───┐ │
│                          │ │ Format: EVT3  │ │
│   Event Canvas           │ │ Size: 1280x720│ │
│   (ON events in red)     │ │ Filter: Both  │ │
│   (OFF events in blue)   │ └───────────────┘ │
│                          │                   │
│                          │ ┌─ Stats ───────┐ │
│                          │ │ Frame: #1234  │ │
│                          │ │ Rate: 1.2Mev/s│ │
│                          │ │ FPS: 29.8     │ │
│                          │ │ Time: 00:02:15│ │
│                          │ └───────────────┘ │
│                          │                   │
│                          │ ┌─ Header ──────┐ │
│                          │ │ (file header) │ │
│                          │ └───────────────┘ │
└──────────────────────────┴───────────────────┘
```

## Technical Details

- **Library**: Uses libreeb for event processing
- **UI**: ratatui for terminal UI, crossterm for terminal control
- **Rendering**: Canvas widget for event visualization with aspect ratio preservation
- **Event Processing**: Frame-by-frame with configurable slice duration
- **Color Coding**:
  - Bright Red (LightRed): ON events (polarity = 1)
  - Bright Cyan (LightCyan): OFF events (polarity = 0)
- **Dynamic Controls**: Adjust slice duration in real-time (1-200ms range)

## Dependencies

- `ratatui = "0.29"` - Terminal UI framework
- `crossterm = "0.28"` - Cross-platform terminal manipulation  
- `libreeb` - Event camera data processing (core features only)

**Note:** The TUI viewer does NOT require:
- USB camera support (libusb)
- Python bindings (pyo3)

These are disabled via `--no-default-features` flag.

## Notes

- The viewer runs at approximately 30 FPS in the terminal
- Event coordinates are automatically scaled to fit the terminal window
- The canvas uses Braille characters for high-resolution display
- Background pixels (black) are skipped for better performance
- Automatically loops when reaching end of file

## Performance

- Release build is optimized for performance
- Binary size: ~829KB (unstripped), smaller when stripped
- Static musl build: Similar size, fully portable
- No external dependencies required when statically linked (no libusb, no python)

## Troubleshooting

### "No such device or address" error
This typically occurs when running in an environment without a TTY (like CI/CD). The viewer requires a proper terminal to run.

### "cannot find -lusb-1.0" or "cannot find -lpython" error during musl build
You forgot to add `--no-default-features` flag. The correct build command is:
```bash
cargo build --release --target x86_64-unknown-linux-musl --bin tui_viewer --no-default-features
```

### Display issues
- Ensure your terminal supports Unicode and Braille characters
- Try resizing the terminal window if the display looks wrong
- Some terminals may not support all color combinations
