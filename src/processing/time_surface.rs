//! Time Surface: per-pixel timestamp accumulator with jet-colourmap rendering.
//!
//! A *time surface* (also called a *surface of active events*, or SAE) stores
//! the timestamp of the **last** event that fired at each pixel.  On rendering,
//! each pixel's age `(current_time - last_event_time)` is mapped through an
//! exponential decay to a value in [0, 1]:
//!
//! ```text
//! value = exp(-age_µs / decay_µs)
//! ```
//!
//! value ≈ 1 → very recent event → hot (red/yellow)
//! value ≈ 0 → stale / no event  → cold (blue) / background

use crate::events::EventBatch;
use image::RgbImage;

const BACKGROUND: [u8; 3] = [30, 30, 46];

/// Per-pixel last-event-time accumulator.
pub struct TimeSurface {
    /// Flat array of last-event timestamps in µs, indexed `y * width + x`.
    /// `u64::MAX` means "never fired".
    timestamps: Vec<u64>,
    width: u32,
    height: u32,
}

impl TimeSurface {
    pub fn new(width: u32, height: u32) -> Self {
        TimeSurface {
            timestamps: vec![u64::MAX; (width * height) as usize],
            width,
            height,
        }
    }

    /// Update the surface with a new batch of events.
    #[inline]
    pub fn update(&mut self, batch: &EventBatch) {
        let w = self.width as usize;
        let h = self.height as usize;
        let n = batch.len();
        for i in 0..n {
            let x = batch.x[i] as usize;
            let y = batch.y[i] as usize;
            if x < w && y < h {
                self.timestamps[y * w + x] = batch.t[i];
            }
        }
    }

    /// Render to an RGB image.
    ///
    /// * `current_time` – reference timestamp in µs (typically the latest event time).
    /// * `decay_us` – decay time constant in µs; smaller = faster fade.
    pub fn render(&self, current_time: u64, decay_us: u64) -> RgbImage {
        let decay = decay_us as f32;
        let mut img = RgbImage::from_pixel(
            self.width,
            self.height,
            image::Rgb(BACKGROUND),
        );
        for y in 0..self.height {
            for x in 0..self.width {
                let ts = self.timestamps[(y * self.width + x) as usize];
                if ts == u64::MAX {
                    continue; // never fired → background
                }
                let age = current_time.saturating_sub(ts) as f32;
                let value = (-age / decay).exp();
                img.put_pixel(x, y, image::Rgb(jet(value)));
            }
        }
        img
    }

    /// Clear all accumulated timestamps (reset to "never fired").
    pub fn reset(&mut self) {
        self.timestamps.fill(u64::MAX);
    }
}

// ── Jet colourmap ─────────────────────────────────────────────────────────────
//
// Maps value ∈ [0, 1] → RGB using the classic MATLAB *jet* ramp:
//   0.0 → dark blue
//   0.5 → green
//   1.0 → red

#[inline]
fn jet(v: f32) -> [u8; 3] {
    let v = v.clamp(0.0, 1.0);
    let (r, g, b): (f32, f32, f32) = if v < 0.125 {
        (0.0, 0.0, 0.5 + 4.0 * v)
    } else if v < 0.375 {
        (0.0, 4.0 * v - 0.5, 1.0)
    } else if v < 0.625 {
        (4.0 * v - 1.5, 1.0, 2.5 - 4.0 * v)
    } else if v < 0.875 {
        (1.0, 3.5 - 4.0 * v, 0.0)
    } else {
        (4.5 - 4.0 * v, 0.0, 0.0)
    };
    [
        (r.clamp(0.0, 1.0) * 255.0) as u8,
        (g.clamp(0.0, 1.0) * 255.0) as u8,
        (b.clamp(0.0, 1.0) * 255.0) as u8,
    ]
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EventBatch;

    #[test]
    fn new_surface_renders_all_background() {
        let ts = TimeSurface::new(4, 4);
        let img = ts.render(1_000_000, 30_000);
        for p in img.pixels() {
            assert_eq!(p.0, BACKGROUND);
        }
    }

    #[test]
    fn recent_event_is_hot() {
        let mut ts = TimeSurface::new(8, 8);
        let mut b = EventBatch::new();
        b.push(3, 3, 1_000_000, 1);
        ts.update(&b);
        let img = ts.render(1_000_000, 30_000); // age = 0 → value = 1 → red-ish
        let p = img.get_pixel(3, 3).0;
        // jet(1.0) → (0.5, 0, 0) = dark red; red channel must be dominant
        assert!(
            p[0] > p[1] && p[0] > p[2],
            "expected red dominant for age=0, got {:?}",
            p
        );
    }

    #[test]
    fn stale_event_is_cold() {
        let mut ts = TimeSurface::new(8, 8);
        let mut b = EventBatch::new();
        b.push(2, 2, 0, 0);
        ts.update(&b);
        // current_time far in the future → age >> decay → value ≈ 0 → dark blue
        let img = ts.render(10_000_000, 30_000);
        let p = img.get_pixel(2, 2).0;
        // jet(≈0) → blue channel dominant
        assert!(p[2] > p[0], "expected blue dominant for stale event, got {:?}", p);
    }

    #[test]
    fn reset_clears_all_pixels() {
        let mut ts = TimeSurface::new(4, 4);
        let mut b = EventBatch::new();
        b.push(1, 1, 500_000, 1);
        ts.update(&b);
        ts.reset();
        let img = ts.render(1_000_000, 30_000);
        for p in img.pixels() {
            assert_eq!(p.0, BACKGROUND);
        }
    }
}
