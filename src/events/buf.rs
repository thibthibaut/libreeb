//! Packed event representation: one `u32` per event plus one time mark per
//! run of equal timestamps.
//!
//! Layout from <https://tibo.space/posts/better-events/>. Each event is an
//! [`EventId`] that doubles as a row-major pixel index, and timestamps are
//! run-length encoded as [`TimeMark`]s. `4 + 8 / (events per timestamp)`
//! B/event: 4.75 on dense data, against 13 for [`EventSoA`](super::EventSoA).

use super::types::{EventCD, Trigger};
use crate::DecodeInto;

const COORD_BITS: u32 = 11;
const COORD_MASK: u32 = (1 << COORD_BITS) - 1;
const POL_SHIFT: u32 = 2 * COORD_BITS;
const PIXEL_MASK: u32 = (1 << POL_SHIFT) - 1;

/// A CD event without its timestamp, packed in a `u32`.
///
/// Bits 0–10 hold x, bits 11–21 hold y, bit 22 holds polarity and bits 23–31
/// are zero (reserved). Coordinates are truncated to 11 bits, so sensors up to
/// 2048×2048 are supported (the EVT2/EVT2.1/EVT3 coordinate range).
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EventId(pub u32);

impl EventId {
    /// Row stride of [`pixel`](Self::pixel): `pixel = y * STRIDE + x`.
    pub const STRIDE: u32 = 1 << COORD_BITS;

    /// Pack an event. Coordinates are masked to 11 bits so a corrupt
    /// out-of-range x can never spill into y or polarity.
    #[inline(always)]
    pub fn new(x: u16, y: u16, p: u8) -> Self {
        EventId(
            ((p != 0) as u32) << POL_SHIFT
                | (y as u32 & COORD_MASK) << COORD_BITS
                | (x as u32 & COORD_MASK),
        )
    }

    #[inline(always)]
    pub fn x(self) -> u16 {
        (self.0 & COORD_MASK) as u16
    }

    #[inline(always)]
    pub fn y(self) -> u16 {
        (self.0 >> COORD_BITS & COORD_MASK) as u16
    }

    #[inline(always)]
    pub fn p(self) -> u8 {
        (self.0 >> POL_SHIFT & 1) as u8
    }

    /// Row-major pixel index `y * 2048 + x`, usable directly as an array index.
    #[inline(always)]
    pub fn pixel(self) -> u32 {
        self.0 & PIXEL_MASK
    }

    /// Pixel index with polarity as the top bit: `p * 2048² + pixel`.
    /// Indexes a two-channel (OFF, ON) per-pixel array.
    #[inline(always)]
    pub fn pol_pixel(self) -> u32 {
        self.0
    }
}

/// Start of a run of events sharing one timestamp.
///
/// The run spans `ids[start..next.start]`, or `ids[start..]` for the last mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeMark {
    /// Timestamp in microseconds, relative to [`EventBuf::t_base`].
    pub t: u32,
    /// Index into [`EventBuf::ids`] of the first event of the run.
    pub start: u32,
}

/// Packed event buffer: [`EventId`]s plus run-length encoded timestamps.
///
/// Invariants: `marks` is empty iff `ids` is; `marks[0].start == 0`; starts
/// strictly increase (no empty run); consecutive marks have different `t`.
/// Time order is preserved exactly but not required: any change of `t`,
/// including a regression, starts a new run.
///
/// Timestamps are stored as `u32` offsets from `t_base`, the first event's
/// timestamp. One buffer therefore holds at most ~71.6 min (`u32::MAX` µs)
/// and no event earlier than its first; pushing outside that range panics.
/// Decoder batches span seconds, so only very long `append` chains get close.
#[derive(Debug, Clone, Default)]
pub struct EventBuf {
    t_base: u64,
    ids: Vec<EventId>,
    marks: Vec<TimeMark>,
    pub triggers: Vec<Trigger>,
}

/// Equal when they hold the same events and triggers, whatever their `t_base`.
impl PartialEq for EventBuf {
    fn eq(&self, other: &Self) -> bool {
        let runs = |b: &Self| {
            b.segments()
                .map(|(t, ids)| (t, ids.len()))
                .collect::<Vec<_>>()
        };
        self.ids == other.ids && self.triggers == other.triggers && runs(self) == runs(other)
    }
}

impl Eq for EventBuf {}

impl EventBuf {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(cap: usize) -> Self {
        EventBuf {
            ids: Vec::with_capacity(cap),
            ..Self::default()
        }
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// All events, in decode order.
    pub fn ids(&self) -> &[EventId] {
        &self.ids
    }

    /// Run starts, one per change of timestamp. `t` is relative to [`t_base`](Self::t_base).
    pub fn marks(&self) -> &[TimeMark] {
        &self.marks
    }

    /// Absolute timestamp (µs) that [`TimeMark::t`] offsets are relative to.
    /// Meaningless while the buffer is empty.
    pub fn t_base(&self) -> u64 {
        self.t_base
    }

    /// Iterate `(t, events)` runs, so per-timestamp work runs once per run.
    pub fn segments(&self) -> impl Iterator<Item = (u64, &[EventId])> + '_ {
        let ends = self
            .marks
            .iter()
            .skip(1)
            .map(|m| m.start as usize)
            .chain(std::iter::once(self.ids.len()));
        self.marks
            .iter()
            .zip(ends)
            .map(|(m, end)| (self.t_base + m.t as u64, &self.ids[m.start as usize..end]))
    }

    /// Absolute timestamp of the current (last) run.
    #[inline(always)]
    fn last_t(&self) -> Option<u64> {
        self.marks.last().map(|m| self.t_base + m.t as u64)
    }

    /// Open a new run if `t` differs from the current one. Call only right
    /// before pushing at least one id, so no run is ever empty.
    #[inline(always)]
    fn mark(&mut self, t: u64) {
        if self.last_t() != Some(t) {
            self.push_mark(t, self.ids.len());
        }
    }

    /// Push a run start; the first run of an empty buffer sets `t_base`.
    fn push_mark(&mut self, t: u64, start: usize) {
        if self.marks.is_empty() {
            self.t_base = t;
        }
        let t = t
            .checked_sub(self.t_base)
            .and_then(|dt| u32::try_from(dt).ok())
            .unwrap_or_else(|| {
                panic!(
                    "EventBuf: timestamp {t} µs outside t_base {} + u32::MAX µs (~71.6 min)",
                    self.t_base
                )
            });
        let start = u32::try_from(start).expect("EventBuf: run start exceeds u32::MAX events");
        self.marks.push(TimeMark { t, start });
    }

    /// Append all events from `other` into `self`, leaving `other` empty.
    ///
    /// # Panics
    /// If `other` holds timestamps outside `self`'s `t_base + u32::MAX` µs range.
    pub fn append(&mut self, other: &mut EventBuf) {
        let offset = self.ids.len();
        let other_base = other.t_base;
        let first = other.marks.first().map(|m| other_base + m.t as u64);
        let merge = first.is_some() && first == self.last_t();
        for m in other.marks.drain(..).skip(merge as usize) {
            self.push_mark(other_base + m.t as u64, m.start as usize + offset);
        }
        self.ids.append(&mut other.ids);
        self.triggers.append(&mut other.triggers);
    }

    /// Split at event index `at`. `self` keeps `[0..at)`, returns `[at..)`.
    /// Triggers stay with `self`, like [`EventSoA::split_off`](super::EventSoA::split_off).
    ///
    /// # Panics
    /// If `at > self.len()`.
    pub fn split_off(&mut self, at: usize) -> EventBuf {
        let ids = self.ids.split_off(at);
        let k = self.marks.partition_point(|m| (m.start as usize) < at);
        let mut marks = self.marks.split_off(k);
        // Marks moved to the tail all start at >= at, so `at` fits in u32 here.
        for m in &mut marks {
            m.start -= at as u32;
        }
        if !ids.is_empty() && marks.first().is_none_or(|m| m.start != 0) {
            // `at` falls inside run k-1: the tail starts with the rest of that run.
            marks.insert(
                0,
                TimeMark {
                    t: self.marks[k - 1].t,
                    start: 0,
                },
            );
        }
        EventBuf {
            t_base: self.t_base,
            ids,
            marks,
            triggers: Vec::new(),
        }
    }
}

impl DecodeInto for EventBuf {
    #[inline]
    fn reserve_cd(&mut self, additional: usize) {
        self.ids.reserve(additional);
    }

    #[inline]
    fn push_cd(&mut self, x: u16, y: u16, p: u8, t: u64) {
        self.mark(t);
        self.ids.push(EventId::new(x, y, p));
    }

    #[inline]
    fn push_trigger(&mut self, id: u8, p: u8, t: u64) {
        self.triggers.push(Trigger { id, p: p != 0, t });
    }

    #[inline]
    fn extend_from_soa(&mut self, x: &[u16], y: &[u16], p: &[u8], t: &[u64]) {
        // Two passes: a branchy scan over t for run starts, then a branch-free
        // (vectorizable) pack of the ids.
        let n = x.len().min(y.len()).min(p.len()).min(t.len());
        let base = self.ids.len();
        let mut last = self.last_t();
        for (i, &t) in t[..n].iter().enumerate() {
            if last != Some(t) {
                self.push_mark(t, base + i);
                last = Some(t);
            }
        }
        self.ids.extend(
            x[..n]
                .iter()
                .zip(&y[..n])
                .zip(&p[..n])
                .map(|((&x, &y), &p)| EventId::new(x, y, p)),
        );
    }

    #[inline]
    fn extend_constant_ypt(&mut self, xs: &[u16], y: u16, p: u8, t: u64) {
        if xs.is_empty() {
            return;
        }
        self.mark(t);
        let base = EventId::new(0, y, p).0;
        self.ids
            .extend(xs.iter().map(|&x| EventId(base | (x as u32 & COORD_MASK))));
    }

    #[inline]
    fn extend_masked_xs(&mut self, base_x: u16, y: u16, p: u8, t: u64, mut mask: u32) {
        if mask == 0 {
            return;
        }
        self.mark(t);
        self.ids.reserve(mask.count_ones() as usize);
        let base = EventId::new(0, y, p).0;
        while mask != 0 {
            let x = base_x as u32 + mask.trailing_zeros();
            mask &= mask - 1;
            self.ids.push(EventId(base | (x & COORD_MASK)));
        }
    }
}

impl EventBuf {
    /// Iterate events as unpacked [`EventCD`]s.
    pub fn iter_cd(&self) -> impl Iterator<Item = EventCD> + '_ {
        self.segments().flat_map(|(t, ids)| {
            ids.iter().map(move |id| EventCD {
                x: id.x(),
                y: id.y(),
                p: id.p(),
                t,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<EventCD> {
        // Repeated timestamps, a regression (30 after 40) and a return to an old t.
        [
            (1, 2, 1, 10),
            (3, 4, 0, 10),
            (5, 6, 1, 20),
            (2047, 2047, 1, 40),
            (0, 0, 0, 30),
            (7, 8, 1, 30),
            (9, 9, 0, 10),
        ]
        .into_iter()
        .map(|(x, y, p, t)| EventCD { x, y, p, t })
        .collect()
    }

    fn build(events: &[EventCD]) -> EventBuf {
        let mut buf = EventBuf::new();
        for e in events {
            buf.push_cd(e.x, e.y, e.p, e.t);
        }
        buf
    }

    fn check_invariants(buf: &EventBuf) {
        assert_eq!(buf.marks.is_empty(), buf.ids.is_empty());
        if let Some(first) = buf.marks.first() {
            assert_eq!(first.start, 0);
        }
        for w in buf.marks.windows(2) {
            assert!(w[0].start < w[1].start, "empty run: {:?}", buf.marks);
            assert_ne!(w[0].t, w[1].t, "unmerged runs: {:?}", buf.marks);
        }
        if let Some(last) = buf.marks.last() {
            assert!((last.start as usize) < buf.ids.len());
        }
    }

    #[test]
    fn event_id_layout() {
        let id = EventId::new(5, 3, 1);
        assert_eq!(id.pixel(), 3 * EventId::STRIDE + 5);
        assert_eq!(id.pol_pixel(), (1 << 22) | id.pixel());
        assert_eq!((id.x(), id.y(), id.p()), (5, 3, 1));
        assert_eq!(EventId::new(0, 0, 7).p(), 1);
    }

    #[test]
    fn out_of_range_x_does_not_spill() {
        let id = EventId::new(2048 + 5, 3, 0);
        assert_eq!((id.x(), id.y(), id.p()), (5, 3, 0));
        let mut buf = EventBuf::new();
        buf.extend_masked_xs(2047, 3, 0, 0, 0b11);
        assert_eq!(buf.ids[1].y(), 3);
        assert_eq!(buf.ids[1].p(), 0);
    }

    #[test]
    fn push_round_trip() {
        let events = sample();
        let buf = build(&events);
        check_invariants(&buf);
        assert_eq!(buf.iter_cd().collect::<Vec<_>>(), events);
        // Runs: 10 | 20 | 40 | 30 | 10
        assert_eq!(buf.marks.len(), 5);
    }

    #[test]
    fn bulk_paths_match_push_cd() {
        let events = sample();
        let mut soa = EventBuf::new();
        let (x, y, p, t): (Vec<_>, Vec<_>, Vec<_>, Vec<_>) =
            events.iter().map(|e| (e.x, e.y, e.p, e.t)).fold(
                Default::default(),
                |(mut a, mut b, mut c, mut d), (x, y, p, t)| {
                    a.push(x);
                    b.push(y);
                    c.push(p);
                    d.push(t);
                    (a, b, c, d)
                },
            );
        soa.extend_from_soa(&x, &y, &p, &t);
        assert_eq!(soa, build(&events));

        let mut grouped = EventBuf::new();
        grouped.extend_constant_ypt(&[], 1, 1, 5); // empty group: no run
        grouped.extend_masked_xs(0, 1, 1, 5, 0); // empty mask: no run
        assert!(grouped.marks.is_empty());
        grouped.extend_constant_ypt(&[4, 6], 1, 1, 5);
        grouped.extend_masked_xs(32, 1, 1, 5, 0b101);
        grouped.extend_masked_xs(0, 2, 0, 6, 0b1);
        check_invariants(&grouped);
        let expected = build(&[
            EventCD {
                x: 4,
                y: 1,
                p: 1,
                t: 5,
            },
            EventCD {
                x: 6,
                y: 1,
                p: 1,
                t: 5,
            },
            EventCD {
                x: 32,
                y: 1,
                p: 1,
                t: 5,
            },
            EventCD {
                x: 34,
                y: 1,
                p: 1,
                t: 5,
            },
            EventCD {
                x: 0,
                y: 2,
                p: 0,
                t: 6,
            },
        ]);
        assert_eq!(grouped, expected);
    }

    #[test]
    fn split_then_append_is_identity_at_every_index() {
        let events = sample();
        let original = build(&events);
        for at in 0..=events.len() {
            let mut head = original.clone();
            let mut tail = head.split_off(at);
            check_invariants(&head);
            check_invariants(&tail);
            assert_eq!(head.iter_cd().collect::<Vec<_>>(), events[..at]);
            assert_eq!(tail.iter_cd().collect::<Vec<_>>(), events[at..]);
            head.append(&mut tail);
            assert!(tail.is_empty() && tail.marks.is_empty());
            assert_eq!(head, original, "split/append at {at}");
        }
    }

    #[test]
    fn append_merges_equal_timestamps() {
        let mut a = build(&[EventCD {
            x: 1,
            y: 1,
            p: 1,
            t: 7,
        }]);
        let mut b = build(&[
            EventCD {
                x: 2,
                y: 2,
                p: 0,
                t: 7,
            },
            EventCD {
                x: 3,
                y: 3,
                p: 0,
                t: 8,
            },
        ]);
        a.append(&mut b);
        check_invariants(&a);
        assert_eq!(
            a.segments()
                .map(|(t, ids)| (t, ids.len()))
                .collect::<Vec<_>>(),
            [(7, 2), (8, 1)]
        );
    }

    #[test]
    fn append_rebases_other_and_eq_ignores_base() {
        let ev = |t| EventCD {
            x: 1,
            y: 1,
            p: 1,
            t,
        };
        let big = 5_000_000_000; // > u32::MAX µs, so absolute t would not fit a u32
        let mut a = build(&[ev(big)]);
        let mut b = build(&[ev(big + 3), ev(big + 9)]);
        a.append(&mut b);
        check_invariants(&a);
        assert_eq!(a.t_base(), big);
        assert_eq!(a, build(&[ev(big), ev(big + 3), ev(big + 9)]));

        // Same events, different t_base: an emptied buffer keeps a stale base.
        let mut c = build(&[ev(1)]);
        let _ = c.split_off(0);
        c.push_cd(1, 1, 1, big);
        assert_eq!(c, build(&[ev(big)]));
    }

    #[test]
    fn max_span_fits() {
        let mut buf = EventBuf::new();
        buf.push_cd(0, 0, 0, 100);
        buf.push_cd(0, 0, 0, 100 + u32::MAX as u64);
        assert_eq!(buf.iter_cd().last().unwrap().t, 100 + u32::MAX as u64);
    }

    #[test]
    #[should_panic(expected = "outside t_base")]
    fn span_over_u32_panics() {
        let mut buf = EventBuf::new();
        buf.push_cd(0, 0, 0, 100);
        buf.push_cd(0, 0, 0, 101 + u32::MAX as u64);
    }

    #[test]
    #[should_panic(expected = "outside t_base")]
    fn timestamp_before_base_panics() {
        let mut buf = EventBuf::new();
        buf.push_cd(0, 0, 0, 100);
        buf.push_cd(0, 0, 0, 99);
    }

    #[test]
    #[should_panic(expected = "outside t_base")]
    fn append_over_u32_span_panics() {
        let mut a = build(&[EventCD {
            x: 0,
            y: 0,
            p: 0,
            t: 0,
        }]);
        let mut b = build(&[EventCD {
            x: 0,
            y: 0,
            p: 0,
            t: 1 << 33,
        }]);
        a.append(&mut b);
    }
}
