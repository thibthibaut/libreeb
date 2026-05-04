use crate::{EventBatch, EventDecoder};
use std::io::Read;

const READ_BUFFER_SIZE: usize = 262144;
const _: () = {
    assert!(READ_BUFFER_SIZE.is_multiple_of(8));
};

#[repr(align(64))]
struct AlignedBuffer([u8; READ_BUFFER_SIZE]);

pub struct EvtReader<R: Read, D: EventDecoder> {
    reader: R,
    decoder: D,
    buffer: AlignedBuffer,
    read_buffer_cursor: usize,
}

impl<R: Read, D: EventDecoder> EvtReader<R, D> {
    pub fn new(reader: R, decoder: D) -> Self {
        EvtReader {
            reader,
            decoder,
            buffer: AlignedBuffer([0; READ_BUFFER_SIZE]),
            read_buffer_cursor: 0,
        }
    }
}

impl<R: Read, D: EventDecoder> Iterator for EvtReader<R, D> {
    type Item = EventBatch;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // Try to fill the read buffer
            let bytes_read = self
                .reader
                .read(&mut self.buffer.0[self.read_buffer_cursor..])
                .ok()?;

            // Stop iteration when reaching end of stream
            if bytes_read == 0 && self.read_buffer_cursor == 0 {
                return None;
            }

            // Update current cursor
            self.read_buffer_cursor += bytes_read;

            // If we didn't fill the buffer we need to continue reading
            if bytes_read > 0 && self.read_buffer_cursor != READ_BUFFER_SIZE {
                continue;
            }

            let word_size = std::mem::size_of::<D::RawEventType>();

            // Compute the usable size (aligned to word boundary)
            let size = self.read_buffer_cursor - (self.read_buffer_cursor % word_size);

            let evts: &[D::RawEventType] = bytemuck::cast_slice(&self.buffer.0[..size]);

            // Reset the cursor
            self.read_buffer_cursor = 0;

            let mut batch = EventBatch::with_capacity(evts.len());
            self.decoder.decode(evts, &mut batch);

            // A buffer full of TIME_HIGH events produces an empty batch; loop to get more.
            if !batch.is_empty() {
                return Some(batch);
            }
        }
    }
}
