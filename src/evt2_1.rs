use crate::{declare_raw_evt, EventBatch, EventDecoder};

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

#[derive(Default)]
pub struct Evt21Decoder {
    time_high: Option<u64>,
}

impl EventDecoder for Evt21Decoder {
    type RawEventType = Evt21;

    fn new() -> Self {
        Self::default()
    }

    fn decode(&mut self, raw_event: &[Self::RawEventType], batch: &mut EventBatch) {
        raw_event.iter().for_each(|evt| {
            match evt.event_type() {
                EVT_NEG | EVT_POS if self.time_high.is_some() => {
                    let full_timestamp = self.time_high.unwrap() | evt.timestamp();
                    let p = evt.event_type();
                    let base_x = evt.x();
                    let y = evt.y();
                    let mut mask = evt.valid_mask();
                    while mask != 0 {
                        let offset = mask.trailing_zeros() as u16;
                        // Clear the lowest set bit
                        mask &= mask - 1;
                        batch.push(base_x + offset, y, full_timestamp, p);
                    }
                }
                EVT_TIME_HIGH => self.time_high = Some(evt.time_high() << 6),
                _ => {}
            }
        });
    }
}
