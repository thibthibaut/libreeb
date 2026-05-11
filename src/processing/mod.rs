//! Event processing algorithms: frame generation, filtering, tracking.
//!
//! This module provides high-level event processing utilities:
//! - [`FrameGenerator`]: Convert event batches to RGB frames

mod frame_gen;

pub use frame_gen::FrameGenerator;
