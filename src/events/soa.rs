//! Structure-of-Arrays (SoA) event representation.

#[cfg(all(not(target_arch = "wasm32"), feature = "python"))]
use pyo3::prelude::*;

use super::types::{EventCD, Trigger};
use crate::DecodeInto;

/// Structure-of-Arrays (SoA) event batch representation.
///
/// Stores events in separate vectors for each field (x, y, p, t). This layout
/// is cache-friendly for bulk operations and provides excellent memory locality
/// for sequential processing.
///
/// This is the default output format for decoders, re-exported as [`EventBatch`].
#[cfg_attr(all(not(target_arch = "wasm32"), feature = "python"), pyclass)]
#[derive(Debug, Clone)]
pub struct EventSoA {
    pub x: Vec<u16>,
    pub y: Vec<u16>,
    pub p: Vec<u8>,
    pub t: Vec<u64>,
    pub triggers: Vec<Trigger>,
}

impl EventSoA {
    pub fn new() -> Self {
        EventSoA {
            x: Vec::new(),
            y: Vec::new(),
            p: Vec::new(),
            t: Vec::new(),
            triggers: Vec::new(),
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
            triggers: Vec::new(),
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
    pub fn append(&mut self, other: &mut EventSoA) {
        self.x.append(&mut other.x);
        self.y.append(&mut other.y);
        self.p.append(&mut other.p);
        self.t.append(&mut other.t);
        self.triggers.append(&mut other.triggers);
    }

    /// Split at index `at`. `self` retains `[0..at)`, returns `[at..)`.
    pub fn split_off(&mut self, at: usize) -> EventSoA {
        EventSoA {
            x: self.x.split_off(at),
            y: self.y.split_off(at),
            p: self.p.split_off(at),
            t: self.t.split_off(at),
            triggers: Vec::new(), // triggers stay with first half
        }
    }
}

impl Default for EventSoA {
    fn default() -> Self {
        Self::new()
    }
}

impl DecodeInto for EventSoA {
    #[inline]
    fn reserve_cd(&mut self, additional: usize) {
        self.x.reserve(additional);
        self.y.reserve(additional);
        self.p.reserve(additional);
        self.t.reserve(additional);
    }

    #[inline]
    fn push_cd(&mut self, x: u16, y: u16, p: u8, t: u64) {
        self.x.push(x);
        self.y.push(y);
        self.p.push(p);
        self.t.push(t);
    }

    #[inline]
    fn push_trigger(&mut self, id: u8, p: u8, t: u64) {
        self.triggers.push(Trigger { id, p: p != 0, t });
    }

    #[inline]
    fn extend_from_soa(&mut self, x: &[u16], y: &[u16], p: &[u8], t: &[u64]) {
        self.reserve_cd(x.len());
        self.x.extend_from_slice(x);
        self.y.extend_from_slice(y);
        self.p.extend_from_slice(p);
        self.t.extend_from_slice(t);
    }

    #[inline]
    fn extend_constant_ypt(&mut self, xs: &[u16], y: u16, p: u8, t: u64) {
        self.reserve_cd(xs.len());
        self.x.extend_from_slice(xs);
        let new_len = self.y.len() + xs.len();
        self.y.resize(new_len, y);
        self.p.resize(new_len, p);
        self.t.resize(new_len, t);
    }

    #[inline]
    fn extend_masked_xs(&mut self, base_x: u16, y: u16, p: u8, t: u64, mut mask: u32) {
        self.reserve_cd(mask.count_ones() as usize);
        while mask != 0 {
            let bit = mask.trailing_zeros() as u16;
            mask &= mask - 1;
            self.x.push(base_x + bit);
            self.y.push(y);
            self.p.push(p);
            self.t.push(t);
        }
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
