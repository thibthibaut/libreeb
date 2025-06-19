use crate::{Event, EventDecoder};
use zerocopy::{FromBytes, Immutable, KnownLayout};

/// Struct for holding raw EVT3 types
#[derive(FromBytes, Immutable, KnownLayout, Copy, Clone)]
#[repr(C)]
pub struct Evt3 {
    data: u16,
}

impl Evt3 {
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

// Event types constants
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

pub struct Evt3Decoder {
    time: u64,
    time_base: Option<u64>, // Keeps track of time high (base time)
    time_high_loop_nb: u32, // Counts overflows of time high
    polarity: u8,
    x: u16,
    y: u16,
}

impl EventDecoder for Evt3Decoder {
    type RawEventType = Evt3;

    fn new() -> Self {
        Evt3Decoder {
            time: 0,
            time_base: None,
            time_high_loop_nb: 0,
            polarity: 0,
            x: 0,
            y: 0,
        }
    }

    fn decode(
        &mut self,
        raw_event: &[Self::RawEventType],
        event_queue: &mut std::collections::VecDeque<Event>,
    ) {
        raw_event.iter().for_each(|evt| {
            // Process the event based on its type
            match evt.event_type() {
                EVT_ADDR_Y => {
                    self.y = evt.y(); // Update State
                }
                EVT_ADDR_X => {
                    if self.time_base.is_none() {
                        return;
                    }
                    // Create Event
                    event_queue.push_back(Event::CD {
                        x: evt.x(),
                        y: self.y,
                        p: evt.pol(),
                        t: self.time,
                    });
                }
                VECT_BASE_X => {
                    // Update State
                    self.polarity = evt.pol();
                    self.x = evt.x();
                }
                VECT_12 => {
                    if self.time_base.is_none() {
                        return;
                    }
                    // Create Event
                    handle_vect!(self, event_queue, evt.valid(), 12);
                }
                VECT_8 => {
                    if self.time_base.is_none() {
                        return;
                    }
                    // Create Event
                    handle_vect!(self, event_queue, evt.valid(), 8);
                }
                EVT_TIME_LOW => {
                    let Some(time_base) = self.time_base else {
                        return;
                    };
                    let event_time = evt.time() as u64;
                    self.time = time_base + event_time;
                }
                EVT_TIME_HIGH => {
                    let event_time = evt.time() as u64;
                    let time_base = *self.time_base.get_or_insert(event_time << 12);
                    let mut new_time_high = event_time << 12;
                    new_time_high += self.time_high_loop_nb as u64 * TIME_LOOP_DURATION_US;

                    if (time_base > new_time_high)
                        && (time_base - new_time_high >= MAX_TIMESTAMP_BASE - LOOP_THRESHOLD)
                    {
                        new_time_high += TIME_LOOP_DURATION_US;
                        self.time_high_loop_nb += 1;
                    }
                    self.time_base = Some(new_time_high);
                    self.time = time_base;

                    let event_time = evt.time() as u64;
                    let mut new_time_high = event_time << 12;
                    new_time_high += self.time_high_loop_nb as u64 * TIME_LOOP_DURATION_US;
                    let time_base = self.time_base.unwrap();
                    if (self.time_base.unwrap() > new_time_high)
                        && (time_base - new_time_high >= MAX_TIMESTAMP_BASE - LOOP_THRESHOLD)
                    {
                        new_time_high += TIME_LOOP_DURATION_US;
                        self.time_high_loop_nb += 1;
                    }
                    self.time_base = Some(new_time_high);
                    self.time = time_base;
                }
                EXT_TRIGGER => {
                    event_queue.push_back(Event::ExternalTrigger {
                        id: evt.trigger_id(),
                        p: evt.trigger_polarity(),
                    });
                }
                _ => {
                    event_queue.push_back(Event::Unknown);
                }
            }
        });
    }
}
