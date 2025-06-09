use crate::{Event, EventDecoder};
use bin_proto::*;
use bytemuck::{Pod, Zeroable};
use fixed_vec_deque::FixedVecDeque;
use std::{collections::VecDeque, io::Read, simd::u16x32};
use zerocopy::{FromBytes, Immutable, KnownLayout};

#[derive(Debug, BitDecode, BitEncode, PartialEq)]
#[codec(discriminant_type = u8, bits = 4)] // 4-bit discriminant
#[repr(u16)] // Ensure the whole thing fits in u16
enum Evt3Codec {
    #[codec(discriminant = 0b0000)]
    VectBase {
        #[codec(bits = 1)]
        valid: bool, // 1 bit
        #[codec(bits = 6)]
        vect_base_x: u8, // 6 bits
        #[codec(bits = 5)]
        vect_base_y: u8, // 5 bits
    },

    #[codec(discriminant = 0b0001)]
    VectOff {
        #[codec(bits = 1)]
        valid: bool, // 1 bit
        #[codec(bits = 4)]
        nb_vect_off: u8, // 4 bits
        #[codec(bits = 4)]
        vect_off_x: u8, // 4 bits
        #[codec(bits = 3)]
        vect_off_y: u8, // 3 bits
    },

    #[codec(discriminant = 0b0010)]
    EvtAddr {
        #[codec(bits = 1)]
        system_type: bool, // 1 bit
        #[codec(bits = 1)]
        pol: bool, // 1 bit
        #[codec(bits = 5)]
        x: u8, // 5 bits
        #[codec(bits = 5)]
        y: u8, // 5 bits
    },

    #[codec(discriminant = 0b0011)]
    EvtAddrY {
        #[codec(bits = 1)]
        system_type: bool, // 1 bit
        #[codec(bits = 1)]
        pol: bool, // 1 bit
        #[codec(bits = 10)]
        y: u16, // 10 bits
    },

    #[codec(discriminant = 0b0100)]
    EvtAddrX {
        #[codec(bits = 1)]
        system_type: bool, // 1 bit
        #[codec(bits = 1)]
        pol: bool, // 1 bit
        #[codec(bits = 10)]
        x: u16, // 10 bits
    },

    #[codec(discriminant = 0b0110)]
    EvtTimeLow {
        #[codec(bits = 12)]
        evt_time_low: u16, // 12 bits
    },

    #[codec(discriminant = 0b1000)]
    EvtTimeHigh {
        #[codec(bits = 12)]
        evt_time_high: u16, // 12 bits
    },

    #[codec(discriminant = 0b1110)]
    Others {
        #[codec(bits = 12)]
        data: u16, // 12 bits
    },

    #[codec(discriminant = 0b1111)]
    Continued12 {
        #[codec(bits = 12)]
        data: u16, // 12 bits
    },
}

/// Struct for holding raw EVT3 types
#[derive(FromBytes, Immutable, KnownLayout, Copy, Clone, Pod, Zeroable)]
#[repr(C)]
struct Evt3Word {
    data: u16,
}

/// Struct for holding vectors of EVT3 types
#[derive(Copy, Clone, Pod, Zeroable)]
#[repr(C)]
#[derive(Debug)]
struct Evt3Simd {
    data: u16x32,
}

impl Evt3Simd {
    // Extract event type (bits 15-12)
    fn event_type(&self) -> u16x32 {
        (self.data >> 12) & u16x32::splat(0xF)
    }

    // Extract X or Y coordinate (bits 10-0)
    fn xy(&self) -> u16x32 {
        self.data & u16x32::splat(0x7FF)
    }

    // Extract polarity (bit 11)
    fn pol(&self) -> u16x32 {
        (self.data >> 11) & u16x32::splat(0x1)
    }

    // Extract time value (bits 11-0)
    fn time_and_valid(&self) -> u16x32 {
        self.data & u16x32::splat(0xFFF)
    }

    // // Extract valid bits for vector events
    // fn valid(&self) -> u16x32 {
    //     self.data & u16x32::splat(0xFFF)
    // }

    // Extract trigger ID (bits 11-8)
    fn trigger_id(&self) -> u16x32 {
        (self.data >> 8) & u16x32::splat(0xF)
    }

    // Extract trigger polarity (bit 0)
    fn trigger_polarity(&self) -> u16x32 {
        self.data & u16x32::splat(0x1)
    }
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
        self.data & 0xFFF
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
                *$events.push_back() = Event::CD {
                    x: i,
                    y: $state.y,
                    p: $state.polarity,
                    t: $state.time,
                };
            }
            valid_bits >>= 1;
        }
        $state.x = end;
    }};
}

const MAX_TIMESTAMP_BASE: u64 = ((1u64 << 12) - 1) << 12; // = 16773120us
const TIME_LOOP_DURATION_US: u64 = MAX_TIMESTAMP_BASE + (1 << 12); // = 16777216us
const LOOP_THRESHOLD: u64 = 10 << 12; // It could be another value too, as long as it is a big enough value that we can be sure that the time high looped
const READ_BUFFER_SIZE: usize = 512;

#[repr(align(64))]
struct AlignedBuffer([u8; READ_BUFFER_SIZE]);

pub struct Evt3DecoderCore<'a, R: Read> {
    reader: &'a mut R,
    buffer: AlignedBuffer,
    // event_queue: VecDeque<Event>,
    event_queue: FixedVecDeque<[Event; 4096]>,
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
            buffer: AlignedBuffer([0; READ_BUFFER_SIZE]),
            event_queue: FixedVecDeque::<[Event; 4096]>::new(),
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
                return self.event_queue.pop_front().copied();
            }

            // If the timebase isn't set we need to find  it
            if self.time_base.is_none() {
                let mut buffer: [u8; 2] = [0; 2];
                loop {
                    self.reader.read_exact(&mut buffer).unwrap();
                    let evt: Evt3Word = bytemuck::cast(buffer);
                    if evt.event_type() == EVT_TIME_HIGH {
                        self.time_base = Some((evt.time() as u64) << 12);
                        break;
                    }
                }
            }

            // The event queue is empty, we need to fill it
            // First check the overflow byte (if we didn't manage to get an aligned read)
            let n = if let Some(byte) = self.overflow_byte {
                self.buffer.0[0] = byte;
                // Read just one byte to complete the slice
                self.reader.read(&mut self.buffer.0[1..2]).ok()?
            } else {
                self.reader.read(&mut self.buffer.0[..]).ok()?
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
                    self.overflow_byte = Some(self.buffer.0[n - 1]);
                }

                // Ensure we process a multiple of 2 bytes
                n - (n % 2)
            };

            // If we did read only one byte from the reader, we need to read again
            if bytes_to_process == 0 {
                continue;
            }

            if bytes_to_process == READ_BUFFER_SIZE {
                // FIXME: USING SIMD IS ACTUALLY SLOWER
                let simd_slice: &[Evt3Simd] =
                    bytemuck::cast_slice(&self.buffer.0[..bytes_to_process]);

                // let simd_data: u16x32 = Simd::from_slice(&self.buffer.0[..]);

                for simd_evt in simd_slice {
                    let event_types = simd_evt.event_type();
                    let xys = simd_evt.xy();
                    let pols = simd_evt.pol();
                    let times_and_valids = simd_evt.time_and_valid();

                    for i in 0..32 {
                        let event_type = event_types[i];
                        let xy = xys[i];
                        let pol = pols[i];
                        let time_and_valid = times_and_valids[i];
                        // Process the event based on its type
                        match event_type as u8 {
                            EVT_ADDR_Y => {
                                self.y = xy; // Update State
                            }
                            EVT_ADDR_X => {
                                // Create Event
                                *self.event_queue.push_back() = Event::CD {
                                    x: xy,
                                    y: self.y,
                                    p: pol as u8,
                                    t: self.time,
                                };
                            }
                            VECT_BASE_X => {
                                // Update State
                                self.polarity = pol as u8;
                                self.x = xy;
                            }
                            VECT_12 => {
                                // Create Event
                                handle_vect!(self, self.event_queue, time_and_valid, 12);
                            }
                            VECT_8 => {
                                // Create Event
                                handle_vect!(self, self.event_queue, time_and_valid, 8);
                            }
                            EVT_TIME_LOW => {
                                let event_time = time_and_valid as u64;
                                self.time = self.time_base.unwrap() + event_time;
                            }
                            EVT_TIME_HIGH => {
                                let time_base = self.time_base.unwrap();
                                let event_time = time_and_valid as u64;
                                let mut new_time_high = event_time << 12;
                                new_time_high +=
                                    self.time_high_loop_nb as u64 * TIME_LOOP_DURATION_US;

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
                            EXT_TRIGGER => {
                                *self.event_queue.push_back() = Event::ExternalTrigger {
                                    id: 0 as u8, // FIXME
                                    p: 0 as u8,  // FIXME
                                };

                                // self.event_queue.push_back(Event::ExternalTrigger {
                                //     id: 0 as u8, // FIXME
                                //     p: 0 as u8,  // FIXME
                                // });
                            }
                            _ => {
                                *self.event_queue.push_back() = Event::Unknown;
                                // self.event_queue.push_back(Event::Unknown);
                            }
                        }
                    }
                }
            } else {
                let evts = <[Evt3Word]>::ref_from_bytes_with_elems(
                    &self.buffer.0[..bytes_to_process],
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
                            *self.event_queue.push_back() = Event::CD {
                                x: evt.x(),
                                y: self.y,
                                p: evt.pol(),
                                t: self.time,
                            };
                        }
                        VECT_BASE_X => {
                            // Update State
                            self.polarity = evt.pol();
                            self.x = evt.x();
                        }
                        VECT_12 => {
                            // Create Event
                            handle_vect!(self, self.event_queue, evt.valid(), 12);
                        }
                        VECT_8 => {
                            // Create Event
                            handle_vect!(self, self.event_queue, evt.valid(), 8);
                        }
                        EVT_TIME_LOW => {
                            let event_time = evt.time() as u64;
                            self.time = self.time_base.unwrap() + event_time;
                        }
                        EVT_TIME_HIGH => {
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
                        EXT_TRIGGER => {
                            *self.event_queue.push_back() = Event::ExternalTrigger {
                                id: evt.trigger_id(),
                                p: evt.trigger_polarity(),
                            };
                        }
                        _ => {
                            *self.event_queue.push_back() = Event::Unknown;
                        }
                    }
                });
            }
        }
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
