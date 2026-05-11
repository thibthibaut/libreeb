//! Core event types: EventCD and Trigger.

#[cfg(all(not(target_arch = "wasm32"), feature = "python"))]
use pyo3::prelude::*;

/// A single decoded Change Detection (CD) event.
///
/// CD events are the primary output of an event-based camera: each one records
/// that a pixel at (`x`, `y`) crossed a log-intensity threshold at time `t`,
/// with polarity `p` (1 = brighter, 0 = darker).
///
/// Timestamps are in microseconds relative to the start of the recording.
#[cfg_attr(all(not(target_arch = "wasm32"), feature = "python"), pyclass)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventCD {
    pub x: u16,
    pub y: u16,
    pub p: u8,
    pub t: u64,
}

/// An external trigger event.
///
/// External triggers are synchronization signals recorded alongside CD events,
/// typically used for correlating camera data with external sensors or systems.
///
/// - `id`: trigger channel/ID (0–15 in EVT3, 0–31 in EVT2/EVT21)
/// - `p`: trigger value/polarity (0 or 1)
/// - `t`: timestamp in microseconds relative to the start of the recording
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trigger {
    pub id: u8,
    pub p: bool,
    pub t: u64,
}
