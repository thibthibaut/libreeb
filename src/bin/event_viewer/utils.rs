//! Sensor-time sliding window for event-based data.
//!
//! [`TimeWindow`] evicts items by sensor time (event timestamps in µs) rather
//! than wall-clock time, which keeps it correct across file loops and pauses.
//! A count cap is enforced as a hard safeguard against very high event rates.

use std::collections::VecDeque;

// ── Timestamped trait ─────────────────────────────────────────────────────────

/// Any item that carries a sensor timestamp (µs).
pub trait Timestamped {
    fn timestamp_us(&self) -> u64;
}

// ── EventBatch ────────────────────────────────────────────────────────────────

impl Timestamped for libreeb::EventBatch {
    /// Use the first event's timestamp as the slice anchor.
    /// Returns 0 for empty batches (handled gracefully by clock-reset detection).
    fn timestamp_us(&self) -> u64 {
        self.t.first().copied().unwrap_or(0)
    }
}

// ── RateSample ────────────────────────────────────────────────────────────────

/// One event-rate measurement anchored to a sensor timestamp.
pub struct RateSample {
    /// Sensor time (µs) of the first event in the batch that produced this sample.
    pub t: u64,
    /// Events per second.
    pub rate: f64,
}

impl Timestamped for RateSample {
    fn timestamp_us(&self) -> u64 {
        self.t
    }
}

// ── TimeWindow ────────────────────────────────────────────────────────────────

/// Sensor-time sliding window with a time cap and a count cap.
///
/// **Ordering**: front = oldest, back = newest.
///
/// Every [`push`](TimeWindow::push) performs four steps in order:
///
/// 1. **Clock-reset guard** — if the incoming timestamp is more than 1 s
///    *before* the current newest, the buffer is cleared.  This handles file
///    looping where the sensor clock resets to zero.
/// 2. **Append** to the back.
/// 3. **Time eviction** — pop from the front while
///    `oldest.t < (newest.t − time_cap_us)`.
/// 4. **Count safeguard** — pop from the front while `len > count_cap`.
///    Items evicted by the count cap increment [`dropped_count`](TimeWindow::dropped_count).
pub struct TimeWindow<T: Timestamped> {
    items: VecDeque<T>,
    time_cap_us: u64,
    count_cap: usize,
    /// Monotonically increasing count of items dropped by the count cap.
    dropped_count: u64,
}

impl<T: Timestamped> TimeWindow<T> {
    pub fn new(time_cap_us: u64, count_cap: usize) -> Self {
        TimeWindow {
            items: VecDeque::new(),
            time_cap_us,
            count_cap,
            dropped_count: 0,
        }
    }

    pub fn push(&mut self, item: T) {
        let t = item.timestamp_us();

        // 1. Clock-reset guard (e.g. file loop).
        if let Some(newest) = self.newest_t() {
            if t < newest.saturating_sub(1_000_000) {
                self.items.clear();
            }
        }

        // 2. Append.
        self.items.push_back(item);

        // 3. Time eviction from front (oldest first).
        let cutoff = t.saturating_sub(self.time_cap_us);
        while self
            .items
            .front()
            .map(|e| e.timestamp_us() < cutoff)
            .unwrap_or(false)
        {
            self.items.pop_front();
        }

        // 4. Count safeguard from front (oldest first).
        while self.items.len() > self.count_cap {
            self.items.pop_front();
            self.dropped_count += 1;
        }
    }

    /// Iterate oldest → newest.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.items.iter()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Total items evicted by the count cap since construction (monotonically increasing).
    pub fn dropped_count(&self) -> u64 {
        self.dropped_count
    }

    /// Sensor timestamp of the newest (most recently pushed) item.
    pub fn newest_t(&self) -> Option<u64> {
        self.items.back().map(|e| e.timestamp_us())
    }
    /// Sensor timestamp of the oldest (front) item.
    pub fn oldest_t(&self) -> Option<u64> {
        self.items.front().map(|e| e.timestamp_us())
    }
    /// Reference to the newest item.
    pub fn newest(&self) -> Option<&T> {
        self.items.back()
    }
}
