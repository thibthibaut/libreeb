use crate::{declare_raw_evt, Event, EventDecoder};
use std::collections::VecDeque;
use zerocopy::{FromBytes, Immutable, KnownLayout};

// EVT2.1 raw events definition, the layout is:
//
// 63        60 59                         0
// +---4 bits--+-------60 bits------------+
// | Event Type|           Payload        |
// +-----------+--------------------------+
declare_raw_evt! {
    pub struct Evt21(u64);
    event_type(u8): 63, 60;
    time_high(u64): 59, 32;
    timestamp(u64): 59, 54;
    x(u16): 53, 43;
    y(u16): 42, 32;
    valid_mask(u32): 31, 0;
    trigger_channel_id(u8): 44, 40;
    trigger_value(u8): 32, 32;
}

const EVT_NEG: u8 = 0b0000;
const EVT_POS: u8 = 0b0001;
const EVT_TIME_HIGH: u8 = 0b1000;
const EXT_TRIGGER: u8 = 0b1010;
const _OTHERS: u8 = 0b1110;

#[derive(Default)]
pub struct Evt21Decoder {
    time_high: Option<u64>,
}

impl EventDecoder for Evt21Decoder {
    type RawEventType = Evt21;

    fn new() -> Self {
        Self::default()
    }

    fn decode(&mut self, raw_event: &[Self::RawEventType], event_queue: &mut VecDeque<Event>) {
        raw_event.iter().for_each(|evt| {
            match evt.event_type() {
                EVT_NEG | EVT_POS if self.time_high.is_some() => {
                    // Compute the full timestamp
                    let full_timestamp = self.time_high.unwrap() | evt.timestamp();
                    let mut mask = evt.valid_mask();
                    while mask != 0 {
                        let offset = mask.trailing_zeros();
                        // Clear the lowest set bit
                        mask = mask & (mask - 1);
                        event_queue.push_back(Event::CD {
                            x: evt.x() + offset as u16,
                            y: evt.y(),
                            p: evt.event_type(), // Use the event type for the polarity because CD_OFF is 0x0 and CD_ON is 0x1
                            t: full_timestamp,
                        });
                    }
                }
                EVT_TIME_HIGH => self.time_high = Some(evt.time_high() << 6),
                EXT_TRIGGER if self.time_high.is_some() => {
                    let full_timestamp = self.time_high.unwrap() | evt.timestamp();
                    event_queue.push_back(Event::ExternalTrigger {
                        id: evt.trigger_channel_id(),
                        p: evt.trigger_value(),
                        t: full_timestamp,
                    })
                }
                EVT_NEG | EVT_POS => {}
                EXT_TRIGGER => {}
                _ => event_queue.push_back(Event::Unknown()),
            }
        });
    }
}
