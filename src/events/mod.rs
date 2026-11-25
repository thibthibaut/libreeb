//! Event representation types for decoder output.
//!
//! This module provides different ways to store decoded events:
//! - [`EventSoA`]: Structure-of-Arrays layout (cache-friendly, default)
//! - [`EventAoS`]: Array-of-Structures layout (simpler, more intuitive)
//! - [`EventBuf`]: packed `u32` events + run-length timestamps (smallest)
//! - [`EventCD`]: Individual Change Detection event
//! - [`Trigger`]: External trigger event
//!
//! The [`EventCollection`] trait provides a unified interface for collections
//! that can be queried, iterated, and manipulated.

mod aos;
mod buf;
mod soa;
mod types;

pub use aos::EventAoS;
pub use buf::{EventBuf, EventId, TimeMark};
pub use soa::{EventBatch, EventSoA};
pub use types::{EventCD, Trigger};

use crate::DecodeInto;

/// Trait for event representations that can be iterated and manipulated as collections.
///
/// This trait represents event storage types that:
/// - Can be queried for their size (`num_events`, `len`)
/// - Can be iterated over to access individual events
/// - Support collection operations like `append` and `split_off`
///
/// Not all `DecodeInto` implementations are collections. For example, destructive
/// representations like histograms or time surfaces implement `DecodeInto` (to receive
/// decoded events) but do NOT implement `EventCollection` (cannot iterate over past events).
///
/// Types implementing this trait:
/// - [`EventSoA`]: Structure-of-Arrays layout
/// - [`EventAoS`]: Array-of-Structures layout
/// - [`EventBuf`]: packed events + run-length timestamps
pub trait EventCollection: DecodeInto {
    /// Create a new empty collection.
    fn new() -> Self;

    /// Create a new collection with pre-allocated capacity for `cap` events.
    fn with_capacity(cap: usize) -> Self;

    /// Total number of CD events stored in this collection.
    fn num_events(&self) -> usize;

    /// Standard Rust collection method. For event collections, returns the same
    /// value as `num_events()`. Provided for consistency with Rust conventions.
    fn len(&self) -> usize {
        self.num_events()
    }

    /// Whether the collection contains no CD events.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Append all events from `other` into `self`, leaving `other` empty.
    fn append(&mut self, other: &mut Self);

    /// Split the collection at index `at`.
    ///
    /// Returns a new collection containing events `[at..)`, while `self` retains `[0..at)`.
    fn split_off(&mut self, at: usize) -> Self;

    /// Iterator over CD events in this collection.
    fn iter_cd(&self) -> impl Iterator<Item = EventCD> + '_;

    /// Iterator over trigger events in this collection.
    fn iter_triggers(&self) -> impl Iterator<Item = Trigger> + '_;
}

// ── EventCollection implementations ─────────────────────────────────────────

impl EventCollection for EventSoA {
    fn new() -> Self {
        EventSoA::new()
    }

    fn with_capacity(cap: usize) -> Self {
        EventSoA::with_capacity(cap)
    }

    fn num_events(&self) -> usize {
        self.x.len()
    }

    fn append(&mut self, other: &mut Self) {
        EventSoA::append(self, other);
    }

    fn split_off(&mut self, at: usize) -> Self {
        EventSoA::split_off(self, at)
    }

    fn iter_cd(&self) -> impl Iterator<Item = EventCD> + '_ {
        (0..self.len()).map(move |i| EventCD {
            x: self.x[i],
            y: self.y[i],
            p: self.p[i],
            t: self.t[i],
        })
    }

    fn iter_triggers(&self) -> impl Iterator<Item = Trigger> + '_ {
        self.triggers.iter().copied()
    }
}

impl EventCollection for EventAoS {
    fn new() -> Self {
        EventAoS::new()
    }

    fn with_capacity(cap: usize) -> Self {
        EventAoS::with_capacity(cap)
    }

    fn num_events(&self) -> usize {
        self.events.len()
    }

    fn append(&mut self, other: &mut Self) {
        EventAoS::append(self, other);
    }

    fn split_off(&mut self, at: usize) -> Self {
        EventAoS::split_off(self, at)
    }

    fn iter_cd(&self) -> impl Iterator<Item = EventCD> + '_ {
        self.events.iter().copied()
    }

    fn iter_triggers(&self) -> impl Iterator<Item = Trigger> + '_ {
        self.triggers.iter().copied()
    }
}

impl EventCollection for EventBuf {
    fn new() -> Self {
        EventBuf::new()
    }

    fn with_capacity(cap: usize) -> Self {
        EventBuf::with_capacity(cap)
    }

    fn num_events(&self) -> usize {
        EventBuf::len(self)
    }

    fn append(&mut self, other: &mut Self) {
        EventBuf::append(self, other);
    }

    fn split_off(&mut self, at: usize) -> Self {
        EventBuf::split_off(self, at)
    }

    fn iter_cd(&self) -> impl Iterator<Item = EventCD> + '_ {
        EventBuf::iter_cd(self)
    }

    fn iter_triggers(&self) -> impl Iterator<Item = Trigger> + '_ {
        self.triggers.iter().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_soa_basic() {
        let mut batch = EventSoA::new();
        batch.push(10, 20, 1000, 1);
        batch.push(15, 25, 2000, 0);

        assert_eq!(batch.len(), 2);
        assert_eq!(batch.x[0], 10);
        assert_eq!(batch.t[1], 2000);
    }

    #[test]
    fn test_event_aos_basic() {
        let mut batch = EventAoS::new();
        batch.push(EventCD {
            x: 10,
            y: 20,
            p: 1,
            t: 1000,
        });
        batch.push(EventCD {
            x: 15,
            y: 25,
            p: 0,
            t: 2000,
        });

        assert_eq!(batch.len(), 2);
        assert_eq!(batch.events[0].x, 10);
        assert_eq!(batch.events[1].t, 2000);
    }

    #[test]
    fn test_events_repr_trait_soa() {
        let mut batch = EventSoA::new();
        DecodeInto::push_cd(&mut batch, 10, 20, 1, 1000);
        DecodeInto::push_cd(&mut batch, 15, 25, 0, 2000);

        assert_eq!(batch.len(), 2);
    }

    #[test]
    fn test_events_repr_trait_aos() {
        let mut batch = EventAoS::new();
        DecodeInto::push_cd(&mut batch, 10, 20, 1, 1000);
        DecodeInto::push_cd(&mut batch, 15, 25, 0, 2000);

        assert_eq!(batch.len(), 2);
        assert_eq!(batch.events[0].x, 10);
    }
}
