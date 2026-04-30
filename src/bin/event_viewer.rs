use std::path::Path;

use anyhow::Context;
use libreeb::{processors::FrameGenerator, RawFileReader, SliceBy, SliceIter};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} <file.raw> [window_us]", args[0]);
        eprintln!();
        eprintln!("  window_us  Time window per frame in microseconds. Default: 33333.");
        eprintln!();
        eprintln!("  Sensor dimensions are read from the file header when available.");
        eprintln!("  Override them by appending width and height:");
        eprintln!("    {} <file.raw> [window_us] [width] [height]", args[0]);
        std::process::exit(1);
    }

    let path = Path::new(&args[1]);
    let window_us: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(33_333);

    let mut reader =
        RawFileReader::new(path).with_context(|| format!("Failed to open {:?}", path))?;

    // Geometry: prefer what the header says; allow CLI overrides as fallback.
    let width: u32 = if reader.header.camera_geometry.width > 0 {
        reader.header.camera_geometry.width
    } else {
        args.get(3).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
            eprintln!("Warning: no geometry in header and no CLI override — assuming 1280.");
            1280
        })
    };

    let height: u32 = if reader.header.camera_geometry.height > 0 {
        reader.header.camera_geometry.height
    } else {
        args.get(4).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
            eprintln!("Warning: no geometry in header and no CLI override — assuming 720.");
            720
        })
    };

    println!("File   : {}", path.display());
    println!("Sensor : {}×{}", width, height);
    println!(
        "Window : {} µs  ({:.1} fps)",
        window_us,
        1_000_000.0 / window_us as f64
    );

    let rec = rerun::RecordingStreamBuilder::new("libreeb/event_viewer")
        .spawn()
        .context("Failed to spawn rerun viewer")?;

    let generator = FrameGenerator::new(width, height);

    let mut frame_nr: i64 = 0;
    for batch in SliceIter::new(reader.read_events(), SliceBy::Time(window_us)) {
        let t_secs = batch.t[0] as f64 / 1_000_000.0;

        rec.set_time_sequence("frame_nr", frame_nr);
        rec.set_duration_secs("event_time", t_secs);

        let frame = generator.generate(&batch);
        rec.log(
            "events/frame",
            &rerun::Image::from_rgb24(frame.into_raw(), [width, height]),
        )?;

        frame_nr += 1;
    }

    println!("Logged {} frames.", frame_nr);
    Ok(())
}
