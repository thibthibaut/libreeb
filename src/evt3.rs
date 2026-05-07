use crate::evt_reader::RawFormatDecoder;
use crate::{declare_raw_evt, DecodeInto, Stage};
use wide::u16x8;

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

const VECT_BASE_X: u8 = 0x3;
const VECT_12: u8 = 0x4;
const VECT_8: u8 = 0x5;
const EVT_TIME_LOW: u8 = 0x6;
const EVT_TIME_HIGH: u8 = 0x8;
const EXT_TRIGGER: u8 = 0xA;

const MAX_TIMESTAMP_BASE: u64 = ((1u64 << 12) - 1) << 12;
const TIME_LOOP_DURATION_US: u64 = MAX_TIMESTAMP_BASE + (1 << 12);
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

impl RawFormatDecoder for Evt3Decoder {
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

    fn current_time(&self) -> u64 {
        self.time
    }

    fn decode<O: DecodeInto>(
        &mut self,
        raw_events: &[Evt3],
        output: &mut O,
        end_time: u64,
        max_events: usize,
    ) -> usize {
        let words: &[u16] = bytemuck::cast_slice(raw_events);
        let n = words.len();
        let mut i = 0usize;

        if !self.has_time_base {
            while i < n {
                let w = words[i];
                i += 1;
                if (w >> 12) as u8 == EVT_TIME_HIGH {
                    self.handle_time_high(w);
                    break;
                }
            }
            if !self.has_time_base {
                return i;
            }
        }

        let mut stage = Stage::new();
        let mut decoded_count = 0usize;

        let mut cur_y = self.y;
        let mut cur_t = self.time;
        let mut cur_x = self.x;
        let mut cur_pol = self.polarity;
        let mut time_base = self.time_base;

        // Inline flush helper to avoid closure overhead
        #[inline(always)]
        fn flush_and_count<O: DecodeInto>(stage: &mut Stage, output: &mut O, count: &mut usize) {
            *count += stage.len();
            stage.flush(output);
        }

        macro_rules! emit {
            ($x:expr, $y:expr, $p:expr, $t:expr) => {
                if stage.is_full() {
                    flush_and_count(&mut stage, output, &mut decoded_count);
                }
                stage.push_unchecked($x, $y, $p, $t);
            };
        }

        // ── SIMD bulk path: process runs of EVT_ADDR_X (8 at a time) ─────
        while i + 8 <= n {
            // Pre-compute budget to avoid repeated addition
            let current_total = decoded_count + stage.len();
            if current_total >= max_events.saturating_sub(8) {
                break;
            }

            // Load 8 words
            let arr: [u16; 8] = words[i..i + 8].try_into().unwrap();
            let simd = u16x8::new(arr);

            // Extract top 4 bits for event type classification
            let types = (simd >> 12_u32).to_array();

            // Check if ALL 8 words are EVT_ADDR_X (type == 0x2)
            if types == [2, 2, 2, 2, 2, 2, 2, 2] {
                // SIMD fast path: extract x and p for all 8 events
                let x_vals = (simd & u16x8::splat(0x07FF)).to_array();
                let p_raw = ((simd >> 11_u32) & u16x8::splat(1)).to_array();
                
                // Extract p values as u8 array
                let p_vals: [u8; 8] = [
                    p_raw[0] as u8, p_raw[1] as u8, p_raw[2] as u8, p_raw[3] as u8,
                    p_raw[4] as u8, p_raw[5] as u8, p_raw[6] as u8, p_raw[7] as u8,
                ];

                // Flush stage if needed and write 8 events at once
                if stage.len() + 8 > crate::STAGE_CAP {
                    flush_and_count(&mut stage, output, &mut decoded_count);
                }
                stage.push_8(x_vals, cur_y, p_vals, cur_t);
                i += 8;
                continue;
            }

            // Not all EVT_ADDR_X — fall to scalar
            break;
        }

        // ── Scalar path: process remaining words one at a time ───────────
        while i < n {
            let w = words[i];
            i += 1;

            // Fast path: EVT_ADDR_X (type 0x2, words 0x2000..0x3000)
            // This is the most common case, so optimize it first
            if w >= 0x2000 && w < 0x3000 {
                // Check budget before emitting
                if decoded_count + stage.len() >= max_events {
                    i -= 1; // Back up to reprocess this word
                    break;
                }
                
                emit!(w & 0x07FF, cur_y, (w >> 11) as u8 & 1, cur_t);

                // Try SIMD again if we have a run
                while i + 8 <= n {
                    let current_total = decoded_count + stage.len();
                    if current_total >= max_events.saturating_sub(8) {
                        break;
                    }

                    let arr: [u16; 8] = words[i..i + 8].try_into().unwrap();
                    let simd = u16x8::new(arr);
                    let types = (simd >> 12_u32).to_array();
                    
                    if types == [2, 2, 2, 2, 2, 2, 2, 2] {
                        let x_vals = (simd & u16x8::splat(0x07FF)).to_array();
                        let p_raw = ((simd >> 11_u32) & u16x8::splat(1)).to_array();
                        let p_vals: [u8; 8] = [
                            p_raw[0] as u8, p_raw[1] as u8, p_raw[2] as u8, p_raw[3] as u8,
                            p_raw[4] as u8, p_raw[5] as u8, p_raw[6] as u8, p_raw[7] as u8,
                        ];
                        
                        if stage.len() + 8 > crate::STAGE_CAP {
                            flush_and_count(&mut stage, output, &mut decoded_count);
                        }
                        stage.push_8(x_vals, cur_y, p_vals, cur_t);
                        i += 8;
                    } else {
                        break;
                    }
                }
                continue;
            }

            // EVT_ADDR_Y (type 0x0, words < 0x1000)
            if w < 0x1000 {
                cur_y = w & 0x07FF;
                // Peek: if next word is EVT_ADDR_X, emit immediately
                if i < n {
                    let next = words[i];
                    if next >= 0x2000 && next < 0x3000 {
                        if decoded_count + stage.len() < max_events {
                            emit!(next & 0x07FF, cur_y, (next >> 11) as u8 & 1, cur_t);
                            i += 1;
                        }
                    }
                }
                continue;
            }

            // Less common types: match on top 4 bits
            let event_type = (w >> 12) as u8;
            
            match event_type {
                VECT_BASE_X => {
                    cur_pol = (w >> 11) as u8 & 1;
                    cur_x = w & 0x07FF;
                }
                VECT_12 => {
                    let mut mask = (w & 0x0FFF) as u32;
                    while mask != 0 {
                        if decoded_count + stage.len() >= max_events {
                            i -= 1; // Back up
                            break;
                        }
                        let bit = mask.trailing_zeros() as u16;
                        mask &= mask - 1;
                        emit!(cur_x + bit, cur_y, cur_pol, cur_t);
                    }
                    if decoded_count + stage.len() >= max_events {
                        break;
                    }
                    cur_x += 12;
                }
                VECT_8 => {
                    let mut mask = (w & 0x00FF) as u32;
                    while mask != 0 {
                        if decoded_count + stage.len() >= max_events {
                            i -= 1; // Back up
                            break;
                        }
                        let bit = mask.trailing_zeros() as u16;
                        mask &= mask - 1;
                        emit!(cur_x + bit, cur_y, cur_pol, cur_t);
                    }
                    if decoded_count + stage.len() >= max_events {
                        break;
                    }
                    cur_x += 8;
                }
                EVT_TIME_LOW => {
                    let low = (w & 0x0FFF) as u64;
                    let prev_low = cur_t & 0x0FFF;
                    if low < prev_low {
                        time_base += 1 << 12;
                        self.time_base = time_base;
                    }
                    cur_t = time_base | low;
                    // Time budget check
                    if cur_t >= end_time {
                        i -= 1; // don't consume this word — next slice starts here
                        break;
                    }
                }
                EVT_TIME_HIGH => {
                    self.y = cur_y;
                    self.x = cur_x;
                    self.polarity = cur_pol;
                    self.time = cur_t;
                    self.handle_time_high(w);
                    cur_t = self.time;
                    time_base = self.time_base;
                    cur_y = self.y;
                    cur_x = self.x;
                    cur_pol = self.polarity;
                    // Time budget check
                    if cur_t >= end_time {
                        break;
                    }
                }
                EXT_TRIGGER => {
                    // EVT3 trigger: id in bits 11:8, polarity in bit 0
                    let id = ((w >> 8) & 0xF) as u8;
                    let pol = (w & 1) as u8;
                    // Flush stage before emitting trigger (preserve ordering)
                    flush_and_count(&mut stage, output, &mut decoded_count);
                    output.push_trigger(id, pol, cur_t);
                }
                _ => {}
            }
        }

        stage.flush(output);

        self.y = cur_y;
        self.time = cur_t;
        self.x = cur_x;
        self.polarity = cur_pol;
        self.time_base = time_base;

        // i = number of u16 words consumed = number of Evt3 raw events consumed
        i
    }
}

impl Evt3Decoder {
    #[cold]
    #[inline(never)]
    fn handle_time_high(&mut self, w: u16) {
        let event_time = (w & 0x0FFF) as u64;

        if !self.has_time_base {
            self.time_base = event_time << 12;
            self.has_time_base = true;
            self.time = self.time_base;
            return;
        }

        let prev = self.time_base;
        let mut new_t =
            (event_time << 12) + self.time_high_loop_nb as u64 * TIME_LOOP_DURATION_US;

        if prev > new_t && prev - new_t >= MAX_TIMESTAMP_BASE - LOOP_THRESHOLD {
            new_t += TIME_LOOP_DURATION_US;
            self.time_high_loop_nb += 1;
        }

        self.time_base = new_t;
        self.time = new_t;
    }
}
