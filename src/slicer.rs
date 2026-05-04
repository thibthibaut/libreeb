use crate::EventBatch;

/// Slice configuration options.
#[derive(Clone, Copy, Debug)]
pub enum SliceBy {
    /// Slice by time in microseconds.
    Time(u64),
    /// Slice by count of events.
    Count(usize),
    /// Slice by whichever limit is reached first: time or count.
    Both(u64, usize),
}

/// Splits a stream of [`EventBatch`]es into fixed-size slices.
///
/// For time-based slicing, binary search on the (sorted) timestamp array is
/// used to locate the split boundary efficiently.
///
/// ## Dynamic configuration
///
/// The slicing configuration can be changed between frames by calling
/// [`SliceIter::set_slice_by`]. The new setting takes effect on the very next
/// call to [`Iterator::next`]; the new window / count is anchored to the first
/// event of that slice, exactly as if a fresh `SliceIter` had been created.
///
/// ```rust,ignore
/// let mut slicer = SliceIter::new(reader.read_events(), SliceBy::Time(33_333));
///
/// // Use while-let so the &mut borrow from .next() is released before
/// // calling set_slice_by() on the same slicer.
/// while let Some(batch) = slicer.next() {
///     render(&batch);
///     if user_changed_window {
///         slicer.set_slice_by(SliceBy::Time(new_window_us));
///     }
/// }
/// ```
pub struct SliceIter<I: Iterator<Item = EventBatch>> {
    inner: I,
    /// Leftover events from the previous slice that belong to the next one.
    overflow: EventBatch,
    slice_by: SliceBy,
    done: bool,
}

impl<I: Iterator<Item = EventBatch>> SliceIter<I> {
    pub fn new(inner: I, slice_by: SliceBy) -> Self {
        SliceIter {
            inner,
            overflow: EventBatch::new(),
            slice_by,
            done: false,
        }
    }

    /// Update the slicing configuration.
    ///
    /// Takes effect on the next call to [`Iterator::next`].
    /// The new window / count is anchored to the first event of that slice,
    /// just as it would be for a freshly constructed `SliceIter`.
    pub fn set_slice_by(&mut self, slice_by: SliceBy) {
        self.slice_by = slice_by;
    }

    /// Mutable reference to the inner iterator.
    ///
    /// Useful when the inner iterator carries additional state (e.g.
    /// [`CameraReader`](crate::camera::CameraReader)) that needs to be
    /// accessed between slices (e.g. to apply bias changes).
    pub fn inner_mut(&mut self) -> &mut I {
        &mut self.inner
    }
}

impl<I: Iterator<Item = EventBatch>> Iterator for SliceIter<I> {
    type Item = EventBatch;

    fn next(&mut self) -> Option<EventBatch> {
        if self.done {
            return None;
        }

        // Start with whatever was left over from the previous slice.
        let mut acc = std::mem::take(&mut self.overflow);

        match self.slice_by {
            // ── Count ────────────────────────────────────────────────────────
            SliceBy::Count(n) => {
                loop {
                    if acc.len() >= n {
                        self.overflow = acc.split_off(n);
                        return Some(acc);
                    }
                    match self.inner.next() {
                        Some(batch) => acc.append(batch),
                        None => {
                            self.done = true;
                            return if acc.is_empty() { None } else { Some(acc) };
                        }
                    }
                }
            }

            // ── Time ─────────────────────────────────────────────────────────
            SliceBy::Time(micros) => {
                // Ensure we have at least one event to anchor the window.
                while acc.is_empty() {
                    match self.inner.next() {
                        Some(batch) => acc = batch,
                        None => {
                            self.done = true;
                            return None;
                        }
                    }
                }

                let end_t = acc.t[0] + micros;

                loop {
                    // Binary search: timestamps are non-decreasing.
                    let split = acc.t.partition_point(|&t| t < end_t);
                    if split < acc.len() {
                        self.overflow = acc.split_off(split);
                        return Some(acc);
                    }
                    match self.inner.next() {
                        Some(batch) => acc.append(batch),
                        None => {
                            self.done = true;
                            return if acc.is_empty() { None } else { Some(acc) };
                        }
                    }
                }
            }

            // ── Both ─────────────────────────────────────────────────────────
            SliceBy::Both(micros, n) => {
                while acc.is_empty() {
                    match self.inner.next() {
                        Some(batch) => acc = batch,
                        None => {
                            self.done = true;
                            return None;
                        }
                    }
                }

                let end_t = acc.t[0] + micros;

                loop {
                    let time_split = acc.t.partition_point(|&t| t < end_t);
                    // Stop at whichever limit is reached first.
                    if time_split < acc.len() || acc.len() >= n {
                        let split = time_split.min(n);
                        self.overflow = acc.split_off(split);
                        return Some(acc);
                    }
                    match self.inner.next() {
                        Some(batch) => acc.append(batch),
                        None => {
                            self.done = true;
                            return if acc.is_empty() { None } else { Some(acc) };
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod slicer_tests {
    use super::*;
    use crate::RawFileReader;
    use rstest::rstest;
    use std::path::Path;

    /// Count total events in a file (used as a reference for the slicing tests).
    fn total_events(path: &str) -> usize {
        let mut reader = RawFileReader::new(Path::new(path)).unwrap();
        reader.read_events().map(|b| b.len()).sum()
    }

    // ── SliceBy::Count ───────────────────────────────────────────────────────

    /// Every slice except the last must contain exactly `count` events.
    /// The sum across all slices must equal the total event count in the file.
    #[rstest]
    #[case("data/gen4_evt3_hand.raw", 50_000)]
    #[case("data/claque_doigt_evt21.raw", 50_000)]
    #[case("data/blinking_leds.raw", 50_000)]
    fn slice_by_count(#[case] path: &str, #[case] count: usize) {
        let total = total_events(path);
        let mut reader = RawFileReader::new(Path::new(path)).unwrap();
        let mut slicer = SliceIter::new(reader.read_events(), SliceBy::Count(count));

        let mut sliced_total = 0;
        let mut prev: Option<EventBatch> = None;

        for slice in &mut slicer {
            assert!(!slice.is_empty());

            // Every full slice must have exactly `count` events; partial slices
            // only appear at the very end of the stream.
            if let Some(p) = prev.replace(slice) {
                assert_eq!(p.len(), count, "mid-stream slice has wrong length");
            }

            sliced_total += prev.as_ref().unwrap().len();
        }

        assert_eq!(sliced_total, total, "event count changed after slicing");
    }

    // ── SliceBy::Time ────────────────────────────────────────────────────────

    /// Every slice must:
    /// - be non-empty,
    /// - have all events within [first_t, first_t + window),
    /// - have non-decreasing timestamps internally.
    ///
    /// Windows are anchored to their first event, so gaps in the event stream
    /// do not produce empty slices.
    ///
    /// The total event count must be preserved.
    #[rstest]
    #[case("data/gen4_evt3_hand.raw", 50_000)]
    #[case("data/claque_doigt_evt21.raw", 50_000)]
    #[case("data/blinking_leds.raw", 50_000)]
    fn slice_by_time(#[case] path: &str, #[case] window_us: u64) {
        let total = total_events(path);
        let mut reader = RawFileReader::new(Path::new(path)).unwrap();
        let mut slicer = SliceIter::new(reader.read_events(), SliceBy::Time(window_us));

        let mut sliced_total = 0;
        let mut prev_last_t: Option<u64> = None;

        for slice in &mut slicer {
            assert!(!slice.is_empty());

            let first_t = slice.t[0];
            let last_t = *slice.t.last().unwrap();

            // All events are inside [first_t, first_t + window)
            assert!(
                last_t < first_t + window_us,
                "slice spans {last_t} - {first_t} = {} µs, exceeds window {window_us}",
                last_t - first_t,
            );

            // Timestamps are non-decreasing within the slice
            assert!(
                slice.t.windows(2).all(|w| w[0] <= w[1]),
                "timestamps not sorted within slice"
            );

            // The slice starts no earlier than the previous slice ended
            if let Some(prev_t) = prev_last_t {
                assert!(
                    first_t >= prev_t,
                    "slice boundary went backwards: {first_t} < {prev_t}"
                );
            }
            prev_last_t = Some(last_t);

            sliced_total += slice.len();
        }

        assert_eq!(sliced_total, total, "event count changed after time-slicing");
    }

    // ── SliceBy::Both ────────────────────────────────────────────────────────

    /// Every slice (including the last) must respect both limits simultaneously:
    /// at most `count` events AND a time span strictly less than `window_us`.
    ///
    /// The total event count must be preserved.
    #[rstest]
    #[case("data/gen4_evt3_hand.raw", 50_000, 50_000)]
    #[case("data/claque_doigt_evt21.raw", 50_000, 50_000)]
    #[case("data/blinking_leds.raw", 50_000, 50_000)]
    fn slice_by_both(#[case] path: &str, #[case] window_us: u64, #[case] count: usize) {
        let total = total_events(path);
        let mut reader = RawFileReader::new(Path::new(path)).unwrap();
        let mut slicer = SliceIter::new(reader.read_events(), SliceBy::Both(window_us, count));

        let mut sliced_total = 0;

        for slice in &mut slicer {
            assert!(!slice.is_empty());

            // Count limit
            assert!(
                slice.len() <= count,
                "slice has {} events, exceeds count limit {count}",
                slice.len()
            );

            // Time limit
            let first_t = slice.t[0];
            let last_t = *slice.t.last().unwrap();
            assert!(
                last_t < first_t + window_us,
                "slice spans {} µs, exceeds window {window_us}",
                last_t - first_t,
            );

            sliced_total += slice.len();
        }

        assert_eq!(sliced_total, total, "event count changed after both-slicing");
    }

    // ── set_slice_by ─────────────────────────────────────────────────────────

    /// Changing the slice configuration mid-stream must preserve the total
    /// event count and take effect from the very next slice.
    ///
    /// Note: `set_slice_by` must be called between explicit `.next()` calls
    /// (i.e. via `while let`), not inside a `for … in &mut slicer` loop,
    /// because the `for` loop holds the mutable borrow for the entire body.
    #[test]
    fn set_slice_by_mid_stream() {
        let path = "data/gen4_evt3_hand.raw";
        let total = total_events(path);

        let mut reader = RawFileReader::new(Path::new(path)).unwrap();
        let mut slicer = SliceIter::new(reader.read_events(), SliceBy::Time(50_000));

        let mut sliced_total = 0;
        let mut frame_nr = 0usize;

        // Use while-let so the &mut borrow from .next() is released before
        // we call set_slice_by() on the same slicer.
        while let Some(slice) = slicer.next() {
            sliced_total += slice.len();
            frame_nr += 1;

            // Switch to a tighter window after the first slice.
            if frame_nr == 1 {
                slicer.set_slice_by(SliceBy::Time(10_000));
            }
        }

        assert_eq!(sliced_total, total, "event count changed after mid-stream set_slice_by");
    }
}
