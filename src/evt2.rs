use crate::declare_raw_evt;
use crate::{Event, EventDecoder};
use std::collections::VecDeque;
use zerocopy::{FromBytes, Immutable, KnownLayout};

declare_raw_evt! {
    pub struct Evt2(u32);

    event_type(u8): 31, 28;
    x(u16): 21, 11;
    y(u16): 10, 0;
    time_low(u64): 27, 22;
    time_high(u64): 27, 0;
    trigger_channel_id(u8): 12, 8;
    trigger_value(u8): 1, 0;
}

// EVT2 raw events definition, the layout is:
//
// 32        28 27                        0
// +---4 bits--+-------28 bits------------+
// | Event Type|           Payload        |
// +-----------+--------------------------+
// #[derive(FromBytes, Immutable, KnownLayout, Copy, Clone)]
// #[repr(C)]
// pub struct Evt2 {
//     data: u32,
// }

const NUM_BITS_IN_TIMESTAMP_LSB: u64 = 6;
const MAX_TIMESTAMP: u64 = ((1 << 28) - 1) << NUM_BITS_IN_TIMESTAMP_LSB;
const LOOP_THRESHOLD: u64 = 10000;
const TIME_LOOP: u64 = MAX_TIMESTAMP + (1 << NUM_BITS_IN_TIMESTAMP_LSB);

const CD_OFF: u8 = 0b0000;
const CD_ON: u8 = 0b0001;
const EVT_TIME_HIGH: u8 = 0b1000;
const EXT_TRIGGER: u8 = 0b1010;
const _OTHERS: u8 = 0b1110;
const _CONTINUED: u8 = 0b1111;

#[derive(Debug, Default)]
pub struct Evt2Decoder {
    time_high: Option<u64>,
}

/*jjjjjjjj
impl Evt2 {
    /// Extracts the event type from bits 31:28
    fn event_type(&self) -> u8 {
        ((self.data >> 28) & 0xF) as u8
    }

    /// Extracts the x-coordinate from bits 27:17 (11 bits)
    fn x(&self) -> u16 {
        ((self.data >> 17) & 0x7FF) as u16
    }

    /// Extracts the y-coordinate from bits 16:6 (11 bits)
    fn y(&self) -> u16 {
        ((self.data >> 6) & 0x7FF) as u16
    }

    /// Extracts the timestamp low bits from bits 5:0 (6 bits)
    fn time_low(&self) -> u64 {
        (self.data & 0x3F) as u64
    }

    /// Extracts the high timestamp value for EVT_TIME_HIGH events
    /// Uses bits 27:0 (28 bits)
    fn time_high(&self) -> u64 {
        ((self.data & 0x0FFFFFFF) << NUM_BITS_IN_TIMESTAMP_LSB) as u64
    }

    /// Extracts the trigger channel ID from an EXT_TRIGGER event
    /// Uses bits 27:23 (5 bits)
    fn trigger_channel_id(&self) -> u8 {
        ((self.data >> 23) & 0x1F) as u8
    }

    /// Extracts the trigger value (edge polarity) from an EXT_TRIGGER event
    /// Uses bit 22: 1 for rising edge, 0 for falling edge
    fn trigger_value(&self) -> u8 {
        ((self.data >> 22) & 0x1) as u8
    }
}

*/
impl EventDecoder for Evt2Decoder {
    type RawEventType = Evt2;

    fn new() -> Self {
        Self::default()
    }

    fn decode(&mut self, raw_event: &[Self::RawEventType], event_queue: &mut VecDeque<Event>) {
        raw_event.iter().for_each(|evt| {
            match evt.event_type() {
                CD_ON | CD_OFF if self.time_high.is_some() => {
                    let full_timestamp = self.time_high.unwrap() | evt.time_low();
                    event_queue.push_back(Event::CD {
                        x: evt.x(),
                        y: evt.y(),
                        p: evt.event_type(),
                        t: full_timestamp,
                    })
                }
                EVT_TIME_HIGH => {
                    self.time_high = Some(evt.time_high() << NUM_BITS_IN_TIMESTAMP_LSB)
                }
                EXT_TRIGGER => event_queue.push_back(Event::ExternalTrigger {
                    id: evt.trigger_channel_id(),
                    p: evt.trigger_value(),
                    t: 0,
                }),
                CD_ON | CD_OFF => {}
                _ => event_queue.push_back(Event::Unknown()),
            }; // end match type of event
        });
    }
}
