use crate::{Event, EventDecoder};
// use stackvector::StackVec;
use std::{collections::VecDeque, io::Read};
use zerocopy::{FromBytes, Immutable, KnownLayout};

// Define a struct for all EVT3 event types
#[derive(FromBytes, Immutable, KnownLayout)]
#[repr(C)]
struct Evt3Word {
    // 16-bit value containing all event data
    data: u16,
}

impl Evt3Word {
    // Extract event type (bits 15-12)
    fn event_type(&self) -> u8 {
        ((self.data >> 12) & 0xF) as u8
    }

    // Extract Y coordinate (bits 10-0)
    fn y(&self) -> u16 {
        self.data & 0x7FF
    }

    // Extract X coordinate (bits 10-0)
    fn x(&self) -> u16 {
        self.data & 0x7FF
    }

    // Extract polarity (bit 11)
    fn pol(&self) -> u8 {
        ((self.data >> 11) & 0x1) as u8
    }

    // Extract origin/system_type (bit 11)
    fn origin(&self) -> u8 {
        ((self.data >> 11) & 0x1) as u8
    }

    // Extract time value (bits 11-0)
    fn time(&self) -> u16 {
        self.data & 0xFFF
    }

    // Extract valid bits for vector events (bits 11-0 for VECT_12, bits 7-0 for VECT_8)
    fn valid(&self) -> u16 {
        // if self.event_type() == 0x4 {
        // VECT_12
        self.data & 0xFFF
        // } else {
        // VECT_8
        // self.data & 0xFF
        // }
    }

    // Extract trigger ID (bits 11-8)
    fn trigger_id(&self) -> u8 {
        ((self.data >> 8) & 0xF) as u8
    }

    // Extract trigger polarity (bit 0)
    fn trigger_polarity(&self) -> u8 {
        (self.data & 0x1) as u8
    }
}

// Define event type constants
const EVT_ADDR_Y: u8 = 0x0;
const EVT_ADDR_X: u8 = 0x2;
const VECT_BASE_X: u8 = 0x3;
const VECT_12: u8 = 0x4;
const VECT_8: u8 = 0x5;
const EVT_TIME_LOW: u8 = 0x6;
const EVT_TIME_HIGH: u8 = 0x8;
const EXT_TRIGGER: u8 = 0xA;

macro_rules! handle_vect {
    ($state:expr, $events:expr, $valid:expr, $vect_size:expr) => {{
        let end = $state.x + $vect_size;
        let mut valid_bits = $valid;

        for i in $state.x..end {
            if valid_bits & 1 == 1 {
                $events.push_back(Event::CD {
                    x: i,
                    y: $state.y,
                    p: $state.polarity,
                    t: $state.time,
                });
            }
            valid_bits >>= 1;
        }
        $state.x = end;
    }};
}

const MAX_TIMESTAMP_BASE: u64 = ((1u64 << 12) - 1) << 12; // = 16773120us
const TIME_LOOP_DURATION_US: u64 = MAX_TIMESTAMP_BASE + (1 << 12); // = 16777216us
const LOOP_THRESHOLD: u64 = 10 << 12; // It could be another value too, as long as it is a big enough value that we can be sure that the time high looped
const READ_BUFFER_SIZE: usize = 2048;

#[derive(Debug)]
pub struct Evt3DecoderCore<'a, R: Read> {
    reader: &'a mut R,
    buffer: [u8; READ_BUFFER_SIZE],
    event_queue: VecDeque<Event>,
    time: u64,
    time_base: Option<u64>, // Keeps track of time high (base time)
    time_high_loop_nb: u32, // Counts overflows of time high
    polarity: u8,
    x: u16,
    y: u16,
    overflow_byte: Option<u8>,
}
impl<'a, R: Read> Evt3DecoderCore<'a, R> {
    pub fn new(reader: &'a mut R) -> Self {
        Evt3DecoderCore {
            reader,
            buffer: [0; READ_BUFFER_SIZE],
            event_queue: std::collections::VecDeque::with_capacity(READ_BUFFER_SIZE),
            time: 0,
            time_base: None,
            time_high_loop_nb: 0,
            polarity: 0,
            x: 0,
            y: 0,
            overflow_byte: None,
        }
    }
}

impl<'a, R: Read> Iterator for Evt3DecoderCore<'a, R> {
    type Item = Event;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if !self.event_queue.is_empty() {
                return self.event_queue.pop_front();
            }

            // The event queue is empty, we need to fill it
            // Check the overflow byte
            let n = if let Some(byte) = self.overflow_byte {
                self.buffer[0] = byte;
                // Read just one byte to complete the slice
                self.reader.read(&mut self.buffer[1..2]).ok()?
            } else {
                self.reader.read(&mut self.buffer[..]).ok()?
            };

            if n == 0 {
                // we reach end of stream, end of iteration
                return None;
            }

            let bytes_to_process = if self.overflow_byte.take().is_some() {
                2
            } else {
                if n % 2 != 0 {
                    // If we have an odd number of bytes, store the last byte
                    self.overflow_byte = Some(self.buffer[n - 1]);
                }

                // Ensure we process a multiple of 2 bytes
                n - (n % 2)
            };

            // If we did read only one byte from the reader, we need to read again
            if bytes_to_process == 0 {
                continue;
            }

            let evts = <[Evt3Word]>::ref_from_bytes_with_elems(
                &self.buffer[..bytes_to_process],
                bytes_to_process / 2,
            )
            .unwrap();

            evts.iter().for_each(|evt| {
                // Process the event based on its type
                match evt.event_type() {
                    EVT_ADDR_Y => {
                        self.y = evt.y(); // Update State
                    }
                    EVT_ADDR_X => {
                        // Create Event
                        if self.time_base.is_some() {
                            self.event_queue.push_back(Event::CD {
                                x: evt.x(),
                                y: self.y,
                                p: evt.pol(),
                                t: self.time,
                            });
                        }
                    }
                    VECT_BASE_X => {
                        // Update State
                        self.polarity = evt.pol();
                        self.x = evt.x();
                    }
                    VECT_12 => {
                        // Create Event
                        if self.time_base.is_some() {
                            handle_vect!(self, self.event_queue, evt.valid(), 12);
                        }
                    }
                    VECT_8 => {
                        // Create Event
                        if self.time_base.is_some() {
                            handle_vect!(self, self.event_queue, evt.valid(), 8);
                        }
                    }
                    EVT_TIME_LOW => {
                        let event_time = evt.time() as u64;
                        if self.time_base.is_some() {
                            self.time = self.time_base.unwrap() + event_time;
                        }
                    }
                    EVT_TIME_HIGH => {
                        if self.time_base.is_none() {
                            self.time_base = Some((evt.time() as u64) << 12);
                        } else {
                            let time_base = self.time_base.unwrap();
                            let event_time = evt.time() as u64;
                            let mut new_time_high = event_time << 12;
                            new_time_high += self.time_high_loop_nb as u64 * TIME_LOOP_DURATION_US;

                            if (time_base > new_time_high)
                                && (time_base - new_time_high
                                    >= MAX_TIMESTAMP_BASE - LOOP_THRESHOLD)
                            {
                                new_time_high += TIME_LOOP_DURATION_US;
                                self.time_high_loop_nb += 1;
                            }
                            self.time_base = Some(new_time_high);
                            self.time = time_base;
                        }
                    }
                    EXT_TRIGGER => {
                        if self.time_base.is_some() {
                            self.event_queue.push_back(Event::ExternalTrigger {
                                id: evt.trigger_id(),
                                p: evt.trigger_polarity(),
                            });
                        }
                    }
                    _ => {
                        if self.time_base.is_some() {
                            self.event_queue.push_back(Event::Unknown);
                        }
                    }
                }
            });
        }

        // None
    }
}

#[derive(Default)]
pub struct Evt3Decoder;
impl EventDecoder for Evt3Decoder {
    fn decode<'a>(&'a mut self, reader: &'a mut impl Read) -> Box<dyn Iterator<Item = Event> + 'a> {
        let decoder = Evt3DecoderCore::new(reader);
        Box::new(decoder)
    }
}
