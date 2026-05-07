//! Event representation types for decoder output.
//!
//! This module defines different ways to represent decoded events:
//! - [`EventSoA`]: Structure-of-Arrays (SoA) layout - separate vectors for x, y, p, t
//! - [`EventAoS`]: Array-of-Structures (AoS) layout - vector of EventCD structs
//!
//! The default representation is [`EventBatch`], which is a type alias for [`EventSoA`].

#[cfg(all(not(target_arch = "wasm32"), feature = "python"))]
use pyo3::prelude::*;

use crate::EventsRepr;

// ── EventCD ──────────────────────────────────────────────────────────────────

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

// ── EventSoA (Structure-of-Arrays) ──────────────────────────────────────────

/// Structure-of-Arrays (SoA) event batch representation.
///
/// Stores events in separate vectors for each field (x, y, p, t). This layout
/// is cache-friendly for bulk operations and SIMD processing, and is the default
/// output format for decoders.
///
/// This type is re-exported as [`EventBatch`] for convenience.
#[cfg_attr(all(not(target_arch = "wasm32"), feature = "python"), pyclass)]
#[derive(Debug, Clone)]
pub struct EventSoA {
    pub x: Vec<u16>,
    pub y: Vec<u16>,
    pub p: Vec<u8>,
    pub t: Vec<u64>,
}

impl EventSoA {
    pub fn new() -> Self {
        EventSoA {
            x: Vec::new(),
            y: Vec::new(),
            p: Vec::new(),
            t: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.x.len()
    }

    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
    }

    pub fn with_capacity(cap: usize) -> Self {
        EventSoA {
            x: Vec::with_capacity(cap),
            y: Vec::with_capacity(cap),
            p: Vec::with_capacity(cap),
            t: Vec::with_capacity(cap),
        }
    }

    /// Push from separate field values.
    #[inline]
    pub fn push(&mut self, x: u16, y: u16, t: u64, p: u8) {
        self.x.push(x);
        self.y.push(y);
        self.p.push(p);
        self.t.push(t);
    }

    /// Push a single [`EventCD`] (convenience wrapper).
    #[inline]
    pub fn push_event(&mut self, evt: EventCD) {
        self.push(evt.x, evt.y, evt.t, evt.p);
    }

    /// Append all events from `other` into `self`.
    pub fn append(&mut self, mut other: EventSoA) {
        self.x.append(&mut other.x);
        self.y.append(&mut other.y);
        self.p.append(&mut other.p);
        self.t.append(&mut other.t);
    }

    /// Split at index `at`. `self` retains `[0..at)`, returns `[at..)`.
    pub fn split_off(&mut self, at: usize) -> EventSoA {
        EventSoA {
            x: self.x.split_off(at),
            y: self.y.split_off(at),
            p: self.p.split_off(at),
            t: self.t.split_off(at),
        }
    }
}

impl Default for EventSoA {
    fn default() -> Self {
        Self::new()
    }
}

impl EventsRepr for EventSoA {
    #[inline]
    fn push_cd(&mut self, x: u16, y: u16, p: u8, t: u64) {
        self.x.push(x);
        self.y.push(y);
        self.p.push(p);
        self.t.push(t);
    }

    #[inline]
    fn extend_from_soa(&mut self, x: &[u16], y: &[u16], p: &[u8], t: &[u64]) {
        self.x.extend_from_slice(x);
        self.y.extend_from_slice(y);
        self.p.extend_from_slice(p);
        self.t.extend_from_slice(t);
    }

    #[inline]
    fn extend_constant_ypt(&mut self, xs: &[u16], y: u16, p: u8, t: u64) {
        self.x.extend_from_slice(xs);
        let new_len = self.y.len() + xs.len();
        self.y.resize(new_len, y);
        self.p.resize(new_len, p);
        self.t.resize(new_len, t);
    }

    #[inline]
    fn len(&self) -> usize {
        self.x.len()
    }

    #[inline]
    fn last_t(&self) -> Option<u64> {
        self.t.last().copied()
    }

    #[inline]
    fn reserve(&mut self, n: usize) {
        self.x.reserve(n);
        self.y.reserve(n);
        self.p.reserve(n);
        self.t.reserve(n);
    }
}

pub struct EventSoAIter<'a> {
    batch: &'a EventSoA,
    index: usize,
}

impl<'a> Iterator for EventSoAIter<'a> {
    type Item = EventCD;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.batch.len() {
            let i = self.index;
            self.index += 1;
            Some(EventCD {
                x: self.batch.x[i],
                y: self.batch.y[i],
                p: self.batch.p[i],
                t: self.batch.t[i],
            })
        } else {
            None
        }
    }
}

impl<'a> IntoIterator for &'a EventSoA {
    type Item = EventCD;
    type IntoIter = EventSoAIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        EventSoAIter {
            batch: self,
            index: 0,
        }
    }
}

#[cfg(all(not(target_arch = "wasm32"), feature = "python"))]
#[pymethods]
impl EventSoA {
    fn __len__(&self) -> usize {
        self.len()
    }

    fn __repr__(&self) -> String {
        format!("EventSoA(len={})", self.len())
    }
}

/// Type alias for the default event batch representation.
///
/// Currently points to [`EventSoA`] (Structure-of-Arrays layout).
pub type EventBatch = EventSoA;

// ── EventAoS (Array-of-Structures) ──────────────────────────────────────────

/// Array-of-Structures (AoS) event batch representation.
///
/// Stores events as a vector of [`EventCD`] structs. This layout is simpler
/// and more intuitive than SoA, but may be less cache-efficient for bulk
/// operations. Use this when you need to work with individual events or when
/// interoperating with code that expects `Vec<EventCD>`.
#[derive(Debug, Clone)]
pub struct EventAoS {
    pub events: Vec<EventCD>,
}

impl EventAoS {
    pub fn new() -> Self {
        EventAoS {
            events: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn with_capacity(cap: usize) -> Self {
        EventAoS {
            events: Vec::with_capacity(cap),
        }
    }

    pub fn push(&mut self, evt: EventCD) {
        self.events.push(evt);
    }

    pub fn append(&mut self, mut other: EventAoS) {
        self.events.append(&mut other.events);
    }

    pub fn split_off(&mut self, at: usize) -> EventAoS {
        EventAoS {
            events: self.events.split_off(at),
        }
    }
}

impl Default for EventAoS {
    fn default() -> Self {
        Self::new()
    }
}

impl EventsRepr for EventAoS {
    #[inline]
    fn push_cd(&mut self, x: u16, y: u16, p: u8, t: u64) {
        self.events.push(EventCD { x, y, p, t });
    }

    #[inline]
    fn extend_from_soa(&mut self, x: &[u16], y: &[u16], p: &[u8], t: &[u64]) {
        self.events.reserve(x.len());
        for i in 0..x.len() {
            self.events.push(EventCD {
                x: x[i],
                y: y[i],
                p: p[i],
                t: t[i],
            });
        }
    }

    #[inline]
    fn extend_constant_ypt(&mut self, xs: &[u16], y: u16, p: u8, t: u64) {
        self.events.reserve(xs.len());
        for &x in xs {
            self.events.push(EventCD { x, y, p, t });
        }
    }

    #[inline]
    fn len(&self) -> usize {
        self.events.len()
    }

    #[inline]
    fn last_t(&self) -> Option<u64> {
        self.events.last().map(|e| e.t)
    }

    #[inline]
    fn reserve(&mut self, n: usize) {
        self.events.reserve(n);
    }
}

impl<'a> IntoIterator for &'a EventAoS {
    type Item = &'a EventCD;
    type IntoIter = std::slice::Iter<'a, EventCD>;

    fn into_iter(self) -> Self::IntoIter {
        self.events.iter()
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
        batch.push(EventCD { x: 10, y: 20, p: 1, t: 1000 });
        batch.push(EventCD { x: 15, y: 25, p: 0, t: 2000 });
        
        assert_eq!(batch.len(), 2);
        assert_eq!(batch.events[0].x, 10);
        assert_eq!(batch.events[1].t, 2000);
    }

    #[test]
    fn test_events_repr_trait_soa() {
        let mut batch = EventSoA::new();
        EventsRepr::push_cd(&mut batch, 10, 20, 1, 1000);
        EventsRepr::push_cd(&mut batch, 15, 25, 0, 2000);
        
        assert_eq!(batch.len(), 2);
    }

    #[test]
    fn test_events_repr_trait_aos() {
        let mut batch = EventAoS::new();
        EventsRepr::push_cd(&mut batch, 10, 20, 1, 1000);
        EventsRepr::push_cd(&mut batch, 15, 25, 0, 2000);
        
        assert_eq!(batch.len(), 2);
        assert_eq!(batch.events[0].x, 10);
    }
}
