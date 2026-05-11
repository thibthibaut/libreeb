use crate::{DecodeInto, EventBatch, SliceBy};
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
/// The `decode` method is generic over `O: DecodeInto`, allowing the same
/// decoder implementation to produce different output types depending on what
/// the caller needs (SoA batches, AoS vectors, histograms, etc.).
///
/// This trait is not part of the public API.
pub(crate) trait RawFormatDecoder: Send + Sync {
    type RawEventType: bytemuck::Pod + Copy;
    fn new() -> Self;

    /// Decode raw events into the output, respecting budget limits.
    ///
    /// Returns the number of raw events consumed from the input (may be less
    /// than `raw_events.len()` if a budget limit was reached).
    ///
    /// - `end_time`: stop when decoded timestamp >= this (u64::MAX = no limit)
    /// - `max_events`: stop after output contains this many CD events (usize::MAX = no limit)
    fn decode<O: DecodeInto>(
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

/// Object-safe trait wrapping a RawFormatDecoder + Read source.
/// Parameterized by output type `O` for compile-time dispatch.
pub(crate) trait DecoderStream<O: DecodeInto>: Send + Sync {
    fn next_batch(&mut self, end_time: u64, max_events: usize) -> Option<O>;
    fn current_time(&self) -> u64;
}

/// Concrete reader for a specific format. Holds the byte reader + format decoder.
pub(crate) struct BufferedDecoder<R: Read, D: RawFormatDecoder> {
    reader: R,
    decoder: D,
    buffer: AlignedBuffer,
    buffer_len: usize,
    buffer_cursor: usize,
}

impl<R: Read, D: RawFormatDecoder> BufferedDecoder<R, D> {
    pub fn new(reader: R, decoder: D) -> Self {
        BufferedDecoder {
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

impl<R: Read + Send + Sync, D: RawFormatDecoder, O: crate::events::EventCollection> DecoderStream<O>
    for BufferedDecoder<R, D>
{
    fn next_batch(&mut self, end_time: u64, max_events: usize) -> Option<O> {
        let word_size = std::mem::size_of::<D::RawEventType>();
        let mut output = O::default();

        loop {
            // Calculate remaining budget for this decode call
            let remaining_budget = max_events.saturating_sub(output.num_events());
            if remaining_budget == 0 {
                return Some(output);
            }

            // If we have data in the buffer, decode from it
            if self.buffer_cursor < self.buffer_len {
                let usable = self.buffer_len - self.buffer_cursor;
                let aligned = usable - (usable % word_size);

                if aligned > 0 {
                    let slice = &self.buffer.0[self.buffer_cursor..self.buffer_cursor + aligned];
                    let events: &[D::RawEventType] = bytemuck::cast_slice(slice);

                    let consumed =
                        self.decoder
                            .decode(events, &mut output, end_time, remaining_budget);

                    self.buffer_cursor += consumed * word_size;

                    // If decoder consumed less than available, budget was exhausted.
                    // Return whatever we have (might be empty = "window is empty").
                    if consumed < events.len() {
                        return Some(output);
                    }

                    // If budget is reached, yield
                    if output.num_events() >= max_events {
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
/// type implementing [`DecodeInto`]. This is determined at compile time,
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
/// decoder.set_slice_by(SliceBy::Time(33_333));
/// while let Some(batch) = decoder.next() {
///     process(&batch);
///     decoder.set_slice_by(SliceBy::Time(10_000)); // can change mid-stream
/// }
/// ```
/// Public iterator for decoding events with optional time/count/byte slicing.
///
/// This struct wraps a format-specific decoder and provides a unified interface
/// for iterating over batches of events. The output type `O` is determined at compile
/// time and defaults to [`EventBatch`].
///
/// # Slicing Modes
///
/// The decoder supports several slicing modes via [`set_slice_by`](EventDecoder::set_slice_by):
///
/// - `SliceBy::Time(us)` — batches span a fixed time window
/// - `SliceBy::Count(n)` — batches contain up to n events
/// - `SliceBy::Both(us, n)` — whichever limit is hit first
/// - `SliceBy::Bytes` — decode in fixed 256 KB chunks (default, fastest)
///
/// The default mode is `Bytes`, which processes data in buffer-sized
/// chunks (256 KB) without enforcing specific time or count constraints. This is the fastest
/// mode for streaming through data when you don't need precise batch boundaries.
///
/// # Examples
///
/// ```no_run
/// # use libreeb::{RawFileReader, SliceBy};
/// let mut reader = RawFileReader::new("data.raw")?;
/// let decoder = reader.decoder();
///
/// // Default mode: chunked decoding
/// for batch in decoder {
///     println!("Got {} events", batch.len());
/// }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// With time-based slicing:
/// ```no_run
/// # use libreeb::{RawFileReader, SliceBy};
/// let mut reader = RawFileReader::new("data.raw")?;
/// let decoder = reader.decoder();
/// decoder.set_slice_by(SliceBy::Time(33_333)); // 33.3ms slices
///
/// for batch in decoder {
///     // Each batch spans ~33ms
/// }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// Changing slice mode mid-stream:
/// ```no_run
/// # use libreeb::{RawFileReader, SliceBy};
/// let mut reader = RawFileReader::new("data.raw")?;
/// let mut decoder = reader.decoder();
///
/// decoder.set_slice_by(SliceBy::Time(10_000)); // Start with 10ms
/// if let Some(first) = decoder.next() {
///     println!("First batch: {} events", first.len());
///     decoder.set_slice_by(SliceBy::Count(5000)); // Switch to count-based
/// }
///
/// for batch in decoder {
///     println!("Got {} events", batch.len());
/// }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Type Erasure and Compile-Time Polymorphism
///
/// The output type `O` is a compile-time generic parameter that must implement
/// [`DecodeInto`](crate::DecodeInto). By default, it's [`EventBatch`] (SoA format).
///
/// For most users, the default is fine — just use [`RawFileReader::decoder()`]
/// which returns `EventDecoder<EventBatch>`.
///
/// Advanced users can create custom output types using [`make_decoder_generic`]:
///
/// ```no_run
/// # use std::io::{BufReader, Cursor};
/// # use libreeb::{make_decoder_generic, header::RawEventType, DecodeInto};
/// # struct MyOutput;
/// # impl Default for MyOutput { fn default() -> Self { MyOutput } }
/// # impl DecodeInto for MyOutput {
/// #     fn push_cd(&mut self, x: u16, y: u16, p: u8, t: u64) {}
/// # }
/// let cursor = Cursor::new(vec![/* raw data */]);
/// let mut decoder = make_decoder_generic::<_, MyOutput>(
///     BufReader::new(cursor),
///     RawEventType::Evt3
/// )?;
///
/// for output in decoder {
///     // output is MyOutput, not EventBatch
/// }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```DecodeInto
///
/// The type erasure happens via the `DecoderStream<O>` trait, which is boxed
/// internally. This allows different format decoders (EVT2, EVT21, EVT3) to be
/// used interchangeably while maintaining compile-time polymorphism for the
/// output type.
pub struct EventDecoder<O: DecodeInto = EventBatch> {
    stream: Box<dyn DecoderStream<O>>,
    slice_by: SliceBy,
    slice_anchor: Option<u64>,
}

impl<O: DecodeInto> EventDecoder<O> {
    /// Create a new decoder from a boxed stream (internal use).
    pub(crate) fn from_stream(stream: Box<dyn DecoderStream<O>>) -> Self {
        EventDecoder {
            stream,
            slice_by: SliceBy::Bytes,
            slice_anchor: None,
        }
    }

    /// Set the slicing mode.
    ///
    /// Takes effect on the next call to [`Iterator::next`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use libreeb::{RawFileReader, SliceBy};
    /// let mut reader = RawFileReader::new("data.raw")?;
    /// let mut decoder = reader.decoder();
    ///
    /// // Time-based slicing: 33.3ms windows
    /// decoder.set_slice_by(SliceBy::Time(33_333));
    ///
    /// // Count-based slicing: 10k events per batch
    /// decoder.set_slice_by(SliceBy::Count(10_000));
    ///
    /// // Chunked decoding: fixed 256 KB chunks (fastest)
    /// decoder.set_slice_by(SliceBy::Bytes);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn set_slice_by(&mut self, slice_by: SliceBy) {
        self.slice_by = slice_by;
        self.slice_anchor = None;
    }

    /// Compute (end_time, max_events) for the current slice.
    fn budget(&self) -> (u64, usize) {
        match self.slice_by {
            SliceBy::Bytes => (u64::MAX, usize::MAX),
            SliceBy::Time(us) => {
                let end = self
                    .slice_anchor
                    .map_or(u64::MAX, |anchor| anchor.saturating_add(us));
                (end, usize::MAX)
            }
            SliceBy::Count(n) => (u64::MAX, n),
            SliceBy::Both(us, n) => {
                let end = self
                    .slice_anchor
                    .map_or(u64::MAX, |anchor| anchor.saturating_add(us));
                (end, n)
            }
        }
    }
}

impl<O: crate::events::EventCollection> Iterator for EventDecoder<O> {
    type Item = O;

    fn next(&mut self) -> Option<O> {
        // Bytes mode: just decode one full chunk without time/count constraints
        if matches!(self.slice_by, SliceBy::Bytes) {
            return self.stream.next_batch(u64::MAX, usize::MAX);
        }

        // Time/Count/Both mode: need anchoring and windowing
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
