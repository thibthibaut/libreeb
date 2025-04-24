use crate::{define_raw_evt, Event, EventDecoder};
use std::io::Read;

// EVT2.1 raw events definition, the layout is:
//
// 63        60 59                         0
// +---4 bits--+-------60 bits------------+
// | Event Type|           Payload        |
// +-----------+--------------------------+
define_raw_evt! {
    #[storage(u32), size(4),  discriminant(28, 4)]
    enum Evt2 {
        CDoff(0x0) {
            #[22,6]
            timestamp: u16,
            #[11,11]
            x: u16,
            #[0,11]
            y: u16
        },
        CDon(0x1) {
            #[22,6]
            timestamp: u16,
            #[21,11]
            x: u16,
            #[0,11]
            y: u16
        },
        EvtTimeHigh (0x8) {
            #[0,28]
            timestamp: u32
        },
        ExtTrigger (0xa) {
            #[22,6]
            timestamp: u16,
            #[8,5]
            id: u8,
            #[0,1]
            polarity: u8
        }
    }
}

#[derive(Debug, Default)]
pub struct Evt2Decoder {
    time_high: Option<u64>,
}

impl EventDecoder for Evt2Decoder {
    fn decode<'a>(&'a mut self, reader: &'a mut impl Read) -> Box<dyn Iterator<Item = Event> + 'a> {
        Box::new(std::iter::from_fn(move || {
            loop {
                let mut buffer = [0u8; 4];
                match reader.read_exact(&mut buffer) {
                    Ok(()) => {
                        // Convert byte chunk into raw event
                        let raw_event: Evt2 = buffer.as_slice().into();
                        let decoded_event = match raw_event {
                            Evt2::CDon { timestamp, x, y } => {
                                // Compute the full timestamp
                                if let Some(time_high) = self.time_high {
                                    let time_low = timestamp as u64;
                                    let full_timestamp = time_high | time_low;
                                    Some(Event::CD {
                                        x,
                                        y,
                                        p: 1,
                                        t: full_timestamp,
                                    })
                                } else {
                                    None
                                }
                            }
                            Evt2::CDoff { timestamp, x, y } => {
                                // Compute the full timestamp
                                if let Some(time_high) = self.time_high {
                                    let time_low = timestamp as u64;
                                    let full_timestamp = time_high | time_low;
                                    Some(Event::CD {
                                        x,
                                        y,
                                        p: 0,
                                        t: full_timestamp,
                                    })
                                } else {
                                    None
                                }
                            }
                            Evt2::EvtTimeHigh { timestamp } => {
                                self.time_high = Some((timestamp as u64) << 6);
                                None
                            }
                            Evt2::ExtTrigger {
                                timestamp: _, // TODO: propagate external trigger
                                id,
                                polarity,
                            } => Some(Event::ExternalTrigger { id, p: polarity }),
                            _ => Some(Event::Unknown),
                        }; // end match type of event

                        if decoded_event.is_some() {
                            return decoded_event;
                        }
                    }
                    Err(_e) => return None,
                }
            }
        }))
    }
}
