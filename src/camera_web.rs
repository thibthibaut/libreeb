//! WebUSB-based EVK4 camera reader for WASM targets.
//!
//! Implements the same Treuzell protocol as `camera.rs`, but uses the browser's
//! WebUSB API (`web-sys`) instead of `rusb`.  All USB operations are async
//! (they return JS Promises under the hood).
//!
//! The entry point is [`WasmCameraReader::open`], which shows the browser's
//! USB device-picker dialog, runs the full ISSD init sequence, and then
//! provides [`WasmCameraReader::read_batch`] for streaming EVT3 event batches.

use js_sys::{Array, Object, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{UsbDevice, UsbDeviceRequestOptions};

/// Log a formatted string to the browser JS console.
macro_rules! clog {
    ($($t:tt)*) => {
        web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(&format!($($t)*)))
    };
}

use crate::{biases::{BiasGroup, BiasInfo, BIAS_TABLE}, evt3::{Evt3, Evt3Decoder}, EventBatch, EventDecoder};

// ── EVK4 USB constants ────────────────────────────────────────────────────────

const EVK4_VID: u16 = 0x04b4;
const EVK4_PID: u16 = 0x00f5;
const BULK_BUF_SIZE: u32 = 128 * 1024;

// Treuzell protocol constants
const TZ_WRITE_FLAG: u32         = 0x4000_0000;
const TZ_PROP_DEVICES: u32       = 0x0001_0000;
const TZ_PROP_DEVICE_REG32: u32  = 0x0001_0102;
const TZ_PROP_DEVICE_STREAM: u32 = 0x0001_0200;

// ── Sensor spec table ─────────────────────────────────────────────────────────

pub struct SensorSpec {
    pub name:   &'static str,
    pub width:  u32,
    pub height: u32,
}

static SENSOR_TABLE: &[(u32, SensorSpec)] = &[
    (0xA040_1806, SensorSpec { name: "IMX646", width: 1280, height: 720 }),
];

const SENSOR_ID_ADDR: u32 = 0x0000_0014;

// ── RegOp sequences (identical to camera.rs) ─────────────────────────────────

#[derive(Clone, Copy)]
enum RegOp {
    Write(u32, u32),
    WriteField(u32, u32, u32),
    Delay(u32),
}
use RegOp::{Delay, Write, WriteField};

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
    Delay(1_000_000),
    Write(0x0040_0004, 0x0000_0001),
    Delay(500_000),
    Write(0x0040_0004, 0x0000_0000),
    Delay(1_000_000),
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

static SENSOR_TEMP_INIT: &[RegOp] = &[
    WriteField(0x0000_004C, 0x0000_0001, 0x0000_0001),
    WriteField(0x0000_004C, 0x0000_0002, 0x0000_0002),
    WriteField(0x0000_0054, 0x0000_0002, 0x0000_0002),
    Delay(100),
    WriteField(0x0000_005C, 0x0000_0002, 0x0000_0002),
    WriteField(0x0000_005C, 0x0000_0001, 0x0000_0001),
    Delay(100),
    WriteField(0x0000_004C, 0x0000_0000, 0x0000_0002),
];

static SENSOR_IPH_MIRROR: &[RegOp] = &[
    WriteField(0x0000_0074, 0x0000_0001, 0x0000_0001),
    Delay(20),
    WriteField(0x0000_0074, 0x0000_0002, 0x0000_0002),
    Delay(20),
];

static SENSOR_LIFO: &[RegOp] = &[
    WriteField(0x0000_000C, 0x0000_0001, 0x0000_0001),
    Delay(1_000),
    WriteField(0x0000_000C, 0x0000_0002, 0x0000_0002),
    Delay(1_000),
    WriteField(0x0000_000C, 0x0000_0004, 0x0000_0004),
];

static SENSOR_EVT3_FORMAT: &[RegOp] = &[
    WriteField(0x0000_7000, 0x0000_0000, 0x0000_0002),
    WriteField(0x0000_8000, 0x0000_0080, 0x0000_00C0),
];

// ── Endpoint addresses (WebUSB uses plain endpoint number, no direction bit) ──
//
// USB descriptor addresses:  ctrl_out=0x02, ctrl_in=0x82, data_in=0x81
// WebUSB endpoint number = address & 0x0F (direction is implied by the method)

const EP_CTRL: u8 = 2;   // transferIn / transferOut endpoint 2 (Treuzell control)
const EP_DATA: u8 = 1;   // transferIn endpoint 1 (EVT3 bulk stream)
const IFACE:   u8 = 0;

// ── Treuzell async helpers ────────────────────────────────────────────────────

/// Encode a Treuzell frame: `[property, size_bytes, payload...]` as LE u32s → u8 bytes.
/// The `size` field is the payload length in **bytes** (= words × 4), matching the
/// native `build_tz_frame` in `camera.rs`.
fn encode_tz(prop: u32, payload: &[u32]) -> Vec<u8> {
    let size_bytes = (payload.len() as u32) * 4;
    let mut out = Vec::with_capacity(8 + payload.len() * 4);
    out.extend_from_slice(&prop.to_le_bytes());
    out.extend_from_slice(&size_bytes.to_le_bytes());
    for &w in payload {
        out.extend_from_slice(&w.to_le_bytes());
    }
    out
}

/// Decode a Treuzell response: strip 2-word header, return payload u32s.
fn decode_tz(bytes: &[u8]) -> Vec<u32> {
    if bytes.len() < 8 { return vec![]; }
    bytes[8..]
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

async fn tz_transfer(
    device: &UsbDevice,
    prop: u32,
    payload: &[u32],
) -> Result<Vec<u32>, JsValue> {
    let frame = encode_tz(prop, payload);
    let arr = Uint8Array::from(frame.as_slice());

    // Write request — transfer_out returns Result<Promise, JsValue>
    JsFuture::from(device.transfer_out_with_u8_array(EP_CTRL, &arr)?).await?;

    // Read response
    let result = JsFuture::from(
        device.transfer_in(EP_CTRL, 1024)
    ).await?;
    let transfer: web_sys::UsbInTransferResult = result.dyn_into()?;
    let data = transfer.data()
        .ok_or_else(|| JsValue::from_str("empty USB response"))?;
    let bytes: Vec<u8> = (0..data.byte_length())
        .map(|i| data.get_uint8(i))
        .collect();

    // Check for Treuzell failure flag
    if bytes.len() >= 4 {
        let resp_prop = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        if resp_prop & 0x8000_0000 != 0 {
            return Err(JsValue::from_str(&format!("Treuzell error: prop={:#010x}", resp_prop)));
        }
    }

    Ok(decode_tz(&bytes))
}

async fn tz_read_reg(device: &UsbDevice, dev_id: u32, addr: u32) -> Result<u32, JsValue> {
    // Request payload: [dev_id, addr, nval=1]  (nval is required by Treuzell)
    // Response payload (after decode_tz strips header): [dev_id, addr, val]
    let payload = tz_transfer(
        device,
        TZ_PROP_DEVICE_REG32,
        &[dev_id, addr, 1],
    ).await?;
    payload.get(2).copied().ok_or_else(|| JsValue::from_str("reg read response too short"))
}

async fn tz_write_reg(device: &UsbDevice, dev_id: u32, addr: u32, val: u32) -> Result<(), JsValue> {
    tz_transfer(
        device,
        TZ_PROP_DEVICE_REG32 | TZ_WRITE_FLAG,
        &[dev_id, addr, val],
    ).await?;
    Ok(())
}

async fn tz_write_reg_field(
    device: &UsbDevice,
    dev_id: u32,
    addr: u32,
    data: u32,
    mask: u32,
) -> Result<(), JsValue> {
    let old = tz_read_reg(device, dev_id, addr).await?;
    let new_val = (old & !mask) | (data & mask);
    tz_write_reg(device, dev_id, addr, new_val).await
}

async fn run_seq(device: &UsbDevice, dev_id: u32, seq: &[RegOp]) -> Result<(), JsValue> {
    for &op in seq {
        match op {
            Write(addr, val) => tz_write_reg(device, dev_id, addr, val).await?,
            WriteField(addr, data, mask) =>
                tz_write_reg_field(device, dev_id, addr, data, mask).await?,
            Delay(usec) => {
                // In WASM we use a JS setTimeout-based sleep
                let ms = (usec / 1000).max(1) as f64;
                let promise = js_sys::Promise::new(&mut |resolve, _reject| {
                    web_sys::window()
                        .unwrap()
                        .set_timeout_with_callback_and_timeout_and_arguments_0(
                            &resolve,
                            ms as i32,
                        )
                        .unwrap();
                });
                JsFuture::from(promise).await?;
            }
        }
    }
    Ok(())
}

async fn find_sensor(device: &UsbDevice) -> Result<(u32, &'static SensorSpec), JsValue> {
    clog!("[libreeb] find_sensor: querying TZ_PROP_DEVICES");
    let resp = tz_transfer(device, TZ_PROP_DEVICES, &[]).await
        .map_err(|e| {
            clog!("[libreeb] find_sensor: TZ_PROP_DEVICES failed: {:?}", e);
            e
        })?;

    if resp.is_empty() {
        clog!("[libreeb] find_sensor: TZ_PROP_DEVICES response empty");
        return Err(JsValue::from_str("TZ_PROP_DEVICES response empty"));
    }

    let count = resp[0];
    clog!("[libreeb] find_sensor: device count = {}", count);

    let mut last_err: Option<String> = None;

    for dev_id in 0..count {
        clog!("[libreeb] find_sensor: reading sensor ID register from device {}", dev_id);
        match tz_read_reg(device, dev_id, SENSOR_ID_ADDR).await {
            Err(e) => {
                let msg = format!("{:?}", e);
                clog!("[libreeb] find_sensor: tz_read_reg dev={} FAILED: {}", dev_id, msg);
                last_err = Some(msg);
            }
            Ok(id_val) => {
                clog!("[libreeb] find_sensor: dev={} sensor_id = {:#010x}", dev_id, id_val);
                if let Some((_, spec)) = SENSOR_TABLE.iter().find(|(id, _)| *id == id_val) {
                    clog!("[libreeb] find_sensor: matched sensor \"{}\" ({}x{})",
                          spec.name, spec.width, spec.height);
                    return Ok((dev_id, spec));
                }
                let known: Vec<String> = SENSOR_TABLE.iter()
                    .map(|(id, s)| format!("{:#010x} ({})", id, s.name))
                    .collect();
                let msg = format!(
                    "Sensor ID {:#010x} not in table. Known: [{}]",
                    id_val,
                    known.join(", ")
                );
                clog!("[libreeb] find_sensor: {}", msg);
                last_err = Some(msg);
            }
        }
    }

    let detail = last_err.unwrap_or_else(|| "no devices enumerated".to_string());
    Err(JsValue::from_str(&format!("No known sensor found: {}", detail)))
}

// ── Public camera struct ──────────────────────────────────────────────────────

pub struct WasmCameraReader {
    device:        UsbDevice,
    dev_id:        u32,
    pub spec:      &'static SensorSpec,
    pub bias_defaults: Vec<u8>,
    decoder:       Evt3Decoder,
}

impl WasmCameraReader {
    /// Show the browser's USB device picker, then open + initialise the EVK4.
    /// Must be called from a user-gesture context (button click, etc.).
    pub async fn open() -> Result<Self, JsValue> {
        let window = web_sys::window().ok_or("no window")?;
        let usb = window.navigator().usb();

        // Build filter: VID + PID
        let filter = Object::new();
        Reflect::set(&filter, &"vendorId".into(),  &(EVK4_VID as u32).into())?;
        Reflect::set(&filter, &"productId".into(), &(EVK4_PID as u32).into())?;
        let filters = Array::new();
        filters.push(&filter);
        let opts = UsbDeviceRequestOptions::new(&filters);

        let device_val = JsFuture::from(usb.request_device(&opts)).await?;
        let device: UsbDevice = device_val.dyn_into()?;

        JsFuture::from(device.open()).await?;
        JsFuture::from(device.claim_interface(IFACE)).await?;

        let (dev_id, spec) = find_sensor(&device).await?;

        // ISSD stop + destroy (pre-init reset, failures expected)
        let _ = run_seq(&device, dev_id, ISSD_STOP).await;
        let _ = run_seq(&device, dev_id, ISSD_DESTROY).await;

        // Full init sequence
        run_seq(&device, dev_id, ISSD_INIT).await?;

        // Sensor config
        run_seq(&device, dev_id, SENSOR_TEMP_INIT).await?;
        run_seq(&device, dev_id, SENSOR_IPH_MIRROR).await?;

        let _ = gloo_timers_sleep(1).await;

        run_seq(&device, dev_id, SENSOR_LIFO).await?;
        run_seq(&device, dev_id, SENSOR_EVT3_FORMAT).await?;

        // ISSD start
        run_seq(&device, dev_id, ISSD_START).await?;

        // stream-on (non-fatal)
        let _ = tz_transfer(&device, TZ_PROP_DEVICE_STREAM | TZ_WRITE_FLAG, &[dev_id, 1]).await;

        // Read bias factory defaults
        let bias_defaults: Vec<u8> = {
            let mut defaults = Vec::new();
            for bi in BIAS_TABLE {
                let v = tz_read_reg(&device, dev_id, bi.addr).await
                    .map(|r| (r & 0xFF) as u8)
                    .unwrap_or(128);
                defaults.push(v);
            }
            defaults
        };

        Ok(WasmCameraReader {
            device,
            dev_id,
            spec,
            bias_defaults,
            decoder: Evt3Decoder::new(),
        })
    }

    pub fn width(&self)  -> u32 { self.spec.width }
    pub fn height(&self) -> u32 { self.spec.height }
    pub fn sensor_name(&self) -> &str { self.spec.name }
    pub fn bias_default(&self, index: usize) -> u8 {
        self.bias_defaults.get(index).copied().unwrap_or(128)
    }

    pub async fn read_batch(&mut self) -> Option<EventBatch> {
        let result = JsFuture::from(
            self.device.transfer_in(EP_DATA, BULK_BUF_SIZE)
        ).await.ok()?;
        let transfer: web_sys::UsbInTransferResult = result.dyn_into().ok()?;
        let data = transfer.data()?;
        let len = data.byte_length();
        let bytes: Vec<u8> = (0..len).map(|i| data.get_uint8(i)).collect();
        if bytes.is_empty() { return None; }

        // Reinterpret raw bytes as Evt3 (u16) words via bytemuck (zero-copy).
        let word_size  = std::mem::size_of::<Evt3>();
        let word_count = bytes.len() / word_size;
        if word_count == 0 { return None; }
        let evts: &[Evt3] = bytemuck::cast_slice(&bytes[..word_count * word_size]);

        let mut batch = EventBatch::new();
        self.decoder.decode(evts, &mut batch);
        if batch.is_empty() { None } else { Some(batch) }
    }

    pub async fn write_bias(&self, addr: u32, factory: u8, offset: i32) -> Result<(), JsValue> {
        let new_idac = (factory as i32 + offset).clamp(0, 255) as u32;
        let encoded = 0x11A1_0000u32 | new_idac;
        tz_write_reg(&self.device, self.dev_id, addr, encoded).await
    }

    pub async fn stop(&self) {
        let _ = run_seq(&self.device, self.dev_id, ISSD_STOP).await;
        let _ = run_seq(&self.device, self.dev_id, ISSD_DESTROY).await;
        let _ = JsFuture::from(self.device.release_interface(IFACE)).await;
        let _ = JsFuture::from(self.device.close()).await;
    }
}

// Small async sleep helper using JS setTimeout
async fn gloo_timers_sleep(ms: i32) -> Result<(), JsValue> {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
            .unwrap();
    });
    JsFuture::from(promise).await?;
    Ok(())
}
