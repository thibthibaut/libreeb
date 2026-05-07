use crate::{DecoderOutput, EventBatch, SliceBy};
use std::io::Read;

const READ_BUFFER_SIZE: usize = 262144;
const _: () = {
    assert!(READ_BUFFER_SIZE.is_multiple_of(8));
};

#[repr(align(64))]
struct AlignedBuffer([u8; READ_BUFFER_SIZE]);

// ── Internal format-specific decoder trait ───────────────────────────────────

/// Internal trait implemented by format-specific decoders (Evt2, Evt21, Evt3).
///
/// The `decode` method is generic over `O: DecoderOutput`, allowing the same
/// decoder implementation to produce different output types depending on what
/// the caller needs (SoA batches, AoS vectors, histograms, etc.).
///
/// This trait is not part of the public API.
pub(crate) trait FormatDecoder: Send + Sync {
    type RawEventType: bytemuck::Pod + Copy;
    fn new() -> Self;

    /// Decode raw events into the output, respecting budget limits.
    ///
    /// Returns the number of raw events consumed from the input (may be less
    /// than `raw_events.len()` if a budget limit was reached).
    ///
    /// - `end_time`: stop when decoded timestamp >= this (u64::MAX = no limit)
    /// - `max_events`: stop after output contains this many CD events (usize::MAX = no limit)
    fn decode<O: DecoderOutput>(
        &mut self,
        raw_events: &[Self::RawEventType],
        output: &mut O,
        end_time: u64,
        max_events: usize,
    ) -> usize;

    /// Current decoder timestamp.
    fn current_time(&self) -> u64;
}

// ── Object-safe wrapper for type-erased decoding ─────────────────────────────

/// Object-safe trait wrapping a FormatDecoder + Read source.
/// Parameterized by output type `O` for compile-time dispatch.
pub(crate) trait DecoderStream<O: DecoderOutput>: Send + Sync {
    fn next_batch(&mut self, end_time: u64, max_events: usize) -> Option<O>;
    fn current_time(&self) -> u64;
}

/// Concrete reader for a specific format. Holds the byte reader + format decoder.
pub(crate) struct FormatReader<R: Read, D: FormatDecoder> {
    reader: R,
    decoder: D,
    buffer: AlignedBuffer,
    buffer_len: usize,
    buffer_cursor: usize,
}

impl<R: Read, D: FormatDecoder> FormatReader<R, D> {
    pub fn new(reader: R, decoder: D) -> Self {
        FormatReader {
            reader,
            decoder,
            buffer: AlignedBuffer([0; READ_BUFFER_SIZE]),
            buffer_len: 0,
            buffer_cursor: 0,
        }
    }

    /// Refill the buffer from the reader.
    /// Moves any unconsumed bytes to the front, then fills the rest.
    /// Returns true if we got new data, false on EOF.
    fn refill(&mut self) -> bool {
        // Move unconsumed data to front
        if self.buffer_cursor > 0 && self.buffer_cursor < self.buffer_len {
            self.buffer
                .0
                .copy_within(self.buffer_cursor..self.buffer_len, 0);
            self.buffer_len -= self.buffer_cursor;
            self.buffer_cursor = 0;
        } else if self.buffer_cursor >= self.buffer_len {
            self.buffer_len = 0;
            self.buffer_cursor = 0;
        }

        // Fill remaining space
        loop {
            let bytes_read = self
                .reader
                .read(&mut self.buffer.0[self.buffer_len..])
                .unwrap_or(0);

            if bytes_read == 0 {
                return self.buffer_len > self.buffer_cursor;
            }

            self.buffer_len += bytes_read;

            // Keep reading until buffer is full or EOF
            if self.buffer_len == READ_BUFFER_SIZE {
                return true;
            }
        }
    }
}

impl<R: Read + Send + Sync, D: FormatDecoder, O: DecoderOutput> DecoderStream<O>
    for FormatReader<R, D>
{
    fn next_batch(&mut self, end_time: u64, max_events: usize) -> Option<O> {
        let word_size = std::mem::size_of::<D::RawEventType>();
        let mut output = O::default();

        loop {
            // If we have data in the buffer, decode from it
            if self.buffer_cursor < self.buffer_len {
                let usable = self.buffer_len - self.buffer_cursor;
                let aligned = usable - (usable % word_size);

                if aligned > 0 {
                    let slice = &self.buffer.0[self.buffer_cursor..self.buffer_cursor + aligned];
                    let events: &[D::RawEventType] = bytemuck::cast_slice(slice);

                    let consumed =
                        self.decoder
                            .decode(events, &mut output, end_time, max_events);

                    self.buffer_cursor += consumed * word_size;

                    // If decoder consumed less than available, budget was exhausted.
                    // Return whatever we have (might be empty = "window is empty").
                    if consumed < events.len() {
                        return Some(output);
                    }

                    // If budget is reached, yield
                    if output.len() >= max_events {
                        return Some(output);
                    }
                }
            }

            // Need more data — refill the buffer
            if !self.refill() {
                // EOF — return whatever we have (None = true end of stream)
                return if output.is_empty() { None } else { Some(output) };
            }

            // If after refill we still can't form a complete word, we're done
            let usable = self.buffer_len - self.buffer_cursor;
            if usable < word_size {
                return if output.is_empty() { None } else { Some(output) };
            }
        }
    }

    fn current_time(&self) -> u64 {
        self.decoder.current_time()
    }
}

// ── Public EventDecoder ──────────────────────────────────────────────────────

/// Decodes raw event bytes into output batches of type `O`. Supports optional slicing.
///
/// This is the main decoder type. Create one from any `Read` source and an
/// event format type, then iterate to get decoded event batches.
///
/// The output type `O` defaults to [`EventBatch`] (SoA layout) but can be any
/// type implementing [`DecoderOutput`]. This is determined at compile time,
/// allowing the compiler to fully optimize for the chosen output layout.
///
/// # Slicing
///
/// By default, each call to `next()` decodes one buffer's worth of data
/// (~256KB). To get time- or count-bounded slices instead, call
/// [`set_slice_by`](Self::set_slice_by). The slice condition can be changed
/// at any time between calls to `next()`.
///
/// # Example
///
/// ```rust,ignore
/// use libreeb::{EventDecoder, SliceBy};
///
/// let mut decoder = EventDecoder::new(reader, RawEventType::Evt3)?;
/// decoder.set_slice_by(Some(SliceBy::Time(33_333)));
/// while let Some(batch) = decoder.next() {
///     process(&batch);
///     decoder.set_slice_by(Some(SliceBy::Time(10_000))); // can change mid-stream
/// }
/// ```
pub struct EventDecoder<O: DecoderOutput = EventBatch> {
    stream: Box<dyn DecoderStream<O>>,
    slice_by: Option<SliceBy>,
    slice_anchor: Option<u64>,
}

impl<O: DecoderOutput> EventDecoder<O> {
    /// Create a new decoder from a boxed stream (internal use).
    pub(crate) fn from_stream(stream: Box<dyn DecoderStream<O>>) -> Self {
        EventDecoder {
            stream,
            slice_by: None,
            slice_anchor: None,
        }
    }

    /// Set or clear the slicing condition.
    ///
    /// Takes effect on the next call to [`Iterator::next`].
    /// Pass `None` to disable slicing (returns full chunk-sized batches).
    pub fn set_slice_by(&mut self, slice_by: Option<SliceBy>) {
        self.slice_by = slice_by;
        self.slice_anchor = None;
    }

    /// Compute (end_time, max_events) for the current slice.
    fn budget(&self) -> (u64, usize) {
        match self.slice_by {
            None => (u64::MAX, usize::MAX),
            Some(SliceBy::Time(us)) => {
                let end = self
                    .slice_anchor
                    .map_or(u64::MAX, |anchor| anchor.saturating_add(us));
                (end, usize::MAX)
            }
            Some(SliceBy::Count(n)) => (u64::MAX, n),
            Some(SliceBy::Both(us, n)) => {
                let end = self
                    .slice_anchor
                    .map_or(u64::MAX, |anchor| anchor.saturating_add(us));
                (end, n)
            }
        }
    }
}

impl<O: DecoderOutput> Iterator for EventDecoder<O> {
    type Item = O;

    fn next(&mut self) -> Option<O> {
        if self.slice_by.is_none() {
            // No slicing: decode one full chunk
            let batch = self.stream.next_batch(u64::MAX, usize::MAX)?;
            return Some(batch);
        }

        loop {
            // First call for this slice: anchor to the decoder's current time
            if self.slice_anchor.is_none() {
                self.slice_anchor = Some(self.stream.current_time());
            }

            let (end_time, max_events) = self.budget();
            let batch = self.stream.next_batch(end_time, max_events)?;

            if batch.is_empty() {
                // Time window contained no events (stream jumped past end_time).
                // Advance anchor to the stream's actual position and retry.
                self.slice_anchor = None;
                continue;
            }

            // After yielding, reset anchor for the next slice
            self.slice_anchor = None;
            return Some(batch);
        }
    }
}
