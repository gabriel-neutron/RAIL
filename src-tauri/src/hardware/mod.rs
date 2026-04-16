//! RTL-SDR hardware access.
//!
//! See `docs/HARDWARE.md` for librtlsdr sequencing and tuning constraints.
//!
//! Phase 0 scope: USB enumeration only, via the pure-Rust `nusb` crate.
//! RTL-SDR dongles (RTL2832U) expose USB VID `0x0bda` with PID in a known
//! set. Full librtlsdr integration (tuning, streaming) lands in Phase 1.

use serde::Serialize;

use crate::error::RailError;

pub mod stream;

/// Serializable RTL-SDR device description sent to the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfo {
    pub index: u32,
    pub name: String,
}

/// Known RTL2832U-based dongle USB identifiers (vendor_id, product_id).
/// Reference: rtl-sdr.com hardware list. NESDR Smart v5 = `0x0bda:0x2838`.
const KNOWN_RTLSDR_IDS: &[(u16, u16)] = &[
    (0x0bda, 0x2832), // Generic RTL2832U
    (0x0bda, 0x2838), // Generic RTL2832U / NESDR Smart
    (0x0bda, 0x2831), // Realtek RTL2831U
    (0x0bda, 0x2834), // Realtek variants
    (0x0ccd, 0x00a9), // Terratec Cinergy T Stick Black
    (0x0ccd, 0x00b3), // Terratec NOXON DAB/DAB+
    (0x185b, 0x0620), // Compro Videomate U620F
    (0x1f4d, 0xa803), // SVEON STV20
    (0x1554, 0x5020), // PixelView PV-DT235U
];

fn is_rtlsdr(vid: u16, pid: u16) -> bool {
    KNOWN_RTLSDR_IDS.iter().any(|&(v, p)| v == vid && p == pid)
}

fn friendly_name(vid: u16, pid: u16, product: Option<&str>) -> String {
    if let Some(p) = product.filter(|s| !s.is_empty()) {
        return p.to_string();
    }
    match (vid, pid) {
        (0x0bda, 0x2838) => "Generic RTL2832U / NESDR".into(),
        (0x0bda, 0x2832) => "Generic RTL2832U".into(),
        (vid, pid) => format!("RTL-SDR compatible ({vid:04x}:{pid:04x})"),
    }
}

/// Enumerate attached RTL-SDR compatible USB devices.
///
/// Returns the first matching device, or `RailError::DeviceNotFound` if none
/// are attached. Does **not** open the device — Phase 1 will handle streaming.
pub fn check_device() -> Result<DeviceInfo, RailError> {
    let devices = nusb::list_devices()
        .map_err(|e| RailError::DeviceOpenFailed(format!("USB enumeration failed: {e}")))?;

    for (idx, dev) in devices.enumerate() {
        let vid = dev.vendor_id();
        let pid = dev.product_id();
        if is_rtlsdr(vid, pid) {
            let name = friendly_name(vid, pid, dev.product_string());
            log::info!("RTL-SDR found at USB index {idx}: {name} ({vid:04x}:{pid:04x})");
            return Ok(DeviceInfo {
                index: idx as u32,
                name,
            });
        }
    }

    Err(RailError::DeviceNotFound)
}
