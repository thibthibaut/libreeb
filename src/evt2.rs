use crate::declare_raw_evt;
use crate::{EventBatch, EventDecoder};

// EVT2 raw events definition, the layout is:
//
// 32        28 27                        0
// +---4 bits--+-------28 bits------------+
// | Event Type|           Payload        |
// +-----------+--------------------------+
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

const NUM_BITS_IN_TIMESTAMP_LSB: u64 = 6;
const _MAX_TIMESTAMP: u64 = ((1 << 28) - 1) << NUM_BITS_IN_TIMESTAMP_LSB;
const _LOOP_THRESHOLD: u64 = 10000;
const _TIME_LOOP: u64 = _MAX_TIMESTAMP + (1 << NUM_BITS_IN_TIMESTAMP_LSB);

const CD_OFF: u8 = 0b0000;
const CD_ON: u8 = 0b0001;
const EVT_TIME_HIGH: u8 = 0b1000;

#[derive(Debug, Default)]
pub struct Evt2Decoder {
    time_high: Option<u64>,
}

impl EventDecoder for Evt2Decoder {
    type RawEventType = Evt2;

    fn new() -> Self {
        Self::default()
    }

    fn decode(&mut self, raw_event: &[Self::RawEventType], batch: &mut EventBatch) {
        raw_event.iter().for_each(|evt| {
            match evt.event_type() {
                CD_ON | CD_OFF if self.time_high.is_some() => {
                    let full_timestamp = self.time_high.unwrap() | evt.time_low();
                    batch.push(evt.x(), evt.y(), full_timestamp, evt.event_type());
                }
                EVT_TIME_HIGH => {
                    self.time_high = Some(evt.time_high() << NUM_BITS_IN_TIMESTAMP_LSB)
                }
                _ => {}
            };
        });
    }
}
