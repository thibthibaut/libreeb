//! Bias metadata for the IMX646 (EVK4 HD) sensor.
//!
//! All six bias registers use the **offset convention** introduced with IMX636:
//! values are signed offsets from the factory-programmed `idac_ctl` default.
//! The directions below apply to Gen4 / IMX636 / IMX646:
//!
//! | Bias | ↑ effect |
//! |---|---|
//! | `bias_fo`  | wider low-pass BW (more high-freq events) |
//! | `bias_hpf` | stronger high-pass (less slow background) |
//! | `bias_diff_on/off` | higher threshold (less sensitive) |
//! | `bias_refr` | shorter pixel dead-time (more burst events) |

/// UI grouping for the bias controls panel.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BiasGroup {
    /// Low-pass / high-pass pixel filters (`bias_fo`, `bias_hpf`).
    Bandwidth,
    /// Contrast sensitivity thresholds (`bias_diff_on`, `bias_diff_off`, `bias_diff`).
    Contrast,
    /// Refractory period and other advanced controls.
    Advanced,
}

/// Per-bias metadata for one IMX646 register.
pub struct BiasInfo {
    pub name: &'static str,
    pub addr: u32,
    /// Recommended minimum offset (inclusive).
    pub rec_min: i32,
    /// Recommended maximum offset (inclusive).
    pub rec_max: i32,
    /// Less-noisy starting offset applied automatically on camera open.
    pub default_offset: i32,
    pub group: BiasGroup,
    /// One-line tooltip shown in the UI.
    pub hint: &'static str,
}

/// All six IMX646 bias registers in register-map order.
pub static BIAS_TABLE: &[BiasInfo] = &[
    BiasInfo {
        name: "bias_fo",
        addr: 0x0000_1004,
        rec_min: -20,
        rec_max: 0,
        default_offset: -10,
        group: BiasGroup::Bandwidth,
        hint: "Low-pass cut-off. Decrease → narrower BW → less flicker & high-freq noise.",
    },
    BiasInfo {
        name: "bias_hpf",
        addr: 0x0000_100C,
        rec_min: 0,
        rec_max: 120,
        default_offset: 10,
        group: BiasGroup::Bandwidth,
        hint: "High-pass filter strength. Increase → filter more slow/static background.",
    },
    BiasInfo {
        name: "bias_diff_on",
        addr: 0x0000_1010,
        rec_min: -80,
        rec_max: 145,
        default_offset: 20,
        group: BiasGroup::Contrast,
        hint: "ON event threshold. Increase → brighter change required before an ON event fires.",
    },
    BiasInfo {
        name: "bias_diff",
        addr: 0x0000_1014,
        rec_min: -25,
        rec_max: 23,
        default_offset: 0,
        group: BiasGroup::Contrast,
        hint: "Reference contrast level. Prophesee recommends keeping this at 0.",
    },
    BiasInfo {
        name: "bias_diff_off",
        addr: 0x0000_1018,
        rec_min: -30,
        rec_max: 200,
        default_offset: 20,
        group: BiasGroup::Contrast,
        hint: "OFF event threshold. Increase → darker change required before an OFF event fires.",
    },
    BiasInfo {
        name: "bias_refr",
        addr: 0x0000_1020,
        rec_min: -20,
        rec_max: 235,
        default_offset: -10,
        group: BiasGroup::Advanced,
        hint: "Refractory (dead) period. Decrease → longer pixel dead-time → fewer burst events.",
    },
];
