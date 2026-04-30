use crate::EventBatch;
use image::RgbImage;

/// Prophesee-inspired color palettes for CD event visualization.
///
/// Color values are taken directly from OpenEB's `BaseFrameGenerationAlgorithm`
/// (see `colors_impl.h`).  The two palettes are mirror-images of each other:
/// positive and negative event colors are swapped between `Dark` and `Light`.
#[derive(Debug, Clone, Copy, Default)]
pub enum ColorPalette {
    /// Dark background (navy), ON events → white, OFF events → blue.
    /// This is the default Prophesee palette.
    #[default]
    Dark,

    /// White background, ON events → blue, OFF events → dark navy.
    Light,
}

/// An RGB color as a `[R, G, B]` byte triple.
type Color = [u8; 3];

/// The three concrete RGB values needed to paint a frame.
struct PaletteColors {
    background: Color,
    /// Color written for ON events (polarity = 1).
    positive: Color,
    /// Color written for OFF events (polarity = 0).
    negative: Color,
}

impl PaletteColors {
    fn from(palette: ColorPalette) -> Self {
        match palette {
            // Float values from OpenEB scaled to [0, 255]:
            //   background ≈ (0.117, 0.145, 0.203) * 255 → (30, 37, 52)
            //   positive   = (1.0, 1.0, 1.0)            → (255, 255, 255)
            //   negative   ≈ (0.25, 0.492, 0.785) * 255 → (64, 126, 200)
            ColorPalette::Dark => Self {
                background: [30, 37, 52],
                positive: [255, 255, 255],
                negative: [64, 126, 200],
            },
            // Light palette swaps positive ↔ negative relative to dark.
            ColorPalette::Light => Self {
                background: [255, 255, 255],
                positive: [64, 126, 200],
                negative: [30, 37, 52],
            },
        }
    }
}

/// Generates an [`image::RgbImage`] frame from an [`EventBatch`].
///
/// ## Algorithm
///
/// 1. Allocate a `width × height` frame and fill it with the background color.
/// 2. For every event `(x, y, p)` in the batch, overwrite pixel `(x, y)` with
///    the polarity-dependent color:
///    - `p = 1` (ON event)  → **positive** color (white in `Dark` palette)
///    - `p = 0` (OFF event) → **negative** color (blue in `Dark` palette)
/// 3. When multiple events share the same `(x, y)`, the **last one wins**,
///    matching OpenEB's `BaseFrameGenerationAlgorithm` behaviour.
/// 4. Events whose coordinates fall outside `[0, width) × [0, height)` are
///    silently ignored (no bounds-check panic).
///
/// ## Usage
///
/// ```no_run
/// use libreeb::processors::{ColorPalette, FrameGenerator};
///
/// let gen = FrameGenerator::new(1280, 720);
/// // or: FrameGenerator::with_palette(1280, 720, ColorPalette::Light);
/// ```
pub struct FrameGenerator {
    width: u32,
    height: u32,
    colors: PaletteColors,
}

impl FrameGenerator {
    /// Creates a generator using the [`ColorPalette::Dark`] palette.
    pub fn new(width: u32, height: u32) -> Self {
        Self::with_palette(width, height, ColorPalette::Dark)
    }

    /// Creates a generator using the given [`ColorPalette`].
    pub fn with_palette(width: u32, height: u32, palette: ColorPalette) -> Self {
        FrameGenerator {
            width,
            height,
            colors: PaletteColors::from(palette),
        }
    }

    /// Generates an RGB frame from a single [`EventBatch`].
    ///
    /// Returns an [`image::RgbImage`] of size `width × height`.
    pub fn generate(&self, batch: &EventBatch) -> RgbImage {
        let mut img = RgbImage::from_pixel(
            self.width,
            self.height,
            image::Rgb(self.colors.background),
        );
        for evt in batch {
            if (evt.x as u32) < self.width && (evt.y as u32) < self.height {
                let color = if evt.p != 0 {
                    self.colors.positive
                } else {
                    self.colors.negative
                };
                img.put_pixel(evt.x as u32, evt.y as u32, image::Rgb(color));
            }
        }
        img
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RawFileReader;
    use rstest::rstest;
    use std::path::Path;

    // Dark palette reference values for compact assertions.
    const BG: [u8; 3] = [30, 37, 52];
    const POS: [u8; 3] = [255, 255, 255];
    const NEG: [u8; 3] = [64, 126, 200];

    /// An empty batch should produce a frame entirely filled with the background color.
    #[test]
    fn empty_batch_is_all_background() {
        let gen = FrameGenerator::new(10, 8);
        let frame = gen.generate(&EventBatch::new());
        assert_eq!(frame.dimensions(), (10, 8));
        for pixel in frame.pixels() {
            assert_eq!(pixel.0, BG);
        }
    }

    /// The frame dimensions must match the values supplied at construction.
    #[test]
    fn frame_dimensions_match_constructor() {
        let gen = FrameGenerator::new(320, 240);
        let frame = gen.generate(&EventBatch::new());
        assert_eq!(frame.dimensions(), (320, 240));
    }

    /// A positive event (p=1) paints the pixel with the positive color.
    /// Adjacent pixels must remain untouched.
    #[test]
    fn positive_event_paints_positive_color() {
        let gen = FrameGenerator::new(10, 10);
        let mut batch = EventBatch::new();
        batch.push(5, 3, 0, 1);
        let frame = gen.generate(&batch);
        assert_eq!(frame.get_pixel(5, 3).0, POS);
        assert_eq!(frame.get_pixel(4, 3).0, BG, "neighbor should be background");
        assert_eq!(frame.get_pixel(5, 4).0, BG, "neighbor should be background");
    }

    /// A negative event (p=0) paints the pixel with the negative color.
    #[test]
    fn negative_event_paints_negative_color() {
        let gen = FrameGenerator::new(10, 10);
        let mut batch = EventBatch::new();
        batch.push(2, 7, 0, 0);
        let frame = gen.generate(&batch);
        assert_eq!(frame.get_pixel(2, 7).0, NEG);
    }

    /// Events outside `[0, width) × [0, height)` must be ignored — no panic, no write.
    #[test]
    fn out_of_bounds_events_are_ignored() {
        let gen = FrameGenerator::new(10, 10);
        let mut batch = EventBatch::new();
        batch.push(10, 0, 0, 1); // x == width  → out of bounds
        batch.push(0, 10, 0, 1); // y == height → out of bounds
        batch.push(200, 200, 0, 1); // both way out
        let frame = gen.generate(&batch);
        for pixel in frame.pixels() {
            assert_eq!(pixel.0, BG);
        }
    }

    /// When two events land on the same pixel, the later one (by position in
    /// the batch) wins — matching OpenEB's last-write-wins semantics.
    #[test]
    fn last_event_wins_on_same_pixel() {
        let gen = FrameGenerator::new(10, 10);
        let mut batch = EventBatch::new();
        batch.push(5, 5, 0, 1); // positive first
        batch.push(5, 5, 1, 0); // negative second → should win
        let frame = gen.generate(&batch);
        assert_eq!(frame.get_pixel(5, 5).0, NEG);
    }

    /// The `Light` palette must use the complementary colors relative to `Dark`.
    #[test]
    fn light_palette_uses_correct_colors() {
        let gen = FrameGenerator::with_palette(10, 10, ColorPalette::Light);
        let mut batch = EventBatch::new();
        batch.push(1, 1, 0, 1); // positive in Light → blue (= NEG in Dark)
        batch.push(2, 2, 0, 0); // negative in Light → dark navy (= BG in Dark)
        let frame = gen.generate(&batch);
        assert_eq!(frame.get_pixel(1, 1).0, NEG); // blue
        assert_eq!(frame.get_pixel(2, 2).0, BG); // dark
        // background should be white
        assert_eq!(frame.get_pixel(0, 0).0, [255u8, 255, 255]);
    }

    // ── Integration tests with real files ─────────────────────────────────────

    /// Generate a frame from the first decoded batch of a real recording and
    /// verify:
    ///   - the frame has the correct sensor dimensions (from the file header),
    ///   - at least one pixel differs from the background (real events were painted).
    #[rstest]
    #[case("data/gen4_evt3_hand.raw", 1280, 720)]
    #[case("data/claque_doigt_evt21.raw", 320, 320)]
    fn generate_frame_from_real_file(#[case] path: &str, #[case] w: u32, #[case] h: u32) {
        let mut reader = RawFileReader::new(Path::new(path)).unwrap();
        // Also verify that the parsed header geometry matches what we expect.
        assert_eq!(reader.header.camera_geometry.width, w);
        assert_eq!(reader.header.camera_geometry.height, h);

        let batch = reader.read_events().next().expect("no batches in file");
        let gen = FrameGenerator::new(w, h);
        let frame = gen.generate(&batch);
        assert_eq!(frame.dimensions(), (w, h));

        let has_non_bg = frame.pixels().any(|p| p.0 != BG);
        assert!(has_non_bg, "expected at least one event to be painted");
    }
}
