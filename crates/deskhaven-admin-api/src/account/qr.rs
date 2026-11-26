//! QR codes as SVG path data: the console's CSP allows no `data:` image and no inline style.

use std::fmt::Write as _;

use qrcodegen::{QrCode as Encoder, QrCodeEcc};
use serde::Serialize;
use utoipa::ToSchema;

const QUIET_ZONE: i32 = 4;

/// A QR code to draw: a square of `size` units, dark modules in `path`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct QrCode {
    /// Side length in modules, quiet zone included (the SVG `viewBox` is `0 0 size size`).
    pub size: i32,
    /// SVG path data of the dark modules.
    pub path: String,
}

pub fn encode(text: &str) -> Option<QrCode> {
    let code = Encoder::encode_text(text, QrCodeEcc::Medium).ok()?;
    let side = code.size();
    let mut path = String::new();
    for y in 0..side {
        let mut x = 0;
        while x < side {
            if !code.get_module(x, y) {
                x += 1;
                continue;
            }
            let start = x;
            while x < side && code.get_module(x, y) {
                x += 1;
            }
            let run = x - start;
            // Writing to a String cannot fail.
            let _ = write!(
                path,
                "M{} {}h{run}v1h-{run}z",
                start + QUIET_ZONE,
                y + QUIET_ZONE
            );
        }
    }
    Some(QrCode {
        size: side + 2 * QUIET_ZONE,
        path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_runs_inside_the_quiet_zone() {
        let qr = encode("otpauth://totp/deskhaven:alice?secret=JBSWY3DPEHPK3PXP").unwrap();

        // Version 3+ for this length: at least 29 modules plus the quiet zone.
        assert!(qr.size >= 29 + 2 * QUIET_ZONE, "{}", qr.size);
        assert!(
            qr.path.starts_with("M4 4h7v1h-7z"),
            "finder pattern first: {}",
            qr.path
        );
        assert!(!qr.path.contains('<') && !qr.path.contains('"'));
    }

    #[test]
    fn deterministic_for_the_same_text() {
        assert_eq!(encode("abc"), encode("abc"));
        assert_ne!(encode("abc"), encode("abd"));
    }
}
