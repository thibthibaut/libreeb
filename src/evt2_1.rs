use crate::{Event, EventDecoder};
use std::collections::VecDeque;
use zerocopy::{FromBytes, Immutable, KnownLayout};

// EVT2.1 raw events definition, the layout is:
//
// 63        60 59                         0
// +---4 bits--+-------60 bits------------+
// | Event Type|           Payload        |
// +-----------+--------------------------+
#[derive(FromBytes, Immutable, KnownLayout, Copy, Clone)]
#[repr(C)]
pub struct Evt21 {
    data: u64,
}

const EVT_NEG: u8 = 0b0000;
const EVT_POS: u8 = 0b0001;
const EVT_TIME_HIGH: u8 = 0b1000;
const EXT_TRIGGER: u8 = 0b1010;
const _OTHERS: u8 = 0b1110;

impl Evt21 {
    /// Extracts the event type
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
                    let full_timestamp = self.time_high.unwrap() | evt.time_low();
                    let mut mask = evt.valid_mask();
                    while mask != 0 {
                        let offset = mask.trailing_zeros();
                        // Clear the lowest set bit
                        mask = mask & (mask - 1);
                        event_queue.push_back(Event::CD {
                            x: evt.x() + offset as u16,
                            y: evt.y(),
                            p: evt.event_type(),
                            t: full_timestamp,
                        });
                    }
                }
                EVT_TIME_HIGH => self.time_high = Some(evt.time_high()),
                EXT_TRIGGER => event_queue.push_back(Event::ExternalTrigger {
                    id: evt.trigger_channel_id(),
                    p: evt.trigger_value(),
                }),
                EVT_NEG | EVT_POS => {}
                _ => event_queue.push_back(Event::Unknown),
            }
        });
    }
}
