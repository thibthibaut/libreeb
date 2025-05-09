use crate::{define_raw_evt, Event, EventDecoder};
use stackvector::StackVec;
use std::io::Read;

// Define EVT3 events, the layout is:
//
// 15        12 11                         0
// +---4 bits--+-------12 bits------------+
// | Event Type|           Payload        |
// +-----------+--------------------------+
define_raw_evt! {
    #[storage(u16), size(2), discriminant(12, 4)]
    enum Evt3 {
        EvtAddrY (0x0) {
            #[0,11]
            y: u16,
            #[11,1]
            _origin: u8
        },
        EvtAddrX (0x2) {
            #[0,11]
            x: u16,
            #[11,1]
            pol: u8
        },
        VectBaseX (0x3) {
            #[0,11]
            x: u16,
            #[11,1]
            pol: u8
        },
        Vect12 (0x4) {
            #[0, 12]
            valid: u16
        },
        Vect8 (0x5) {
            #[0, 8]
            valid: u16
        },
        EvtTimeLow (0x6) {
            #[0, 12]
            time: u16
        },
        EvtTimeHigh (0x8) {
            #[0, 12]
            time: u16
        },
        ExtTrigger (0xA){
            #[8,4]
            id: u8,
            #[0,1]
            polarity: u8
        }
    }
}

macro_rules! handle_vect {
    ($state:expr, $events:expr, $valid:expr, $vect_size:expr) => {{
        let end = $state.x + $vect_size;
        let mut valid_bits = $valid;

        for i in $state.x..end {
            if valid_bits & 1 == 1 {
                $events.push(Event::CD {
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
const LOOP_THRESHOLD: u64 = 10 << 12; // It could be another value too, as long as it is a big enough value that we can be  sure that the time high looped

#[derive(Debug, Default)]
pub struct Evt3Decoder {
    time: u64,
    time_base: Option<u64>, // Keeps track of time high (base time)
    time_high_loop_nb: u32, // Counts overflows of time high
    polarity: u8,
    x: u16,
    y: u16,
}

impl EventDecoder for Evt3Decoder {
    fn decode<'a>(&'a mut self, reader: &'a mut impl Read) -> Box<dyn Iterator<Item = Event> + 'a> {
        Box::new(
            std::iter::from_fn(move || {
                let mut buffer = [0u8; 2];
                match reader.read_exact(&mut buffer) {
                    Ok(()) => {
                        let mut events = StackVec::<[Event; 12]>::new();
                        // Convert byte chunk into raw event
                        let raw_event: Evt3 = buffer.as_slice().into();

                        match raw_event {
                            Evt3::EvtAddrY { y, _origin } => self.y = y, // Update State
                            Evt3::EvtAddrX { x, pol } => {
                                // Create Event
                                events.push(Event::CD {
                                    x,
                                    y: self.y,
                                    p: pol,
                                    t: self.time,
                                });
                            }
                            Evt3::VectBaseX { x, pol } => {
                                // Update State
                                self.polarity = pol;
                                self.x = x;
                            }
                            Evt3::Vect12 { valid } => {
                                // Create Event
                                handle_vect!(self, events, valid, 12);
                            }
                            Evt3::Vect8 { valid } => {
                                // Create Event
                                handle_vect!(self, events, valid, 8);
                            }
                            Evt3::EvtTimeLow { time } => {
                                let event_time = time as u64;
                                self.time = self.time_base.unwrap_or(0) + event_time;
                            }
                            Evt3::EvtTimeHigh { time } => {
                                if self.time_base.is_none() {
                                    self.time_base = Some((time as u64) << 12);
                                }

                                let event_time = time as u64;
                                let mut new_time_high = event_time << 12;
                                new_time_high +=
                                    self.time_high_loop_nb as u64 * TIME_LOOP_DURATION_US;

                                if (self.time_base.unwrap_or(0) > new_time_high)
                                    && (self.time_base.unwrap_or(0) - new_time_high
                                        >= MAX_TIMESTAMP_BASE - LOOP_THRESHOLD)
                                {
                                    new_time_high += TIME_LOOP_DURATION_US;
                                    self.time_high_loop_nb += 1;
                                }
                                self.time_base = Some(new_time_high);
                                self.time = self.time_base.unwrap_or(0);
                            }
                            Evt3::ExtTrigger { id, polarity } => {
                                events.push(Event::ExternalTrigger { id, p: polarity })
                            }
                            _ => {
                                events.push(Event::Unknown);
                            }
                        } // end match

                        // Remove invalid events
                        if self.time_base.is_none() {
                            events.clear()
                        }
                        Some(events.into_iter())
                    }
                    Err(_e) => None,
                }
            })
            .flatten(),
        )
    }
}
