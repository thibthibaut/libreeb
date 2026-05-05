use crate::{declare_raw_evt, EventBatch, EventDecoder};

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
                return;
            }
        }

        batch.x.reserve(n);
        batch.y.reserve(n);
        batch.p.reserve(n);
        batch.t.reserve(n);

        macro_rules! emit {
            ($x:expr, $y:expr, $t:expr, $p:expr) => {
                batch.x.push($x);
                batch.y.push($y);
                batch.p.push($p);
                batch.t.push($t);
            };
        }

        let mut cur_y = self.y;
        let mut cur_t = self.time;
        let mut cur_x = self.x;
        let mut cur_pol = self.polarity;
        let mut time_base = self.time_base;

        while i < n {
            let w = words[i];
            i += 1;

            if w >= 0x2000 && w < 0x3000 {
                emit!(w & 0x07FF, cur_y, cur_t, (w >> 11) as u8 & 1);
                continue;
            }

            if w < 0x1000 {
                cur_y = w & 0x07FF;
                if i < n {
                    let next = words[i];
                    if next >= 0x2000 && next < 0x3000 {
                        emit!(next & 0x07FF, cur_y, cur_t, (next >> 11) as u8 & 1);
                        i += 1;
                    }
                }
                continue;
            }

            match (w >> 12) as u8 {
                VECT_BASE_X => {
                    cur_pol = (w >> 11) as u8 & 1;
                    cur_x = w & 0x07FF;
                }
                VECT_12 => {
                    let mut mask = (w & 0x0FFF) as u32;
                    while mask != 0 {
                        let bit = mask.trailing_zeros() as u16;
                        mask &= mask - 1;
                        emit!(cur_x + bit, cur_y, cur_t, cur_pol);
                    }
                    cur_x += 12;
                }
                VECT_8 => {
                    let mut mask = (w & 0x00FF) as u32;
                    while mask != 0 {
                        let bit = mask.trailing_zeros() as u16;
                        mask &= mask - 1;
                        emit!(cur_x + bit, cur_y, cur_t, cur_pol);
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
                }
                _ => {}
            }
        }

        self.y = cur_y;
        self.time = cur_t;
        self.x = cur_x;
        self.polarity = cur_pol;
        self.time_base = time_base;
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

        if prev > new_t && prev - new_t >= MAX_TIMESTAMP_BASE - LOOP_THRESHOLD {
            new_t += TIME_LOOP_DURATION_US;
            self.time_high_loop_nb += 1;
        }

        self.time_base = new_t;
        self.time = new_t;
    }
}
