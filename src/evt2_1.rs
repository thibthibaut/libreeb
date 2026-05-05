use crate::{declare_raw_evt, EventBatch, EventDecoder};
use wide::u64x4;

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
    time_high: u64,
    has_time_high: bool,
}

impl EventDecoder for Evt21Decoder {
    type RawEventType = Evt21;

    fn new() -> Self {
        Self::default()
    }

    fn decode(&mut self, raw_events: &[Self::RawEventType], batch: &mut EventBatch) {
        // Phase 1: fast-forward to first TIME_HIGH
        let events: &[Evt21] = if !self.has_time_high {
            let Some(pos) = raw_events
                .iter()
                .position(|e| e.event_type() == EVT_TIME_HIGH)
            else {
                return;
            };
            self.time_high = raw_events[pos].time_high() << 6;
            self.has_time_high = true;
            &raw_events[pos + 1..]
        } else {
            raw_events
        };

        // Worst-case reserve: every word is a CD event with all 32 mask bits set
        batch.x.reserve(events.len() * 32);
        batch.y.reserve(events.len() * 32);
        batch.p.reserve(events.len() * 32);
        batch.t.reserve(events.len() * 32);

        let mut time_high = self.time_high;

        let words: &[u64] = bytemuck::cast_slice(events);
        let mut i = 0usize;

        // --- SIMD bulk path: 4 words at a time ---
        while i + 4 <= words.len() {
            let chunk = u64x4::new([words[i], words[i + 1], words[i + 2], words[i + 3]]);

            // Extract event types for all 4 lanes: bits [63:60]
            let types = (chunk >> u64x4::splat(60)).to_array();

            // If any lane is TIME_HIGH, fall back to scalar for this chunk.
            // TIME_HIGH is ~0.1% of events so this branch is almost never taken.
            if types[0] == EVT_TIME_HIGH as u64
                || types[1] == EVT_TIME_HIGH as u64
                || types[2] == EVT_TIME_HIGH as u64
                || types[3] == EVT_TIME_HIGH as u64
            {
                for j in i..i + 4 {
                    decode_one(words[j], &mut time_high, batch);
                }
                i += 4;
                continue;
            }

            // All 4 lanes are CD (or unknown) — extract all fields in parallel
            let t_lows = ((chunk >> u64x4::splat(54)) & u64x4::splat(0x3F)).to_array();
            let xs = ((chunk >> u64x4::splat(43)) & u64x4::splat(0x7FF)).to_array();
            let ys = ((chunk >> u64x4::splat(32)) & u64x4::splat(0x7FF)).to_array();
            let masks = (chunk & u64x4::splat(0xFFFF_FFFF)).to_array();

            // Scatter: expand valid_mask per lane
            for lane in 0..4 {
                // Skip non-CD event types
                if types[lane] > EVT_POS as u64 {
                    continue;
                }
                let full_t = time_high | t_lows[lane];
                let p = types[lane] as u8; // 0 = NEG, 1 = POS
                let base_x = xs[lane] as u16;
                let y = ys[lane] as u16;
                let mut mask = masks[lane] as u32;
                while mask != 0 {
                    let offset = mask.trailing_zeros() as u16;
                    mask &= mask - 1;
                    batch.x.push(base_x + offset);
                    batch.y.push(y);
                    batch.p.push(p);
                    batch.t.push(full_t);
                }
            }

            i += 4;
        }

        // --- Scalar tail for remaining < 4 words ---
        while i < words.len() {
            decode_one(words[i], &mut time_high, batch);
            i += 1;
        }

        self.time_high = time_high;
    }
}

#[inline(always)]
fn decode_one(w: u64, time_high: &mut u64, batch: &mut EventBatch) {
    match (w >> 60) as u8 {
        EVT_NEG | EVT_POS => {
            let p = (w >> 60) as u8;
            let full_t = *time_high | ((w >> 54) & 0x3F);
            let base_x = ((w >> 43) & 0x7FF) as u16;
            let y = ((w >> 32) & 0x7FF) as u16;
            let mut mask = (w & 0xFFFF_FFFF) as u32;
            while mask != 0 {
                let offset = mask.trailing_zeros() as u16;
                mask &= mask - 1;
                batch.x.push(base_x + offset);
                batch.y.push(y);
                batch.p.push(p);
                batch.t.push(full_t);
            }
        }
        EVT_TIME_HIGH => {
            *time_high = ((w >> 32) & 0x0FFF_FFFF) << 6;
        }
        _ => {}
    }
}
