use crate::{declare_raw_evt, EventBatch, EventDecoder};
use wide::u16x8;

// Struct for holding raw EVT3 types
declare_raw_evt! {
    pub struct Evt3(u16);
    event_type(u8): 15, 12;
    y(u16): 10, 0;
    x(u16): 10, 0;
    pol(u8): 11, 11;
    _origin(u8): 11, 11;
    time(u16): 11, 0;
    valid(u16): 11, 0;
    trigger_id(u8): 11, 8;
    trigger_polarity(u8): 0, 0;
}

const EVT_ADDR_Y: u8 = 0x0;
const EVT_ADDR_X: u8 = 0x2;
const VECT_BASE_X: u8 = 0x3;
const VECT_12: u8 = 0x4;
const VECT_8: u8 = 0x5;
const EVT_TIME_LOW: u8 = 0x6;
const EVT_TIME_HIGH: u8 = 0x8;

const MAX_TIMESTAMP_BASE: u64 = ((1u64 << 12) - 1) << 12; // = 16773120us
const TIME_LOOP_DURATION_US: u64 = MAX_TIMESTAMP_BASE + (1 << 12); // = 16777216us
const LOOP_THRESHOLD: u64 = 10 << 12;

pub struct Evt3Decoder {
    time: u64,
    time_base: u64,
    has_time_base: bool,
    time_high_loop_nb: u32,
    polarity: u8,
    x: u16,
    y: u16,
}

impl EventDecoder for Evt3Decoder {
    type RawEventType = Evt3;

    fn new() -> Self {
        Evt3Decoder {
            time: 0,
            time_base: 0,
            has_time_base: false,
            time_high_loop_nb: 0,
            polarity: 0,
            x: 0,
            y: 0,
        }
    }

    fn decode(&mut self, raw_events: &[Evt3], batch: &mut EventBatch) {
        // Reinterpret the Evt3 slice as raw u16 words for direct indexed access.
        let words: &[u16] = bytemuck::cast_slice(raw_events);
        let n = words.len();
        let mut i = 0;

        while i < n {
            // ── SIMD fast path: detect a run of consecutive ADDR_X words ─────
            if i + 8 <= n && self.has_time_base {
                let chunk = u16x8::new(
                    words[i..i + 8].try_into().expect("slice is exactly 8 elements"),
                );
                // Isolate top 4 bits of each lane (the event_type field) and
                // compare to EVT_ADDR_X (0x2). Equal lanes → 0xFFFF.
                let types = chunk >> 12u16;
                let is_addr_x = types.simd_eq(u16x8::splat(EVT_ADDR_X as u16));
                // to_bitmask() collects the MSB of each lane; trailing_ones()
                // gives the length of the leading run of matching lanes.
                let run_len = is_addr_x.to_bitmask().trailing_ones() as usize;

                if run_len >= 4 {
                    // Extract x and p for the whole 8-lane chunk; we'll slice
                    // to [..run_len] when calling push_bulk.
                    let x_arr = (chunk & u16x8::splat(0x07FF)).to_array();
                    let p_arr = ((chunk >> 11u16) & u16x8::splat(0x0001)).to_array();
                    batch.push_bulk(&x_arr, &p_arr, self.y, self.time, run_len);
                    i += run_len;
                    continue;
                }
            }

            // ── Scalar path ──────────────────────────────────────────────────
            let w = words[i];
            let evt_type = (w >> 12) as u8;

            match evt_type {
                EVT_ADDR_Y => {
                    self.y = w & 0x07FF;
                }
                EVT_ADDR_X if self.has_time_base => {
                    // x  = bits 10:0,  pol = bit 11
                    batch.push(w & 0x07FF, self.y, self.time, (w >> 11) as u8 & 1);
                }
                EVT_ADDR_X => {}
                VECT_BASE_X => {
                    self.polarity = (w >> 11) as u8 & 1;
                    self.x = w & 0x07FF;
                }
                VECT_12 if self.has_time_base => {
                    let base_x = self.x;
                    // valid = bits 11:0 (12 bits)
                    let mut valid = (w & 0x0FFF) as u32;
                    while valid != 0 {
                        let offset = valid.trailing_zeros() as u16;
                        valid &= valid - 1;
                        batch.push(base_x + offset, self.y, self.time, self.polarity);
                    }
                    self.x = base_x + 12;
                }
                VECT_12 => {}
                VECT_8 if self.has_time_base => {
                    let base_x = self.x;
                    // valid = bits 7:0 (8 bits)
                    let mut valid = (w & 0x00FF) as u32;
                    while valid != 0 {
                        let offset = valid.trailing_zeros() as u16;
                        valid &= valid - 1;
                        batch.push(base_x + offset, self.y, self.time, self.polarity);
                    }
                    self.x = base_x + 8;
                }
                VECT_8 => {}
                EVT_TIME_LOW if self.has_time_base => {
                    self.time = self.time_base + (w & 0x0FFF) as u64;
                }
                EVT_TIME_LOW => {}
                EVT_TIME_HIGH => {
                    let event_time = (w & 0x0FFF) as u64;
                    if !self.has_time_base {
                        self.time_base = event_time << 12;
                        self.has_time_base = true;
                    } else {
                        let prev_time_base = self.time_base;
                        let mut new_time_high = event_time << 12;
                        new_time_high += self.time_high_loop_nb as u64 * TIME_LOOP_DURATION_US;

                        if (prev_time_base > new_time_high)
                            && (prev_time_base - new_time_high
                                >= MAX_TIMESTAMP_BASE - LOOP_THRESHOLD)
                        {
                            new_time_high += TIME_LOOP_DURATION_US;
                            self.time_high_loop_nb += 1;
                        }
                        self.time_base = new_time_high;
                        self.time = new_time_high;
                    }
                }
                _ => {}
            }

            i += 1;
        }
    }
}
