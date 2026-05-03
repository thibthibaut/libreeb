//! Live USB camera reader for Prophesee EVK4 (IMX646 sensor).
//!
//! Implements the Treuzell USB control protocol and the ISSD sensor
//! initialisation sequence derived from OpenEB's `imx636_evk3_issd.h`.
//! On construction the full stop → destroy → init cycle is run, then the
//! sensor is started and EVT3 data flows on the third bulk endpoint.
//! The struct implements `Iterator<Item = EventBatch>`.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use rusb::{DeviceHandle, GlobalContext};
use zerocopy::FromBytes;

use crate::{evt3::Evt3Decoder, EventBatch, EventDecoder};

// ── Treuzell protocol constants ───────────────────────────────────────────────

const TZ_WRITE_FLAG: u32         = 0x4000_0000;
const TZ_PROP_DEVICES: u32       = 0x0001_0000;
const TZ_PROP_DEVICE_REG32: u32  = 0x0001_0102;
const TZ_PROP_DEVICE_STREAM: u32 = 0x0001_0200;

// ── EVK4 USB identifiers ──────────────────────────────────────────────────────

const EVK4_VID: u16 = 0x04b4;
const EVK4_PID: u16 = 0x00f5;

// ── USB transfer parameters ───────────────────────────────────────────────────

const CTRL_TIMEOUT:    Duration = Duration::from_millis(1_000);
const CTRL_RX_TIMEOUT: Duration = Duration::from_millis(10_000);
const DATA_TIMEOUT:    Duration = Duration::from_millis(100);
const BULK_BUF_SIZE:   usize   = 128 * 1024;

// ── Sensor specification table ────────────────────────────────────────────────

/// Static geometry and name for a known sensor model.
pub struct SensorSpec {
    pub name:   &'static str,
    pub width:  u32,
    pub height: u32,
}

/// Maps the value of sensor identification register 0x0014 to a `SensorSpec`.
/// Extend this table when supporting additional sensors (IMX636, GenX320, …).
static SENSOR_TABLE: &[(u32, SensorSpec)] = &[
    // IMX646 — EVK4, 1280×720 HD
    (0xA040_1806, SensorSpec { name: "IMX646", width: 1280, height: 720 }),
];

const SENSOR_ID_ADDR: u32 = 0x0000_0014;

// ── Bias register table ───────────────────────────────────────────────────────

/// Static descriptor for one sensor bias register.
///
/// Values are IMX646-specific.  `rec_min`/`rec_max` are the recommended
/// offset range from the factory-programmed default (same units and direction
/// as OpenEB's `Imx646_LL_Biases`).
pub struct BiasInfo {
    pub name:    &'static str,
    pub addr:    u32,
    pub rec_min: i32,
    pub rec_max: i32,
}

/// All six IMX646 bias registers in the order they appear in the register map.
pub static BIAS_TABLE: &[BiasInfo] = &[
    BiasInfo { name: "bias_fo",       addr: 0x0000_1004, rec_min: -20, rec_max:   0 },
    BiasInfo { name: "bias_hpf",      addr: 0x0000_100C, rec_min:   0, rec_max: 120 },
    BiasInfo { name: "bias_diff_on",  addr: 0x0000_1010, rec_min: -80, rec_max: 145 },
    BiasInfo { name: "bias_diff",     addr: 0x0000_1014, rec_min: -25, rec_max:  23 },
    BiasInfo { name: "bias_diff_off", addr: 0x0000_1018, rec_min: -30, rec_max: 200 },
    BiasInfo { name: "bias_refr",     addr: 0x0000_1020, rec_min: -20, rec_max: 235 },
];

// ── Camera device listing ─────────────────────────────────────────────────────

/// Identifies one EVK4 found on the USB bus (by enumeration index).
#[derive(Clone)]
pub struct CameraDeviceInfo {
    /// 0-based index among all EVK4 devices found.
    pub index:   usize,
    pub bus:     u8,
    pub address: u8,
}

/// Return all EVK4 devices currently visible on the USB bus.
///
/// Does not open or claim any device; safe to call from the UI thread.
pub fn list_cameras() -> Vec<CameraDeviceInfo> {
    let mut result = Vec::new();
    if let Ok(devices) = rusb::devices() {
        let mut index = 0usize;
        for device in devices.iter() {
            if let Ok(desc) = device.device_descriptor() {
                if desc.vendor_id() == EVK4_VID && desc.product_id() == EVK4_PID {
                    result.push(CameraDeviceInfo {
                        index,
                        bus:     device.bus_number(),
                        address: device.address(),
                    });
                    index += 1;
                }
            }
        }
    }
    result
}

// ── Register operation definition ────────────────────────────────────────────

/// A single step in an ISSD or sensor-init sequence.
#[derive(Clone, Copy)]
enum RegOp {
    /// Write `val` verbatim to `addr`.
    Write(u32, u32),
    /// Read-modify-write: `new = (old & !mask) | (data & mask)`.
    WriteField(u32, u32, u32),
    /// Sleep for `usec` microseconds.
    Delay(u32),
}

use RegOp::{Delay, Write, WriteField};

// ── ISSD sequences (verbatim from imx636_evk3_issd.h) ────────────────────────

static ISSD_STOP: &[RegOp] = &[
    WriteField(0x0000_0004, 0xF000_5042, 0x0000_0400),
    Write(0x0000_002C, 0x0022_C324),
    Write(0x0000_9028, 0x0000_0002),
    Delay(1_000),
    WriteField(0x0000_9008, 0x0000_0644, 0x0000_0001),
    Write(0x0000_B000, 0x0000_02F8),
    Delay(300),
];

static ISSD_DESTROY: &[RegOp] = &[
    Write(0x0000_0070, 0x0040_0008),
    Write(0x0000_006C, 0x0EE4_7114),
    Delay(500),
    Write(0x0000_A00C, 0x0002_0400),
    Delay(500),
    Write(0x0000_A010, 0x0000_8068),
    Delay(200),
    Write(0x0000_1104, 0x0000_0000),
    Delay(200),
    Write(0x0000_A020, 0x0000_0050),
    Delay(200),
    Write(0x0000_A004, 0x000B_0500),
    Delay(200),
    Write(0x0000_A008, 0x0000_2404),
    Delay(200),
    Write(0x0000_A000, 0x000B_0500),
    Write(0x0000_B044, 0x0000_0000),
    Write(0x0000_B004, 0x0000_000A),
    Write(0x0000_B040, 0x0000_000E),
    Write(0x0000_B0C8, 0x0000_0000),
    Write(0x0000_B040, 0x0000_0006),
    Write(0x0000_B040, 0x0000_0004),
    Write(0x0000_0000, 0x4F00_6442),
    Write(0x0000_0000, 0x0F00_6442),
    Write(0x0000_00B8, 0x0000_0401),
    Write(0x0000_00B8, 0x0000_0400),
    Write(0x0000_B07C, 0x0000_0000),
];

static ISSD_INIT: &[RegOp] = &[
    Write(0x0000_001C, 0x0000_0001),
    Delay(1_000_000), // 1 s
    Write(0x0040_0004, 0x0000_0001),
    Delay(500_000),   // 500 ms
    Write(0x0040_0004, 0x0000_0000),
    Delay(1_000_000), // 1 s
    Write(0x0000_B000, 0x0000_0158),
    Delay(300),
    Write(0x0000_B044, 0x0000_0000),
    Write(0x0000_B004, 0x0000_000A),
    Write(0x0000_B040, 0x0000_0000),
    Write(0x0000_B0C8, 0x0000_0000),
    Write(0x0000_B040, 0x0000_0000),
    Write(0x0000_B040, 0x0000_0000),
    Write(0x0000_0000, 0x4F00_6442),
    Write(0x0000_0000, 0x0F00_6442),
    Write(0x0000_00B8, 0x0000_0400),
    Write(0x0000_00B8, 0x0000_0400),
    Write(0x0000_B07C, 0x0000_0000),
    Write(0x0000_B074, 0x0000_0002),
    Write(0x0000_B078, 0x0000_00A0),
    Write(0x0000_00C0, 0x0000_0110),
    Write(0x0000_00C0, 0x0000_0210),
    Write(0x0000_B120, 0x0000_0001),
    Write(0x0000_E120, 0x0000_0000),
    Write(0x0000_B068, 0x0000_0004),
    Write(0x0000_B07C, 0x0000_0001),
    Delay(10),
    Write(0x0000_B07C, 0x0000_0003),
    Delay(1_000),
    Write(0x0000_00B8, 0x0000_0401),
    Write(0x0000_00B8, 0x0000_0409),
    Write(0x0000_0000, 0x4F00_6442),
    Write(0x0000_0000, 0x4F00_644A),
    Write(0x0000_B080, 0x0000_0077),
    Write(0x0000_B084, 0x0000_000F),
    Write(0x0000_B088, 0x0000_0037),
    Write(0x0000_B08C, 0x0000_0037),
    Write(0x0000_B090, 0x0000_00DF),
    Write(0x0000_B094, 0x0000_0057),
    Write(0x0000_B098, 0x0000_0037),
    Write(0x0000_B09C, 0x0000_0067),
    Write(0x0000_B0A0, 0x0000_0037),
    Write(0x0000_B0A4, 0x0000_002F),
    Write(0x0000_B0AC, 0x0000_0028),
    Write(0x0000_B0CC, 0x0000_0001),
    Write(0x0000_B000, 0x0000_02F8),
    Write(0x0000_B004, 0x0000_008A),
    Write(0x0000_B01C, 0x0000_0030),
    Write(0x0000_B020, 0x0000_2000),
    Write(0x0000_B02C, 0x0000_00FF),
    Write(0x0000_B030, 0x0000_3E80),
    Write(0x0000_B028, 0x0000_0FA0),
    Write(0x0000_A000, 0x000B_0501),
    Delay(200),
    Write(0x0000_A008, 0x0000_2405),
    Delay(200),
    Write(0x0000_A004, 0x000B_0501),
    Delay(200),
    Write(0x0000_A020, 0x0000_0150),
    Delay(200),
    Write(0x0000_B040, 0x0000_0007),
    Write(0x0000_B064, 0x0000_0006),
    Write(0x0000_B040, 0x0000_000F),
    Delay(100),
    Write(0x0000_B004, 0x0000_008A),
    Delay(200),
    Write(0x0000_B0C8, 0x0000_0003),
    Delay(200),
    Write(0x0000_B044, 0x0000_0001),
    Write(0x0000_B000, 0x0000_02F9),
    Write(0x0000_7008, 0x0000_0001),
    Write(0x0000_7000, 0x0007_0001),
    Write(0x0000_8000, 0x0001_E085),
    Write(0x0000_9008, 0x0000_0644),
    Write(0x0000_0004, 0xF000_5042),
    Write(0x0000_0018, 0x0000_0200),
    Write(0x0000_1014, 0x11A1_504D),
    Write(0x0000_9004, 0x0000_0000),
    Delay(1_000),
    Write(0x0000_9000, 0x0000_0200),
];

static ISSD_START: &[RegOp] = &[
    Write(0x0000_B000, 0x0000_02F9),
    Write(0x0000_9028, 0x0000_0000),
    WriteField(0x0000_9008, 0x0000_0645, 0x0000_0001),
    Write(0x0000_002C, 0x0022_C724),
    WriteField(0x0000_0004, 0xF000_5442, 0x0000_0400),
];

// ── Sensor-specific init sequences ───────────────────────────────────────────

/// `TzImx646::temperature_init()` — ADC + temperature sensor bring-up.
static SENSOR_TEMP_INIT: &[RegOp] = &[
    WriteField(0x0000_004C, 0x0000_0001, 0x0000_0001), // adc_en       = 1 (bit 0)
    WriteField(0x0000_004C, 0x0000_0002, 0x0000_0002), // adc_clk_en   = 1 (bit 1)
    WriteField(0x0000_0054, 0x0000_0002, 0x0000_0002), // adc_buf_cal_en = 1 (bit 1)
    Delay(100),
    WriteField(0x0000_005C, 0x0000_0002, 0x0000_0002), // temp_buf_en     = 1 (bit 1)
    WriteField(0x0000_005C, 0x0000_0001, 0x0000_0001), // temp_buf_cal_en = 1 (bit 0)
    Delay(100),
    WriteField(0x0000_004C, 0x0000_0000, 0x0000_0002), // adc_clk_en = 0 (clear bit 1)
];

/// `TzImx646::iph_mirror_control(true)` — current mirror enable.
static SENSOR_IPH_MIRROR: &[RegOp] = &[
    WriteField(0x0000_0074, 0x0000_0001, 0x0000_0001), // iph_mirr_en     = 1 (bit 0)
    Delay(20),
    WriteField(0x0000_0074, 0x0000_0002, 0x0000_0002), // iph_mirr_amp_en = 1 (bit 1)
    Delay(20),
];

/// `TzImx646::lifo_control(enable=true, out_en=true, cnt_en=true)`.
static SENSOR_LIFO: &[RegOp] = &[
    WriteField(0x0000_000C, 0x0000_0001, 0x0000_0001), // lifo_en    = 1 (bit 0)
    Delay(1_000),
    WriteField(0x0000_000C, 0x0000_0002, 0x0000_0002), // lifo_out_en = 1 (bit 1)
    Delay(1_000),
    WriteField(0x0000_000C, 0x0000_0004, 0x0000_0004), // lifo_cnt_en = 1 (bit 2)
];

/// `TzImx646::set_output_format("EVT3")` — select EVT3 output.
static SENSOR_EVT3_FORMAT: &[RegOp] = &[
    // edf/pipeline_control (0x7000): format field = bit 1, set to 0 (EVT3)
    WriteField(0x0000_7000, 0x0000_0000, 0x0000_0002),
    // eoi/Reserved_8000 (0x8000): Reserved_7_6 field = bits[7:6], set to 2 (0b10 << 6 = 0x80)
    WriteField(0x0000_8000, 0x0000_0080, 0x0000_00C0),
];

// ── Endpoint layout ───────────────────────────────────────────────────────────

struct Endpoints {
    ctrl_in:  u8,
    ctrl_out: u8,
    data_in:  u8,
    iface:    u8,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Live camera reader that streams EVT3 events from a Prophesee EVK4.
///
/// Implements `Iterator<Item = EventBatch>`. Each call to `next()` blocks
/// until a non-empty batch of decoded CD events arrives over USB.
///
/// Drop (or call `stop()`) to gracefully shut down: the camera is returned
/// to its idle state and the USB interface is released.
pub struct CameraReader {
    handle:        DeviceHandle<GlobalContext>,
    ep:            Endpoints,
    dev_id:        u32,
    spec:          &'static SensorSpec,
    bias_defaults: Vec<u8>,   // factory idac_ctl for each BIAS_TABLE entry
    decoder:       Evt3Decoder,
    buf:           Vec<u8>,
    stopped:       bool,
}

impl CameraReader {
    /// Open the first EVK4 found on USB.  Shorthand for `open_nth(0)`.
    pub fn open() -> Result<Self> {
        Self::open_nth(0)
    }

    /// Open the n-th EVK4 found on USB (0-based).
    ///
    /// Runs the full ISSD init + sensor config sequence (~3–4 s).
    pub fn open_nth(n: usize) -> Result<Self> {
        let (handle, ep) = find_and_open_nth(n)?;
        let (dev_id, spec) = find_sensor(&handle, &ep)?;
        eprintln!("camera: {} at Treuzell dev_id={}", spec.name, dev_id);
        let mut cam = CameraReader {
            handle, ep, dev_id, spec,
            bias_defaults: Vec::new(),
            decoder: Evt3Decoder::new(),
            buf: vec![0u8; BULK_BUF_SIZE],
            stopped: false,
        };
        cam.init()?;
        cam.load_bias_defaults();
        Ok(cam)
    }

    /// Width of the sensor in pixels.
    pub fn width(&self)  -> u32 { self.spec.width }

    /// Height of the sensor in pixels.
    pub fn height(&self) -> u32 { self.spec.height }

    /// Human-readable sensor model name (e.g. `"IMX646"`).
    pub fn sensor_name(&self) -> &str { self.spec.name }

    /// Factory-default `idac_ctl` byte for the bias at `BIAS_TABLE[index]`.
    ///
    /// Returns 128 (mid-scale) if the index is out of range or the read failed.
    pub fn bias_default(&self, index: usize) -> u8 {
        self.bias_defaults.get(index).copied().unwrap_or(128)
    }

    /// Read the raw `idac_ctl` byte (bits [7:0]) from a bias register.
    pub fn read_bias_raw(&self, addr: u32) -> Result<u8> {
        let val = tz_read_reg(&self.handle, self.ep.ctrl_out, self.ep.ctrl_in,
                              self.dev_id, addr)?;
        Ok((val & 0xFF) as u8)
    }

    /// Apply a bias offset relative to the factory default.
    ///
    /// `new_idac = (factory as i32 + offset).clamp(0, 255)`
    ///
    /// The register is written with the OpenEB encoding:
    /// `0x11A1_0000 | new_idac`.
    pub fn write_bias(&self, addr: u32, factory: u8, offset: i32) -> Result<()> {
        let new_idac = (factory as i32 + offset).clamp(0, 255) as u32;
        let encoded = 0x11A1_0000u32 | new_idac;
        tz_write_reg(&self.handle, self.ep.ctrl_out, self.ep.ctrl_in,
                     self.dev_id, addr, encoded)
    }

    /// Gracefully stop streaming: stream-off → ISSD stop → ISSD destroy →
    /// release interface. Safe to call multiple times.
    pub fn stop(&mut self) {
        if self.stopped { return; }
        self.stopped = true;
        if let Err(e) = stream_ctrl(&self.handle, &self.ep, self.dev_id, false) {
            eprintln!("camera: stream-off failed (non-fatal): {}", e);
        }
        if let Err(e) = run_seq(&self.handle, &self.ep, self.dev_id, ISSD_STOP) {
            eprintln!("camera: ISSD stop failed (non-fatal): {}", e);
        }
        if let Err(e) = run_seq(&self.handle, &self.ep, self.dev_id, ISSD_DESTROY) {
            eprintln!("camera: ISSD destroy failed (non-fatal): {}", e);
        }
        if let Err(e) = self.handle.release_interface(self.ep.iface) {
            eprintln!("camera: release_interface failed (non-fatal): {}", e);
        }
    }

    // ── Private ──────────────────────────────────────────────────────────────

    /// Read and cache the factory-programmed idac_ctl for every BIAS_TABLE entry.
    /// Called once after `init()`.  Failures are non-fatal (defaults to 128).
    fn load_bias_defaults(&mut self) {
        self.bias_defaults = BIAS_TABLE.iter().map(|bi| {
            self.read_bias_raw(bi.addr).unwrap_or_else(|e| {
                eprintln!("camera: could not read factory default for {} (non-fatal): {}", bi.name, e);
                128
            })
        }).collect();
        eprintln!("camera: bias factory defaults: {:?}", self.bias_defaults);
    }

    fn init(&mut self) -> Result<()> {
        // Safe stop/reset of any previous state (failures are expected on a
        // freshly-plugged device and are intentionally ignored).
        eprintln!("camera: ISSD stop (pre-init reset)...");
        let _ = run_seq(&self.handle, &self.ep, self.dev_id, ISSD_STOP);
        eprintln!("camera: ISSD destroy...");
        let _ = run_seq(&self.handle, &self.ep, self.dev_id, ISSD_DESTROY);

        eprintln!("camera: ISSD init (~3 s)...");
        run_seq(&self.handle, &self.ep, self.dev_id, ISSD_INIT)?;

        eprintln!("camera: sensor config...");
        run_seq(&self.handle, &self.ep, self.dev_id, SENSOR_TEMP_INIT)?;
        run_seq(&self.handle, &self.ep, self.dev_id, SENSOR_IPH_MIRROR)?;
        std::thread::sleep(Duration::from_millis(1));
        run_seq(&self.handle, &self.ep, self.dev_id, SENSOR_LIFO)?;
        run_seq(&self.handle, &self.ep, self.dev_id, SENSOR_EVT3_FORMAT)?;

        eprintln!("camera: ISSD start...");
        run_seq(&self.handle, &self.ep, self.dev_id, ISSD_START)?;

        // Clear any stale USB state on the data endpoint before reading.
        if let Err(e) = self.handle.clear_halt(self.ep.data_in) {
            eprintln!("camera: clear_halt failed (non-fatal): {}", e);
        }

        eprintln!("camera: stream on...");
        // The Treuzell stream command targets a separate "streamer" device in
        // two-device firmware. On single-device firmware (IMX646 only) the
        // device rejects it — ISSD start is sufficient in that case.
        if let Err(e) = stream_ctrl(&self.handle, &self.ep, self.dev_id, true) {
            eprintln!("camera: stream-on not supported on sensor device (ISSD start is sufficient): {}", e);
        }
        eprintln!("camera: streaming.");
        Ok(())
    }
}

impl Iterator for CameraReader {
    type Item = EventBatch;

    fn next(&mut self) -> Option<EventBatch> {
        use crate::evt3::Evt3;
        if self.stopped { return None; }
        loop {
            let n = match self.handle.read_bulk(self.ep.data_in, &mut self.buf, DATA_TIMEOUT) {
                Ok(0)                       => continue,
                Ok(n)                       => n,
                Err(rusb::Error::Timeout)   => continue,
                Err(e) => {
                    eprintln!("camera: bulk read error: {}", e);
                    return None;
                }
            };

            // Trim to a multiple of the EVT3 word size (2 bytes).
            let word_size = std::mem::size_of::<Evt3>();
            let size = n - (n % word_size);
            if size == 0 { continue; }

            let evts = match <[Evt3]>::ref_from_bytes_with_elems(&self.buf[..size], size / word_size) {
                Ok(s) => s,
                Err(_) => continue,
            };

            let mut batch = EventBatch::with_capacity(evts.len() / 4);
            self.decoder.decode(evts, &mut batch);

            if !batch.is_empty() {
                return Some(batch);
            }
        }
    }
}

impl Drop for CameraReader {
    fn drop(&mut self) {
        self.stop();
    }
}

// ── Treuzell protocol helpers ─────────────────────────────────────────────────

/// Build a Treuzell control frame: `[property LE][payload_bytes LE][u32s LE...]`.
fn build_tz_frame(property: u32, payload: &[u32]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(8 + payload.len() * 4);
    frame.extend_from_slice(&property.to_le_bytes());
    frame.extend_from_slice(&((payload.len() as u32) * 4).to_le_bytes());
    for &v in payload {
        frame.extend_from_slice(&v.to_le_bytes());
    }
    frame
}

/// Send a Treuzell frame on `ep_out`, read back response on `ep_in`.
/// Returns the response **payload** bytes (everything after the 8-byte header).
fn tz_transfer(
    handle: &DeviceHandle<GlobalContext>,
    ep_out: u8,
    ep_in:  u8,
    property: u32,
    payload:  &[u32],
) -> Result<Vec<u8>> {
    let frame = build_tz_frame(property, payload);
    handle.write_bulk(ep_out, &frame, CTRL_TIMEOUT)
        .with_context(|| format!("Treuzell TX prop={:#010x}", property))?;

    let mut buf = vec![0u8; 1024];
    let n = handle.read_bulk(ep_in, &mut buf, CTRL_RX_TIMEOUT)
        .with_context(|| format!("Treuzell RX prop={:#010x}", property))?;
    buf.truncate(n);

    if buf.len() < 8 {
        bail!("Treuzell response too short ({} bytes) for prop={:#010x}", buf.len(), property);
    }
    let resp_prop = u32::from_le_bytes(buf[0..4].try_into().unwrap());
    let resp_size = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;

    if resp_prop & 0x8000_0000 != 0 {
        bail!("Treuzell command failed: resp_prop={:#010x}", resp_prop);
    }
    if resp_prop != property {
        bail!("Treuzell property mismatch: got {:#010x} expected {:#010x}", resp_prop, property);
    }
    if buf.len() < 8 + resp_size {
        bail!("Treuzell payload truncated ({} < {})", buf.len(), 8 + resp_size);
    }
    Ok(buf[8..8 + resp_size].to_vec())
}

fn tz_write_reg(
    handle: &DeviceHandle<GlobalContext>,
    ep_out: u8, ep_in: u8,
    dev_id: u32, addr: u32, val: u32,
) -> Result<()> {
    tz_transfer(handle, ep_out, ep_in,
        TZ_PROP_DEVICE_REG32 | TZ_WRITE_FLAG,
        &[dev_id, addr, val])?;
    Ok(())
}

fn tz_read_reg(
    handle: &DeviceHandle<GlobalContext>,
    ep_out: u8, ep_in: u8,
    dev_id: u32, addr: u32,
) -> Result<u32> {
    // Send: [dev_id, addr, nval=1]; Receive payload: [dev_id, addr, val]
    let resp = tz_transfer(handle, ep_out, ep_in,
        TZ_PROP_DEVICE_REG32,
        &[dev_id, addr, 1])?;
    if resp.len() < 12 {
        bail!("read_reg response payload too short ({} bytes)", resp.len());
    }
    Ok(u32::from_le_bytes(resp[8..12].try_into().unwrap()))
}

fn tz_write_reg_field(
    handle: &DeviceHandle<GlobalContext>,
    ep_out: u8, ep_in: u8,
    dev_id: u32, addr: u32, data: u32, mask: u32,
) -> Result<()> {
    let current = tz_read_reg(handle, ep_out, ep_in, dev_id, addr)?;
    let new_val = (current & !mask) | (data & mask);
    tz_write_reg(handle, ep_out, ep_in, dev_id, addr, new_val)
}

fn stream_ctrl(
    handle: &DeviceHandle<GlobalContext>,
    ep: &Endpoints,
    dev_id: u32,
    on: bool,
) -> Result<()> {
    tz_transfer(handle, ep.ctrl_out, ep.ctrl_in,
        TZ_PROP_DEVICE_STREAM | TZ_WRITE_FLAG,
        &[dev_id, on as u32])?;
    Ok(())
}

fn run_seq(
    handle: &DeviceHandle<GlobalContext>,
    ep: &Endpoints,
    dev_id: u32,
    seq: &[RegOp],
) -> Result<()> {
    for &op in seq {
        match op {
            Write(addr, val) => {
                tz_write_reg(handle, ep.ctrl_out, ep.ctrl_in, dev_id, addr, val)?;
            }
            WriteField(addr, data, mask) => {
                tz_write_reg_field(handle, ep.ctrl_out, ep.ctrl_in, dev_id, addr, data, mask)?;
            }
            Delay(usec) => {
                std::thread::sleep(Duration::from_micros(usec as u64));
            }
        }
    }
    Ok(())
}

// ── Device discovery ──────────────────────────────────────────────────────────

fn find_and_open_nth(n: usize) -> Result<(DeviceHandle<GlobalContext>, Endpoints)> {
    let mut found = 0usize;
    for device in rusb::devices()?.iter() {
        let desc = match device.device_descriptor() {
            Ok(d) => d,
            Err(_) => continue,
        };
        if desc.vendor_id() != EVK4_VID || desc.product_id() != EVK4_PID {
            continue;
        }
        if found < n {
            found += 1;
            continue;
        }
        // This is the n-th EVK4.
        let handle = device.open()
            .context("Failed to open EVK4 USB device (check udev rules / permissions)")?;
        let config = device.config_descriptor(0)
            .context("Failed to read EVK4 USB config descriptor")?;
        let ep = match find_treuzell_endpoints(&config) {
            Some(ep) => ep,
            None => bail!("EVK4 found but no Treuzell interface (3 bulk EPs) detected"),
        };
        match handle.kernel_driver_active(ep.iface) {
            Ok(true) => {
                handle.detach_kernel_driver(ep.iface)
                    .context("Failed to detach kernel driver from Treuzell interface")?;
                eprintln!("camera: detached kernel driver from interface {}", ep.iface);
            }
            _ => {}
        }
        handle.claim_interface(ep.iface)
            .context("Failed to claim Treuzell interface (camera busy?)")?;
        if let Err(e) = handle.set_alternate_setting(ep.iface, 0) {
            eprintln!("camera: set_alternate_setting skipped (non-fatal): {}", e);
        }
        eprintln!("camera: claimed interface {} (ctrl_in={:#04x} ctrl_out={:#04x} data_in={:#04x})",
            ep.iface, ep.ctrl_in, ep.ctrl_out, ep.data_in);
        return Ok((handle, ep));
    }
    bail!("EVK4 #{} not found (VID={:#06x} PID={:#06x})", n, EVK4_VID, EVK4_PID)
}

fn find_sensor(
    handle: &DeviceHandle<GlobalContext>,
    ep:     &Endpoints,
) -> Result<(u32, &'static SensorSpec)> {
    let resp = tz_transfer(handle, ep.ctrl_out, ep.ctrl_in, TZ_PROP_DEVICES, &[])
        .context("TZ_PROP_DEVICES query failed")?;
    if resp.len() < 4 {
        bail!("TZ_PROP_DEVICES response too short");
    }
    let count = u32::from_le_bytes(resp[0..4].try_into().unwrap());
    eprintln!("camera: {} Treuzell device(s) on board", count);
    for dev_id in 0..count {
        if let Ok(id_val) = tz_read_reg(handle, ep.ctrl_out, ep.ctrl_in, dev_id, SENSOR_ID_ADDR) {
            if let Some((_, spec)) = SENSOR_TABLE.iter().find(|(id, _)| *id == id_val) {
                return Ok((dev_id, spec));
            }
        }
    }
    bail!("No known sensor found among {} Treuzell device(s)", count)
}

fn find_treuzell_endpoints(config: &rusb::ConfigDescriptor) -> Option<Endpoints> {
    use rusb::{Direction, TransferType};
    for iface in config.interfaces() {
        for desc in iface.descriptors() {
            // Treuzell interface: class=0xFF (vendor), subclass=0x19, protocol=0x00
            if desc.class_code()     != 0xFF
                || desc.sub_class_code() != 0x19
                || desc.protocol_code()  != 0x00
                || desc.num_endpoints()  != 3
            {
                continue;
            }
            let eps: Vec<_> = desc.endpoint_descriptors().collect();
            if eps.len() != 3 { continue; }
            if eps[0].transfer_type() == TransferType::Bulk && eps[0].direction() == Direction::In
                && eps[1].transfer_type() == TransferType::Bulk && eps[1].direction() == Direction::Out
                && eps[2].transfer_type() == TransferType::Bulk && eps[2].direction() == Direction::In
            {
                return Some(Endpoints {
                    ctrl_in:  eps[0].address(),
                    ctrl_out: eps[1].address(),
                    data_in:  eps[2].address(),
                    iface:    desc.interface_number(),
                });
            }
        }
    }
    None
}
