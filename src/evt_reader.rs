use crate::{Event, EventDecoder};
use std::{collections::VecDeque, io::Read};
use zerocopy::FromBytes;
const READ_BUFFER_SIZE: usize = 512;
const _: () = {
    assert!(READ_BUFFER_SIZE % 8 == 0);
};

#[repr(align(64))]
struct AlignedBuffer([u8; READ_BUFFER_SIZE]);

pub struct EvtReader<R: Read, D: EventDecoder> {
    reader: R,
    decoder: D,
    buffer: AlignedBuffer,
    event_queue: VecDeque<Event>,
    read_buffer_cursor: usize,
}

impl<R: Read, D: EventDecoder> EvtReader<R, D> {
    pub fn new(reader: R, decoder: D) -> Self {
        EvtReader {
            reader,
            decoder,
            buffer: AlignedBuffer([0; READ_BUFFER_SIZE]),
            event_queue: VecDeque::<Event>::new(),
            read_buffer_cursor: 0,
        }
    }
}

impl<R: Read, D: EventDecoder> Iterator for EvtReader<R, D> {
    type Item = Event;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if !self.event_queue.is_empty() {
                return self.event_queue.pop_front();
            }

            // If the timebase isn't set we need to find  it
            // if self.time_high.is_none() {
            //     let mut buffer: [u8; 8] = [0; 8];
            //     self.reader.read_exact(&mut buffer).unwrap();
            //     let evt: Evt21Word = bytemuck::cast(buffer);
            //     if evt.event_type() == EVT_TIME_HIGH {
            //         self.time_high = Some(evt.time_high());
            //     }
            //     continue; // Skip the rest of the loop because we don't have a time high
            // }

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

            // If we didn't fill the buffer we need continue reading
            if bytes_read > 0 && self.read_buffer_cursor != READ_BUFFER_SIZE {
                continue;
            }

            let word_size = std::mem::size_of::<D::RawEventType>();

            // Compute the size
            let size = self.read_buffer_cursor - (self.read_buffer_cursor % word_size);

            let evts = <[D::RawEventType]>::ref_from_bytes_with_elems(
                &self.buffer.0[..size],
                size / word_size,
            )
            .unwrap();

            // Reset the cursor
            self.read_buffer_cursor = 0;

            self.decoder.decode(evts, &mut self.event_queue);
            // evts.iter().for_each(); // TODO
        } // end loop{
    }
}
