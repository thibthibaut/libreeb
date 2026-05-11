//! EVT2.1 format decoder.
//!
//! EVT2.1 is an improved version of EVT2 with mask-based event encoding.
//! Specification: https://docs.prophesee.ai/stable/data/encoding_formats/evt21.html

use crate::io::reader::RawFormatDecoder;
use crate::{declare_raw_evt, DecodeInto};

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

// Event type constants
const EVT_NEG: u8 = 0b0000;
const EVT_POS: u8 = 0b0001;
const EVT_TIME_HIGH: u8 = 0b1000;
const EXT_TRIGGER: u8 = 0xA;

#[derive(Default)]
pub struct Evt21Decoder {
    time_high: u64,
    has_time_high: bool,
    // Carry state for partial mask emission (when count budget splits a word)
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

        // Fast-forward to first TIME_HIGH to establish time base
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

        // Main decoding loop - simple and sequential
        for (i, &word) in words.iter().enumerate() {
            // Check budgets
            if decoded_count >= max_events {
                self.time_high = time_high;
                return offset + i;
            }

            let event_type = (word >> 60) as u8;

            match event_type {
                // EVT_NEG (0b0000) or EVT_POS (0b0001): CD events with mask
                EVT_NEG | EVT_POS => {
                    let t_low = (word >> 54) & 0x3F;
                    let full_t = time_high | t_low;

                    // Check time budget
                    if full_t >= end_time {
                        self.time_high = time_high;
                        return offset + i;
                    }

                    let p = event_type;
                    let base_x = ((word >> 43) & 0x7FF) as u16;
                    let y = ((word >> 32) & 0x7FF) as u16;
                    let mask = (word & 0xFFFF_FFFF) as u32;

                    if mask == 0 {
                        continue;
                    }

                    // Expand mask into x coordinates
                    let count = expand_mask(&mut x_buf, base_x, mask);

                    // Check if we can emit all events from this word
                    if decoded_count + count > max_events {
                        // Partial emit: emit what we can, save the rest as carry
                        let remaining = max_events - decoded_count;
                        output.extend_constant_ypt(&x_buf[..remaining], y, p, full_t);

                        // Compute remaining mask (skip first `remaining` set bits)
                        let mut carry_mask = mask;
                        for _ in 0..remaining {
                            carry_mask &= carry_mask - 1;
                        }

                        self.carry_mask = carry_mask;
                        self.carry_base_x = base_x;
                        self.carry_y = y;
                        self.carry_p = p;
                        self.carry_t = full_t;
                        self.time_high = time_high;
                        return offset + i + 1;
                    }

                    // Emit all events from this word
                    output.extend_constant_ypt(&x_buf[..count], y, p, full_t);
                    decoded_count += count;
                }

                // EVT_TIME_HIGH (0b1000): Update time base
                EVT_TIME_HIGH => {
                    time_high = ((word >> 32) & 0x0FFF_FFFF) << 6;
                }

                // EXT_TRIGGER (0xA): External trigger event
                EXT_TRIGGER => {
                    let channel = ((word >> 40) & 0x1F) as u8;
                    let value = ((word >> 32) & 1) as u8;
                    let t_low = (word >> 54) & 0x3F;
                    let full_t = time_high | t_low;
                    output.push_trigger(channel, value, full_t);
                }

                // Unknown event types: skip
                _ => {}
            }
        }

        self.time_high = time_high;
        offset + words.len()
    }
}

/// Expand a 32-bit mask into x coordinates.
///
/// For each set bit in `mask`, computes `base_x + bit_position` and stores
/// it in `x_buf`. Returns the number of set bits (events).
///
/// Example: If mask is 0b1011 (bits 0, 1, 3 set) and base_x is 100,
/// this fills x_buf with [100, 101, 103] and returns 3.
#[inline(always)]
fn expand_mask(x_buf: &mut [u16; 32], base_x: u16, mut mask: u32) -> usize {
    let mut pos = 0;
    while mask != 0 {
        let offset = mask.trailing_zeros() as u16;
        mask &= mask - 1; // Clear lowest set bit
        x_buf[pos] = base_x + offset;
        pos += 1;
    }
    pos
}
