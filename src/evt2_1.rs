use crate::{define_raw_evt, Event, EventDecoder};
use stackvector::StackVec;
use std::io::Read;

// EVT2.1 raw events definition, the layout is:
//
// 63        60 59                         0
// +---4 bits--+-------60 bits------------+
// | Event Type|           Payload        |
// +-----------+--------------------------+
define_raw_evt! {
    #[storage(u64), size(8),  discriminant(60, 4)]
    enum Evt21 {
        EvtNeg (0x0) {
            #[54,6]
            timestamp: u16,
            #[43,11]
            x: u16,
            #[32,11]
            y: u16,
            #[0,32]
            valid: u32
        },
        EvtPos (0x1) {
            #[54,6]
            timestamp: u16,
            #[43,11]
            x: u16,
            #[32,11]
            y: u16,
            #[0,32]
            valid: u32
        },
        EvtTimeHigh (0x8) {
            #[32,28]
            timestamp: u32
        },
        ExtTrigger (0xa) {
            #[54,6]
            timestamp: u16,
            #[40,5]
            id: u8,
            #[32,1]
            polarity: u8
        }
    }
}

#[derive(Debug, Default)]
pub struct Evt21Decoder {
    time_high: Option<u64>,
}

impl EventDecoder for Evt21Decoder {
    fn decode<'a>(&'a mut self, reader: &'a mut impl Read) -> Box<dyn Iterator<Item = Event> + 'a> {
        Box::new(
            std::iter::from_fn(move || {
                let mut buffer = [0u8; 8];
                match reader.read_exact(&mut buffer) {
                    Ok(()) => {
                        let mut events = StackVec::<[Event; 32]>::new();

                        // Convert byte chunk into raw event
                        let raw_event: Evt21 = buffer.as_slice().into();

                        match raw_event {
                            Evt21::EvtNeg {
                                timestamp,
                                x,
                                y,
                                valid,
                            } => {
                                // Compute the full timestamp
                                if let Some(time_high) = self.time_high {
                                    let time_low = timestamp as u64;
                                    let full_timestamp = time_high | time_low;
                                    let mut mask = valid;
                                    while mask != 0 {
                                        let offset = mask.trailing_zeros();
                                        // Clear the lowest set bit
                                        mask = mask & (mask - 1);
                                        events.push(Event::CD {
                                            x: x + offset as u16,
                                            y,
                                            p: 0,
                                            t: full_timestamp,
                                        });
                                    }
                                }
                            }
                            Evt21::EvtPos {
                                timestamp,
                                x,
                                y,
                                valid,
                            } => {
                                // Compute the full timestamp
                                if let Some(time_high) = self.time_high {
                                    let time_low = timestamp as u64;
                                    let full_timestamp = time_high | time_low;
                                    let mut mask = valid;
                                    while mask != 0 {
                                        let offset = mask.trailing_zeros();
                                        // Clear the lowest set bit
                                        mask = mask & (mask - 1);
                                        events.push(Event::CD {
                                            x: x + offset as u16,
                                            y,
                                            p: 1,
                                            t: full_timestamp,
                                        });
                                    }
                                }
                            }
                            Evt21::EvtTimeHigh { timestamp } => {
                                self.time_high = Some((timestamp as u64) << 6)
                            }
                            Evt21::ExtTrigger {
                                timestamp: _, // TODO: propagate external trigger
                                id,
                                polarity,
                            } => events.push(Event::ExternalTrigger { id, p: polarity }),
                            _ => events.push(Event::Unknown),
                        } // end match type of event

                        Some(events.into_iter())
                    }
                    Err(_e) => None,
                }
            })
            .flatten(),
        )
    }
}
