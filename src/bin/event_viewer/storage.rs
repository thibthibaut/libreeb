//! localStorage persistence helpers for per-sensor settings.
//!
//! All keys are scoped to a sensor hardware ID so settings from different
//! sensors never collide.  Values are stored as compact text strings.
//!
//! Key format:
//!   `libreeb_biases_{sensor_id:#010x}`  → comma-separated i32 offsets
//!   `libreeb_roi_{sensor_id:#010x}`     → "left,top,width,height,enabled"

// ── Internal helper ───────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

// ── Biases ────────────────────────────────────────────────────────────────────

/// Persist bias offsets for `sensor_id`.
/// If the write fails (localStorage unavailable / quota exceeded) it is
/// silently ignored — the biases are still applied to the hardware.
#[cfg(target_arch = "wasm32")]
pub fn save_biases(sensor_id: u32, offsets: &[i32]) {
    let Some(storage) = local_storage() else { return };
    let key = format!("libreeb_biases_{:#010x}", sensor_id);
    let val = offsets.iter()
        .map(|o| o.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let _ = storage.set_item(&key, &val);
}

/// Load previously saved bias offsets for `sensor_id`.
/// Returns `None` if no entry exists or if the length doesn't match
/// `expected_len` (guards against stale data from a different bias table).
#[cfg(target_arch = "wasm32")]
pub fn load_biases(sensor_id: u32, expected_len: usize) -> Option<Vec<i32>> {
    let storage = local_storage()?;
    let key = format!("libreeb_biases_{:#010x}", sensor_id);
    let val = storage.get_item(&key).ok().flatten()?;
    let offsets: Vec<i32> = val.split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    if offsets.len() == expected_len { Some(offsets) } else { None }
}

// ── ROI ───────────────────────────────────────────────────────────────────────

/// Persist the current ROI window for `sensor_id`.
/// `enabled` controls whether the ROI should be re-armed on next connect.
#[cfg(target_arch = "wasm32")]
pub fn save_roi(sensor_id: u32, left: u32, top: u32, width: u32, height: u32, enabled: bool) {
    let Some(storage) = local_storage() else { return };
    let key = format!("libreeb_roi_{:#010x}", sensor_id);
    let val = format!("{},{},{},{},{}", left, top, width, height, if enabled { 1 } else { 0 });
    let _ = storage.set_item(&key, &val);
}

/// Load previously saved ROI for `sensor_id`.
/// Returns `(left, top, width, height, enabled)` or `None`.
#[cfg(target_arch = "wasm32")]
pub fn load_roi(sensor_id: u32) -> Option<(u32, u32, u32, u32, bool)> {
    let storage = local_storage()?;
    let key = format!("libreeb_roi_{:#010x}", sensor_id);
    let val = storage.get_item(&key).ok().flatten()?;
    let parts: Vec<&str> = val.split(',').collect();
    if parts.len() != 5 { return None; }
    let left    = parts[0].parse().ok()?;
    let top     = parts[1].parse().ok()?;
    let width   = parts[2].parse().ok()?;
    let height  = parts[3].parse().ok()?;
    let enabled = parts[4] == "1";
    Some((left, top, width, height, enabled))
}

// ── No-op stubs for native (cargo check) ─────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
pub fn save_biases(_: u32, _: &[i32]) {}

#[cfg(not(target_arch = "wasm32"))]
pub fn load_biases(_: u32, _: usize) -> Option<Vec<i32>> { None }

#[cfg(not(target_arch = "wasm32"))]
pub fn save_roi(_: u32, _: u32, _: u32, _: u32, _: u32, _: bool) {}

#[cfg(not(target_arch = "wasm32"))]
pub fn load_roi(_: u32) -> Option<(u32, u32, u32, u32, bool)> { None }
