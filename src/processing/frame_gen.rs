use crate::events::EventBatch;
use crate::io::header::CameraGeometry;
use image::RgbImage;

const BACKGROUND: [u8; 3] = [30, 30, 46];
const POSITIVE: [u8; 3] = [245, 194, 231];
const NEGATIVE: [u8; 3] = [137, 220, 235];
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
    geometry: CameraGeometry,
}

impl FrameGenerator {
    pub fn new(geometry: CameraGeometry) -> Self {
        FrameGenerator { geometry }
    }

    /// Generates an RGB frame from a single [`EventBatch`].
    pub fn generate(&self, batch: &EventBatch) -> RgbImage {
        let mut img = RgbImage::from_pixel(
            self.geometry.width,
            self.geometry.height,
            image::Rgb(BACKGROUND)
        );
        for evt in batch {
            if (evt.x as u32) < self.geometry.width && (evt.y as u32) < self.geometry.height {
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
        let gen = FrameGenerator::new(CameraGeometry { width: 10, height: 8 });
        let frame = gen.generate(&EventBatch::new());
        assert_eq!(frame.dimensions(), (10, 8));
        for pixel in frame.pixels() {
            assert_eq!(pixel.0, BACKGROUND);
        }
    }

    #[test]
    fn frame_dimensions_match_constructor() {
        let gen = FrameGenerator::new(CameraGeometry { width: 320, height: 240 });
        assert_eq!(gen.generate(&EventBatch::new()).dimensions(), (320, 240));
    }

    #[test]
    fn positive_event_paints_orange() {
        let gen = FrameGenerator::new(CameraGeometry { width: 10, height: 10 });
        let mut batch = EventBatch::new();
        batch.push(5, 3, 0, 1);
        let frame = gen.generate(&batch);
        assert_eq!(frame.get_pixel(5, 3).0, POSITIVE);
        assert_eq!(frame.get_pixel(4, 3).0, BACKGROUND);
    }

    #[test]
    fn negative_event_paints_cyan() {
        let gen = FrameGenerator::new(CameraGeometry { width: 10, height: 10 });
        let mut batch = EventBatch::new();
        batch.push(2, 7, 0, 0);
        let frame = gen.generate(&batch);
        assert_eq!(frame.get_pixel(2, 7).0, NEGATIVE);
    }

    #[test]
    fn last_event_wins_on_same_pixel() {
        let gen = FrameGenerator::new(CameraGeometry { width: 10, height: 10 });
        let mut batch = EventBatch::new();
        batch.push(5, 5, 0, 1); // positive first
        batch.push(5, 5, 1, 0); // negative last → wins
        let frame = gen.generate(&batch);
        assert_eq!(frame.get_pixel(5, 5).0, NEGATIVE);
    }

    #[test]
    fn out_of_bounds_events_are_ignored() {
        let gen = FrameGenerator::new(CameraGeometry { width: 10, height: 10 });
        let mut batch = EventBatch::new();
        batch.push(10, 0, 0, 1);
        batch.push(0, 10, 0, 1);
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

        let batch = reader.decoder().next().expect("no batches in file");
        let gen = FrameGenerator::new(reader.header.camera_geometry);
        let frame = gen.generate(&batch);
        assert_eq!(frame.dimensions(), (w, h));
        assert!(
            frame.pixels().any(|p| p.0 != BACKGROUND),
            "expected at least one painted pixel"
        );
    }
}
