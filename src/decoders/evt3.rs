//! EVT3 format decoder.
//!
//! EVT3 is Prophesee's native event format for Gen4 sensors.
//! Specification: https://docs.prophesee.ai/stable/data/encoding_formats/evt3.html

use crate::io::reader::RawFormatDecoder;
use crate::{declare_raw_evt, DecodeInto, Stage};

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

// Event type constants
const VECT_BASE_X: u8 = 0x3;
const VECT_12: u8 = 0x4;
const VECT_8: u8 = 0x5;
const EVT_TIME_LOW: u8 = 0x6;
const EVT_TIME_HIGH: u8 = 0x8;
const EXT_TRIGGER: u8 = 0xA;

// Time handling constants
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

    fn output_capacity_hint(raw_events: usize) -> usize {
        // EVT3 has roughly 1 CD event per raw word on average
        raw_events
    }

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

        // Wait for first time_high to establish time base
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

        // Helper to flush stage and update count
        #[inline(always)]
        fn flush_and_count<O: DecodeInto>(stage: &mut Stage, output: &mut O, count: &mut usize) {
            *count += stage.len();
            stage.flush(output);
        }

        // Helper to emit an event to stage
        macro_rules! emit {
            ($x:expr, $y:expr, $p:expr, $t:expr) => {
                if stage.is_full() {
                    flush_and_count(&mut stage, output, &mut decoded_count);
                }
                stage.push($x, $y, $p, $t);
            };
        }

        // Main decoding loop - sequential, clean, readable
        while i < n {
            // Check budgets
            if decoded_count + stage.len() >= max_events || cur_t >= end_time {
                break;
            }

            let w = words[i];
            i += 1;

            let event_type = (w >> 12) as u8;

            match event_type {
                // EVT_ADDR_Y (0x0): Y coordinate update
                0x0 => {
                    cur_y = w & 0x07FF;
                    // Common pattern: Y followed immediately by X (emit CD event)
                    if i < n && (words[i] >> 12) == 0x2 {
                        let next = words[i];
                        i += 1;
                        let x = next & 0x07FF;
                        let p = ((next >> 11) & 1) as u8;
                        emit!(x, cur_y, p, cur_t);
                    }
                }

                // EVT_ADDR_X (0x2): CD event with current y, pol, time
                0x2 => {
                    let x = w & 0x07FF;
                    let p = ((w >> 11) & 1) as u8;
                    emit!(x, cur_y, p, cur_t);
                }

                // VECT_BASE_X (0x3): Update base X and polarity
                VECT_BASE_X => {
                    cur_pol = ((w >> 11) & 1) as u8;
                    cur_x = w & 0x07FF;
                }

                // VECT_12 (0x4): Emit 12 events from base_x
                VECT_12 => {
                    let valid_mask = w & 0x0FFF;
                    for bit in 0..12 {
                        if valid_mask & (1 << bit) != 0 {
                            emit!(cur_x + bit, cur_y, cur_pol, cur_t);
                        }
                    }
                    cur_x += 12;
                }

                // VECT_8 (0x5): Emit 8 events from base_x
                VECT_8 => {
                    let valid_mask = w & 0x00FF;
                    for bit in 0..8 {
                        if valid_mask & (1 << bit) != 0 {
                            emit!(cur_x + bit, cur_y, cur_pol, cur_t);
                        }
                    }
                    cur_x += 8;
                }

                // EVT_TIME_LOW (0x6): Update time low bits
                EVT_TIME_LOW => {
                    let time_low = (w & 0x0FFF) as u64;
                    cur_t = time_base | time_low;
                    self.time = cur_t;
                }

                // EVT_TIME_HIGH (0x8): Update time high bits
                EVT_TIME_HIGH => {
                    self.handle_time_high(w);
                    time_base = self.time_base;
                    cur_t = self.time;
                }

                // EXT_TRIGGER (0xA): External trigger event
                EXT_TRIGGER => {
                    let id = ((w >> 8) & 0x0F) as u8;
                    let pol = (w & 1) as u8;
                    output.push_trigger(id, pol, cur_t);
                }

                // CONTINUED (0xB): Multi-event continuation
                0xB => {
                    let num_12bit_events = (w & 0x0F) as usize;
                    for _ in 0..num_12bit_events {
                        if i >= n {
                            break;
                        }
                        let next = words[i];
                        i += 1;
                        let event_subtype = (next >> 12) as u8;
                        match event_subtype {
                            0x0 => cur_y = next & 0x07FF,
                            0x2 => {
                                let x = next & 0x07FF;
                                let p = ((next >> 11) & 1) as u8;
                                emit!(x, cur_y, p, cur_t);
                            }
                            VECT_BASE_X => {
                                cur_pol = ((next >> 11) & 1) as u8;
                                cur_x = next & 0x07FF;
                            }
                            VECT_12 => {
                                let valid_mask = next & 0x0FFF;
                                for bit in 0..12 {
                                    if valid_mask & (1 << bit) != 0 {
                                        emit!(cur_x + bit, cur_y, cur_pol, cur_t);
                                    }
                                }
                                cur_x += 12;
                            }
                            VECT_8 => {
                                let valid_mask = next & 0x00FF;
                                for bit in 0..8 {
                                    if valid_mask & (1 << bit) != 0 {
                                        emit!(cur_x + bit, cur_y, cur_pol, cur_t);
                                    }
                                }
                                cur_x += 8;
                            }
                            _ => {} // Ignore unknown subtypes
                        }
                    }
                }

                _ => {} // Ignore unknown event types
            }
        }

        // Final flush
        flush_and_count(&mut stage, output, &mut decoded_count);

        // Save state
        self.y = cur_y;
        self.time = cur_t;
        self.x = cur_x;
        self.polarity = cur_pol;
        self.time_base = time_base;

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
        let mut new_t = (event_time << 12) + self.time_high_loop_nb as u64 * TIME_LOOP_DURATION_US;

        // Handle timestamp wrapping
        if prev > new_t && prev - new_t >= MAX_TIMESTAMP_BASE - LOOP_THRESHOLD {
            new_t += TIME_LOOP_DURATION_US;
            self.time_high_loop_nb += 1;
        }

        self.time_base = new_t;
        self.time = new_t;
    }
}
