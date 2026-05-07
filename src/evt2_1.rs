use crate::evt_reader::RawFormatDecoder;
use crate::{declare_raw_evt, DecodeInto};
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
const EXT_TRIGGER: u8 = 0xA;

#[derive(Default)]
pub struct Evt21Decoder {
    time_high: u64,
    has_time_high: bool,
    // Carry state for partial mask emission (count-budget split within a word)
    carry_mask: u32,
    carry_base_x: u16,
    carry_y: u16,
    carry_p: u8,
    carry_t: u64,
}

impl RawFormatDecoder for Evt21Decoder {
    type RawEventType = Evt21;

    fn new() -> Self {
        Self::default()
    }

    fn current_time(&self) -> u64 {
        self.time_high
    }

    fn decode<O: DecodeInto>(
        &mut self,
        raw_events: &[Self::RawEventType],
        output: &mut O,
        end_time: u64,
        max_events: usize,
    ) -> usize {
        let mut x_buf: [u16; 32] = [0; 32];
        let mut decoded_count = 0usize;

        // Drain carry from previous partial word first
        if self.carry_mask != 0 {
            let remaining = max_events.saturating_sub(decoded_count);
            if remaining == 0 {
                return 0;
            }
            let count = expand_mask(&mut x_buf, self.carry_base_x, self.carry_mask);
            let emit = count.min(remaining);
            output.extend_constant_ypt(&x_buf[..emit], self.carry_y, self.carry_p, self.carry_t);
            decoded_count += emit;
            if emit < count {
                // Still can't emit all — advance carry past emitted bits
                let mut mask = self.carry_mask;
                for _ in 0..emit {
                    mask &= mask - 1;
                }
                self.carry_mask = mask;
                return 0;
            }
            self.carry_mask = 0;
        }

        // Phase 1: fast-forward to first TIME_HIGH
        let (events, offset) = if !self.has_time_high {
            let Some(pos) = raw_events
                .iter()
                .position(|e| e.event_type() == EVT_TIME_HIGH)
            else {
                return raw_events.len();
            };
            self.time_high = raw_events[pos].time_high() << 6;
            self.has_time_high = true;
            (&raw_events[pos + 1..], pos + 1)
        } else {
            (raw_events, 0)
        };

        let mut time_high = self.time_high;
        let words: &[u64] = bytemuck::cast_slice(events);
        let mut i = 0usize;

        // Temporary x buffer for mask expansion (max 32 bits = 32 events per word)
        let mut x_buf: [u16; 32] = [0; 32];
        // Carry state for partial emission in decode_one_generic
        let mut carry: (u32, u16, u16, u8, u64) = (0, 0, 0, 0, 0);

        // --- SIMD bulk path: 4 words at a time ---
        while i + 4 <= words.len() {
            if decoded_count >= max_events {
                break;
            }

            let chunk = u64x4::new([words[i], words[i + 1], words[i + 2], words[i + 3]]);
            let types = (chunk >> u64x4::splat(60)).to_array();

            // If any lane is TIME_HIGH or EXT_TRIGGER, fall back to scalar for this chunk.
            if types[0] >= EVT_TIME_HIGH as u64
                || types[1] >= EVT_TIME_HIGH as u64
                || types[2] >= EVT_TIME_HIGH as u64
                || types[3] >= EVT_TIME_HIGH as u64
            {
                for j in 0..4 {
                    if decoded_count >= max_events || time_high >= end_time {
                        self.time_high = time_high;
                        self.save_carry(&carry);
                        return offset + i + j;
                    }
                    if !decode_one_generic(words[i + j], &mut time_high, output, &mut x_buf, end_time, max_events, &mut decoded_count, &mut carry) {
                        self.time_high = time_high;
                        return offset + i + j;
                    }
                    if carry.0 != 0 {
                        // Partial emit happened — word was consumed
                        self.time_high = time_high;
                        self.save_carry(&carry);
                        return offset + i + j + 1;
                    }
                }
                i += 4;
                continue;
            }

            // All 4 lanes are CD (or unknown) — extract fields in parallel
            let t_lows = ((chunk >> u64x4::splat(54)) & u64x4::splat(0x3F)).to_array();
            let xs = ((chunk >> u64x4::splat(43)) & u64x4::splat(0x7FF)).to_array();
            let ys = ((chunk >> u64x4::splat(32)) & u64x4::splat(0x7FF)).to_array();
            let masks = (chunk & u64x4::splat(0xFFFF_FFFF)).to_array();

            // Constant-field batching per lane
            for lane in 0..4 {
                if types[lane] > EVT_POS as u64 {
                    continue;
                }
                if decoded_count >= max_events {
                    self.time_high = time_high;
                    return offset + i + lane;
                }

                let full_t = time_high | t_lows[lane];
                if full_t >= end_time {
                    self.time_high = time_high;
                    return offset + i + lane;
                }

                let p = types[lane] as u8;
                let base_x = xs[lane] as u16;
                let y = ys[lane] as u16;
                let mask = masks[lane] as u32;

                if mask == 0 {
                    continue;
                }

                // Expand mask into x buffer
                let count = expand_mask(&mut x_buf, base_x, mask);

                // If emitting all would exceed budget, partial emit + carry
                if decoded_count + count > max_events {
                    let remaining = max_events - decoded_count;
                    output.extend_constant_ypt(&x_buf[..remaining], y, p, full_t);
                    // Store carry: remaining mask bits (skip first `remaining` set bits)
                    let mut carry = mask;
                    for _ in 0..remaining {
                        carry &= carry - 1;
                    }
                    self.carry_mask = carry;
                    self.carry_base_x = base_x;
                    self.carry_y = y;
                    self.carry_p = p;
                    self.carry_t = full_t;
                    self.time_high = time_high;
                    // Consume up to and INCLUDING this word
                    return offset + i + lane + 1;
                }

                // Constant-field batching via trait method
                output.extend_constant_ypt(&x_buf[..count], y, p, full_t);
                decoded_count += count;
            }

            i += 4;
        }

        // --- Scalar tail for remaining < 4 words ---
        while i < words.len() {
            if decoded_count >= max_events || time_high >= end_time {
                break;
            }
            if !decode_one_generic(words[i], &mut time_high, output, &mut x_buf, end_time, max_events, &mut decoded_count, &mut carry) {
                break;
            }
            if carry.0 != 0 {
                i += 1; // word was consumed (partial emit)
                break;
            }
            i += 1;
        }

        self.time_high = time_high;
        self.save_carry(&carry);
        offset + i
    }
}

impl Evt21Decoder {
    #[inline(always)]
    fn save_carry(&mut self, carry: &(u32, u16, u16, u8, u64)) {
        if carry.0 != 0 {
            self.carry_mask = carry.0;
            self.carry_base_x = carry.1;
            self.carry_y = carry.2;
            self.carry_p = carry.3;
            self.carry_t = carry.4;
        }
    }
}

/// Expand a 32-bit mask into x offsets. Returns number of set bits.
#[inline(always)]
fn expand_mask(x_buf: &mut [u16; 32], base_x: u16, mut mask: u32) -> usize {
    let mut pos = 0;
    while mask != 0 {
        let offset = mask.trailing_zeros() as u16;
        mask &= mask - 1;
        x_buf[pos] = base_x + offset;
        pos += 1;
    }
    pos
}

/// Decode a single word with constant-field batching (generic over output type).
/// Returns `(processed, carry)`:
/// - `true` = word fully processed (or skipped)
/// - `false` = budget hit, word NOT consumed (time budget)
///
/// If count budget splits mid-mask, sets carry fields and returns true
/// (word IS consumed since partial events were emitted).
#[inline(always)]
fn decode_one_generic<O: DecodeInto>(
    w: u64,
    time_high: &mut u64,
    output: &mut O,
    x_buf: &mut [u16; 32],
    end_time: u64,
    max_events: usize,
    decoded_count: &mut usize,
    carry: &mut (u32, u16, u16, u8, u64), // (mask, base_x, y, p, t)
) -> bool {
    match (w >> 60) as u8 {
        EVT_NEG | EVT_POS => {
            let p = (w >> 60) as u8;
            let full_t = *time_high | ((w >> 54) & 0x3F);

            if full_t >= end_time {
                return false; // don't consume — next slice starts here
            }

            let base_x = ((w >> 43) & 0x7FF) as u16;
            let y = ((w >> 32) & 0x7FF) as u16;
            let mask = (w & 0xFFFF_FFFF) as u32;

            if mask == 0 {
                return true;
            }

            let count = expand_mask(x_buf, base_x, mask);

            if *decoded_count + count > max_events {
                // Partial emit + carry
                let remaining = max_events - *decoded_count;
                output.extend_constant_ypt(&x_buf[..remaining], y, p, full_t);
                *decoded_count += remaining;
                let mut carry_mask = mask;
                for _ in 0..remaining {
                    carry_mask &= carry_mask - 1;
                }
                *carry = (carry_mask, base_x, y, p, full_t);
                return true; // word consumed (partially emitted)
            }

            output.extend_constant_ypt(&x_buf[..count], y, p, full_t);
            *decoded_count += count;
            true
        }
        EVT_TIME_HIGH => {
            *time_high = ((w >> 32) & 0x0FFF_FFFF) << 6;
            true
        }
        EXT_TRIGGER => {
            let full_t = *time_high | ((w >> 54) & 0x3F);
            let channel = ((w >> 40) & 0x1F) as u8;
            let value = ((w >> 32) & 1) as u8;
            output.push_trigger(channel, value, full_t);
            true
        }
        _ => true,
    }
}
