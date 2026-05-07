use crate::declare_raw_evt;
use crate::evt_reader::RawFormatDecoder;
use crate::DecodeInto;

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
const EXT_TRIGGER: u8 = 0xA;

#[derive(Debug, Default)]
pub struct Evt2Decoder {
    time_high: Option<u64>,
    current_time: u64,
}

impl RawFormatDecoder for Evt2Decoder {
    type RawEventType = Evt2;

    fn new() -> Self {
        Self::default()
    }

    fn current_time(&self) -> u64 {
        self.current_time
    }

    fn decode<O: DecodeInto>(
        &mut self,
        raw_events: &[Self::RawEventType],
        output: &mut O,
        end_time: u64,
        max_events: usize,
    ) -> usize {
        let mut decoded_count = 0usize;
        
        for (idx, evt) in raw_events.iter().enumerate() {
            if decoded_count >= max_events {
                return idx;
            }

            match evt.event_type() {
                CD_ON | CD_OFF if self.time_high.is_some() => {
                    let full_timestamp = self.time_high.unwrap() | evt.time_low();
                    if full_timestamp >= end_time {
                        return idx;
                    }
                    self.current_time = full_timestamp;
                    output.push_cd(evt.x(), evt.y(), evt.event_type(), full_timestamp);
                    decoded_count += 1;
                }
                EVT_TIME_HIGH => {
                    let th = evt.time_high() << NUM_BITS_IN_TIMESTAMP_LSB;
                    self.time_high = Some(th);
                    self.current_time = th;
                    if th >= end_time {
                        return idx;
                    }
                }
                EXT_TRIGGER if self.time_high.is_some() => {
                    let full_timestamp = self.time_high.unwrap() | evt.time_low();
                    self.current_time = full_timestamp;
                    output.push_trigger(
                        evt.trigger_channel_id(),
                        evt.trigger_value(),
                        full_timestamp,
                    );
                }
                _ => {}
            }
        }
        raw_events.len()
    }
}
