//! Array-of-Structures (AoS) event representation.

use super::types::{EventCD, Trigger};
use crate::DecodeInto;

/// Array-of-Structures (AoS) event batch representation.
///
/// Stores events as a vector of [`EventCD`] structs. This layout is simpler
/// and more intuitive than SoA, but may be less cache-efficient for bulk
/// operations. Use this when you need to work with individual events or when
/// interoperating with code that expects `Vec<EventCD>`.
#[derive(Debug, Clone)]
pub struct EventAoS {
    pub events: Vec<EventCD>,
    pub triggers: Vec<Trigger>,
}

impl EventAoS {
    pub fn new() -> Self {
        EventAoS {
            events: Vec::new(),
            triggers: Vec::new(),
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
            triggers: Vec::new(),
        }
    }

    pub fn push(&mut self, evt: EventCD) {
        self.events.push(evt);
    }

    pub fn append(&mut self, other: &mut EventAoS) {
        self.events.append(&mut other.events);
        self.triggers.append(&mut other.triggers);
    }

    pub fn split_off(&mut self, at: usize) -> EventAoS {
        EventAoS {
            events: self.events.split_off(at),
            triggers: Vec::new(), // triggers stay with first half
        }
    }
}

impl Default for EventAoS {
    fn default() -> Self {
        Self::new()
    }
}

impl DecodeInto for EventAoS {
    #[inline]
    fn push_cd(&mut self, x: u16, y: u16, p: u8, t: u64) {
        self.events.push(EventCD { x, y, p, t });
    }

    #[inline]
    fn push_trigger(&mut self, id: u8, p: u8, t: u64) {
        self.triggers.push(Trigger { id, p: p != 0, t });
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
}

impl<'a> IntoIterator for &'a EventAoS {
    type Item = &'a EventCD;
    type IntoIter = std::slice::Iter<'a, EventCD>;

    fn into_iter(self) -> Self::IntoIter {
        self.events.iter()
    }
}
