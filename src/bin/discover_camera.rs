//! Diagnostic tool — dumps all USB info about every EVK4 found on the bus,
//! walks the Treuzell protocol, and prints the raw sensor-ID register value.
//!
//! Run as root, or with a udev rule granting access to VID=04b4 PID=00f5.
//!
//!   cargo run --bin discover_camera

use anyhow::{bail, Context, Result};
use rusb::{DeviceHandle, Direction, GlobalContext, TransferType};
use std::time::Duration;

const EVK4_VID: u16 = 0x04b4;
const EVK4_PID: u16 = 0x00f5;

const TZ_PROP_DEVICES: u32     = 0x0001_0000;
const TZ_PROP_DEVICE_REG32: u32 = 0x0001_0102;

const SENSOR_ID_ADDR: u32 = 0x0000_0014;

const CTRL_TIMEOUT:    Duration = Duration::from_millis(1_000);
const CTRL_RX_TIMEOUT: Duration = Duration::from_millis(2_000);

// ── Known sensor IDs (copy from camera.rs) ────────────────────────────────────

const KNOWN_SENSORS: &[(u32, &str, u32, u32)] = &[
    (0xA040_1806, "IMX646 (EVK4 HD)", 1280, 720),
    // add more here if needed
];

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() {
    println!("=== libreeb discover_camera ===\n");

    let devices = match rusb::devices() {
        Ok(d) => d,
        Err(e) => { eprintln!("Failed to enumerate USB devices: {e}"); return; }
    };

    let mut evk4_count = 0usize;
    for device in devices.iter() {
        let desc = match device.device_descriptor() {
            Ok(d) => d,
            Err(_) => continue,
        };

        // Print every device briefly so we can spot the camera even with wrong IDs.
        println!(
            "USB {:03}:{:03}  VID={:#06x}  PID={:#06x}",
            device.bus_number(), device.address(),
            desc.vendor_id(), desc.product_id(),
        );

        if desc.vendor_id() != EVK4_VID || desc.product_id() != EVK4_PID {
            continue;
        }

        println!("  *** EVK4 candidate found! ***");
        evk4_count += 1;

        // Print string descriptors if available.
        if let Ok(h) = device.open() {
            if let Ok(s) = h.read_manufacturer_string_ascii(&desc) {
                println!("  Manufacturer : {s}");
            }
            if let Ok(s) = h.read_product_string_ascii(&desc) {
                println!("  Product      : {s}");
            }
            if let Ok(s) = h.read_serial_number_string_ascii(&desc) {
                println!("  Serial       : {s}");
            }
        }

        // Walk configurations and interfaces.
        for cfg_idx in 0..desc.num_configurations() {
            let config = match device.config_descriptor(cfg_idx) {
                Ok(c) => c,
                Err(e) => { println!("  [config {cfg_idx}] error: {e}"); continue; }
            };
            println!("  Config {cfg_idx}: {} interface(s)", config.num_interfaces());
            for iface in config.interfaces() {
                for alt in iface.descriptors() {
                    println!(
                        "    Interface {:02}  alt {:02}  class={:#04x}  sub={:#04x}  proto={:#04x}  eps={}",
                        alt.interface_number(), alt.setting_number(),
                        alt.class_code(), alt.sub_class_code(), alt.protocol_code(),
                        alt.num_endpoints(),
                    );
                    for ep in alt.endpoint_descriptors() {
                        println!(
                            "      EP {:#04x}  {:?}  {:?}  max_pkt={}",
                            ep.address(), ep.transfer_type(), ep.direction(), ep.max_packet_size(),
                        );
                    }
                }
            }
        }

        // Try to open and talk Treuzell.
        println!();
        if let Err(e) = probe_treuzell(&device) {
            println!("  Treuzell probe FAILED: {e:#}");
        }
        println!();
    }

    if evk4_count == 0 {
        println!("\nNo EVK4 device found (VID={EVK4_VID:#06x} PID={EVK4_PID:#06x}).");
        println!("Check USB connection and permissions (udev rules / run as root).");
    } else {
        println!("Found {evk4_count} EVK4 device(s).");
    }
}

// ── Treuzell probe ────────────────────────────────────────────────────────────

fn probe_treuzell(device: &rusb::Device<GlobalContext>) -> Result<()> {
    let config = device.config_descriptor(0)
        .context("read config descriptor")?;

    let ep = find_treuzell_endpoints(&config)
        .context("no Treuzell interface found (expected class=0xFF sub=0x19 proto=0x00 with 3 bulk EPs)")?;

    println!(
        "  Treuzell interface {}: ctrl_in={:#04x}  ctrl_out={:#04x}  data_in={:#04x}",
        ep.iface, ep.ctrl_in, ep.ctrl_out, ep.data_in,
    );

    let handle = device.open().context("open device (permissions?)")?;

    match handle.kernel_driver_active(ep.iface) {
        Ok(true) => {
            handle.detach_kernel_driver(ep.iface)
                .context("detach kernel driver")?;
            println!("  Detached kernel driver from interface {}", ep.iface);
        }
        Ok(false) => println!("  No kernel driver on interface {}", ep.iface),
        Err(e)    => println!("  kernel_driver_active: {e} (non-fatal on macOS)"),
    }

    handle.claim_interface(ep.iface)
        .context("claim interface (camera busy?)")?;
    println!("  Interface {} claimed.", ep.iface);

    // ── TZ_PROP_DEVICES ───────────────────────────────────────────────────────
    let resp = tz_transfer(&handle, ep.ctrl_out, ep.ctrl_in, TZ_PROP_DEVICES, &[])
        .context("TZ_PROP_DEVICES")?;
    if resp.len() < 4 {
        bail!("TZ_PROP_DEVICES response too short ({} bytes)", resp.len());
    }
    let count = u32::from_le_bytes(resp[0..4].try_into().unwrap());
    println!("  Treuzell device count: {count}");

    // ── Per-device sensor ID ──────────────────────────────────────────────────
    for dev_id in 0..count {
        print!("  Device {dev_id}: reading sensor ID register {SENSOR_ID_ADDR:#010x} … ");
        match tz_read_reg(&handle, ep.ctrl_out, ep.ctrl_in, dev_id, SENSOR_ID_ADDR) {
            Err(e) => println!("FAILED: {e}"),
            Ok(id) => {
                print!("raw={id:#010x}");
                match KNOWN_SENSORS.iter().find(|(k, _, _, _)| *k == id) {
                    Some((_, name, w, h)) => println!("  → {name} ({w}×{h}) ✓"),
                    None => {
                        println!("  → UNKNOWN (not in SENSOR_TABLE)");
                        println!("    Add this entry to camera.rs SENSOR_TABLE:");
                        println!("    ({id:#010x}, SensorSpec {{ name: \"???\", width: ???, height: ??? }}),");
                    }
                }
            }
        }

        // Also dump a few other informational registers for context.
        for (label, addr) in [
            ("system_id    ", 0x0000_0000u32),
            ("subsystem_id ", 0x0000_0004u32),
            ("build_date   ", 0x0000_0008u32),
            ("version      ", 0x0000_000Cu32),
        ] {
            match tz_read_reg(&handle, ep.ctrl_out, ep.ctrl_in, dev_id, addr) {
                Ok(v)  => println!("    reg {label} @ {addr:#010x} = {v:#010x}"),
                Err(e) => println!("    reg {label} @ {addr:#010x} = ERROR: {e}"),
            }
        }
    }

    Ok(())
}

// ── Treuzell helpers (minimal copies from camera.rs) ─────────────────────────

struct Endpoints { ctrl_in: u8, ctrl_out: u8, data_in: u8, iface: u8 }

fn find_treuzell_endpoints(config: &rusb::ConfigDescriptor) -> Option<Endpoints> {
    for iface in config.interfaces() {
        for desc in iface.descriptors() {
            if desc.class_code()     != 0xFF
                || desc.sub_class_code() != 0x19
                || desc.protocol_code()  != 0x00
                || desc.num_endpoints()  != 3
            { continue; }
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

fn build_tz_frame(property: u32, payload: &[u32]) -> Vec<u8> {
    let size = (payload.len() * 4) as u32;
    let mut buf = Vec::with_capacity(8 + size as usize);
    buf.extend_from_slice(&property.to_le_bytes());
    buf.extend_from_slice(&size.to_le_bytes());
    for &w in payload { buf.extend_from_slice(&w.to_le_bytes()); }
    buf
}

fn tz_transfer(
    handle:   &DeviceHandle<GlobalContext>,
    ep_out:   u8,
    ep_in:    u8,
    property: u32,
    payload:  &[u32],
) -> Result<Vec<u8>> {
    let frame = build_tz_frame(property, payload);
    handle.write_bulk(ep_out, &frame, CTRL_TIMEOUT)
        .with_context(|| format!("Treuzell TX prop={property:#010x}"))?;

    let mut buf = vec![0u8; 1024];
    let n = handle.read_bulk(ep_in, &mut buf, CTRL_RX_TIMEOUT)
        .with_context(|| format!("Treuzell RX prop={property:#010x}"))?;
    buf.truncate(n);

    if buf.len() < 8 {
        bail!("response too short ({} bytes) for prop={property:#010x}", buf.len());
    }
    let resp_prop = u32::from_le_bytes(buf[0..4].try_into().unwrap());
    let resp_size = u32::from_le_bytes(buf[4..8].try_into().unwrap()) as usize;

    if resp_prop & 0x8000_0000 != 0 {
        bail!("Treuzell command failed: resp_prop={resp_prop:#010x}");
    }
    if resp_prop != property {
        bail!("property mismatch: got {resp_prop:#010x} expected {property:#010x}");
    }
    if buf.len() < 8 + resp_size {
        bail!("payload truncated ({} < {})", buf.len(), 8 + resp_size);
    }
    Ok(buf[8..8 + resp_size].to_vec())
}

fn tz_read_reg(
    handle: &DeviceHandle<GlobalContext>,
    ep_out: u8, ep_in: u8,
    dev_id: u32, addr: u32,
) -> Result<u32> {
    let resp = tz_transfer(handle, ep_out, ep_in,
        TZ_PROP_DEVICE_REG32, &[dev_id, addr, 1])?;
    if resp.len() < 12 {
        bail!("read_reg payload too short ({} bytes)", resp.len());
    }
    Ok(u32::from_le_bytes(resp[8..12].try_into().unwrap()))
}
