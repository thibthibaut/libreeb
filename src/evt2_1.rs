use crate::{Event, EventDecoder};
use bytemuck::{Pod, Zeroable};
use std::{collections::VecDeque, io::Read};
use zerocopy::{FromBytes, Immutable, KnownLayout};

// EVT2.1 raw events definition, the layout is:
//
// 63        60 59                         0
// +---4 bits--+-------60 bits------------+
// | Event Type|           Payload        |
// +-----------+--------------------------+
#[derive(FromBytes, Immutable, KnownLayout, Copy, Clone, Pod, Zeroable)]
#[repr(C)]
struct Evt21Word {
    data: u64,
}

const EVT_NEG: u8 = 0b0000;
const EVT_POS: u8 = 0b0001;
const EVT_TIME_HIGH: u8 = 0b1000;
const EXT_TRIGGER: u8 = 0b1010;
const OTHERS: u8 = 0b1110;

impl Evt21Word {
    fn event_type(&self) -> u8 {
        (self.data >> 60) as u8
    }

    /// Extracts msb part of the timestamp.
    fn time_high(&self) -> u64 {
        // Shift it to position 59:32
        let mask = (1u64 << 28) - 1;
        let timestamp_mask = mask << 32;

        // Apply the mask and shift back to get the value
        (self.data & timestamp_mask) >> 26
    }

    fn time_low(&self) -> u64 {
        let mask = (1u64 << 6) - 1;
        let timestamp_mask = mask << 54;
        (self.data & timestamp_mask) >> 54
    }

    /// Extracts the x-coordinate (aligned on 32) from the data word.
    fn x(&self) -> u16 {
        ((self.data >> 43) & 0x7FF) as u16
    }

    /// Extracts the y-coordinate from the data word.
    fn y(&self) -> u16 {
        ((self.data >> 32) & 0x7FF) as u16
    }

    /// Extracts the 32-bit valid mask, indicating which pixels in the 32-pixel group are valid.
    fn valid_mask(&self) -> u32 {
        (self.data & 0xFFFFFFFF) as u32
    }

    /// Extracts the trigger channel ID from an EXT_TRIGGER event.
    /// Returns `None` if the event is not an EXT_TRIGGER.
    fn trigger_channel_id(&self) -> u8 {
        ((self.data >> 40) & 0x1F) as u8
    }

    /// Extracts the trigger value (edge polarity) from an EXT_TRIGGER event.
    /// Returns `None` if the event is not an EXT_TRIGGER.
    /// Returns `Some(true)` for a rising edge (1),
    /// and `Some(false)` for a falling edge (0).
    fn trigger_value(&self) -> u8 {
        ((self.data >> 32) & 0x1) as u8
    }
}

// define_raw_evt! {
//     #[storage(u64), size(8),  discriminant(60, 4)]
//     enum Evt21 {
//         EvtNeg (0x0) {
//             #[54,6]
//             timestamp: u16,
//             #[43,11]
//             x: u16,
//             #[32,11]
//             y: u16,
//             #[0,32]
//             valid: u32
//         },
//         EvtPos (0x1) {
//             #[54,6]
//             timestamp: u16,
//             #[43,11]
//             x: u16,
//             #[32,11]
//             y: u16,
//             #[0,32]
//             valid: u32
//         },
//         EvtTimeHigh (0x8) {
//             #[32,28]
//             timestamp: u32
//         },
//         ExtTrigger (0xa) {
//             #[54,6]
//             timestamp: u16,
//             #[40,5]
//             id: u8,
//             #[32,1]
//             polarity: u8
//         }
//     }
// }
const READ_BUFFER_SIZE: usize = 512;

const _: () = {
    assert!(READ_BUFFER_SIZE % 8 == 0);
};

#[repr(align(64))]
struct AlignedBuffer([u8; READ_BUFFER_SIZE]);

pub struct Evt21DecoderCore<'a, R: Read> {
    reader: &'a mut R,
    buffer: AlignedBuffer,
    event_queue: VecDeque<Event>,
    time_high: Option<u64>,
    read_buffer_cursor: usize,
}

impl<'a, R: Read> Evt21DecoderCore<'a, R> {
    pub fn new(reader: &'a mut R) -> Self {
        Evt21DecoderCore {
            reader,
            buffer: AlignedBuffer([0; READ_BUFFER_SIZE]),
            event_queue: VecDeque::<Event>::new(),
            time_high: None,
            read_buffer_cursor: 0,
        }
    }
}

impl<'a, R: Read> Iterator for Evt21DecoderCore<'a, R> {
    type Item = Event;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if !self.event_queue.is_empty() {
                return self.event_queue.pop_front();
            }

            // If the timebase isn't set we need to find  it
            if self.time_high.is_none() {
                let mut buffer: [u8; 8] = [0; 8];
                self.reader.read_exact(&mut buffer).unwrap();
                let evt: Evt21Word = bytemuck::cast(buffer);
                if evt.event_type() == EVT_TIME_HIGH {
                    self.time_high = Some(evt.time_high());
                }
                continue; // Skip the rest of the loop because we don't have a time high
            }

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

            // Compute the size
            let size = self.read_buffer_cursor - (self.read_buffer_cursor % 8);

            let evts =
                <[Evt21Word]>::ref_from_bytes_with_elems(&self.buffer.0[..size], size / 8).unwrap();

            // Reset the cursor
            self.read_buffer_cursor = 0;

            evts.iter().for_each(|evt| {
                // Process the event based on its type
                // TODO: test if a jump table is faster!
                match evt.event_type() {
                    EVT_NEG | EVT_POS => {
                        // Compute the full timestamp
                        let full_timestamp = self.time_high.unwrap() | evt.time_low();
                        let mut mask = evt.valid_mask();
                        while mask != 0 {
                            let offset = mask.trailing_zeros();
                            // Clear the lowest set bit
                            mask = mask & (mask - 1);
                            self.event_queue.push_back(Event::CD {
                                x: evt.x() + offset as u16,
                                y: evt.y(),
                                p: evt.event_type(),
                                t: full_timestamp,
                            });
                        }
                    }
                    EVT_TIME_HIGH => self.time_high = Some(evt.time_high()),
                    EXT_TRIGGER => self.event_queue.push_back(Event::ExternalTrigger {
                        id: evt.trigger_channel_id(),
                        p: evt.trigger_value(),
                    }),
                    _ => self.event_queue.push_back(Event::Unknown),
                } // end match type of event
            });
        } // end loop{
    }
}

#[derive(Default)]
pub struct Evt21Decoder;
impl EventDecoder for Evt21Decoder {
    fn decode<'a>(&'a mut self, reader: &'a mut impl Read) -> Box<dyn Iterator<Item = Event> + 'a> {
        let decoder = Evt21DecoderCore::new(reader);
        Box::new(decoder)
    }
}
