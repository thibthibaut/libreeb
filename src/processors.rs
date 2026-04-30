use crate::EventBatch;
use image::RgbImage;

// colorgrad is available for future custom gradient work.
#[allow(unused_imports)]
use colorgrad as _;

const BACKGROUND: [u8; 3] = [10, 10, 10];      // near-black
const POSITIVE:   [u8; 3] = [255, 140,   0];   // vivid orange  (ON events)
const NEGATIVE:   [u8; 3] = [  0, 200, 255];   // vivid cyan    (OFF events)

/// Generates an [`image::RgbImage`] frame from an [`EventBatch`].
///
/// Each pixel is painted with the colour of the **last** event that touched it
/// (last-write-wins), matching OpenEB's `BaseFrameGenerationAlgorithm`:
///
/// | condition          | colour       |
/// |--------------------|--------------|
/// | no events          | near-black   |
/// | ON  event (`p=1`)  | vivid orange |
/// | OFF event (`p=0`)  | vivid cyan   |
///
/// Out-of-bounds events are silently ignored.
pub struct FrameGenerator {
    width:  u32,
    height: u32,
}

impl FrameGenerator {
    pub fn new(width: u32, height: u32) -> Self {
        FrameGenerator { width, height }
    }

    /// Generates an RGB frame from a single [`EventBatch`].
    pub fn generate(&self, batch: &EventBatch) -> RgbImage {
        let mut img = RgbImage::from_pixel(
            self.width,
            self.height,
            image::Rgb(BACKGROUND),
        );
        for evt in batch {
            if (evt.x as u32) < self.width && (evt.y as u32) < self.height {
                let color = if evt.p != 0 { POSITIVE } else { NEGATIVE };
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

    #[test]
    fn empty_batch_is_background() {
        let gen = FrameGenerator::new(10, 8);
        let frame = gen.generate(&EventBatch::new());
        assert_eq!(frame.dimensions(), (10, 8));
        for pixel in frame.pixels() {
            assert_eq!(pixel.0, BACKGROUND);
        }
    }

    #[test]
    fn frame_dimensions_match_constructor() {
        let gen = FrameGenerator::new(320, 240);
        assert_eq!(gen.generate(&EventBatch::new()).dimensions(), (320, 240));
    }

    #[test]
    fn positive_event_paints_orange() {
        let gen = FrameGenerator::new(10, 10);
        let mut batch = EventBatch::new();
        batch.push(5, 3, 0, 1);
        let frame = gen.generate(&batch);
        assert_eq!(frame.get_pixel(5, 3).0, POSITIVE);
        assert_eq!(frame.get_pixel(4, 3).0, BACKGROUND);
    }

    #[test]
    fn negative_event_paints_cyan() {
        let gen = FrameGenerator::new(10, 10);
        let mut batch = EventBatch::new();
        batch.push(2, 7, 0, 0);
        let frame = gen.generate(&batch);
        assert_eq!(frame.get_pixel(2, 7).0, NEGATIVE);
    }

    #[test]
    fn last_event_wins_on_same_pixel() {
        let gen = FrameGenerator::new(10, 10);
        let mut batch = EventBatch::new();
        batch.push(5, 5, 0, 1); // positive first
        batch.push(5, 5, 1, 0); // negative last → wins
        let frame = gen.generate(&batch);
        assert_eq!(frame.get_pixel(5, 5).0, NEGATIVE);
    }

    #[test]
    fn out_of_bounds_events_are_ignored() {
        let gen = FrameGenerator::new(10, 10);
        let mut batch = EventBatch::new();
        batch.push(10,  0,  0, 1);
        batch.push( 0, 10,  0, 1);
        batch.push(200, 200, 0, 1);
        let frame = gen.generate(&batch);
        for pixel in frame.pixels() {
            assert_eq!(pixel.0, BACKGROUND);
        }
    }

    #[rstest]
    #[case("data/gen4_evt3_hand.raw", 1280, 720)]
    #[case("data/claque_doigt_evt21.raw", 320, 320)]
    fn generate_frame_from_real_file(#[case] path: &str, #[case] w: u32, #[case] h: u32) {
        let mut reader = RawFileReader::new(Path::new(path)).unwrap();
        assert_eq!(reader.header.camera_geometry.width, w);
        assert_eq!(reader.header.camera_geometry.height, h);

        let batch = reader.read_events().next().expect("no batches in file");
        let gen = FrameGenerator::new(w, h);
        let frame = gen.generate(&batch);
        assert_eq!(frame.dimensions(), (w, h));
        assert!(
            frame.pixels().any(|p| p.0 != BACKGROUND),
            "expected at least one painted pixel"
        );
    }
}
