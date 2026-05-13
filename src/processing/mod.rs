//! Event processing algorithms: frame generation, filtering, tracking.
//!
//! This module provides high-level event processing utilities:
//! - [`FrameGenerator`]: Convert event batches to RGB frames
//! - [`TimeSurface`]: Per-pixel timestamp accumulator with jet-colormap rendering

mod frame_gen;
mod time_surface;

pub use frame_gen::FrameGenerator;
pub use time_surface::TimeSurface;
