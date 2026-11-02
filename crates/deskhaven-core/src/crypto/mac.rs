//! HMAC-SHA256 under a key derived from the master key (06-security §3.3):
//! `k_hmac = HKDF-SHA256(IKM = master key, salt = none, info = "deskhaven/hmac/v1")`.
//!
//! For low-entropy secrets that are looked up by equality (recovery codes, emailed codes):
//! a database dump alone does not allow an offline brute force.
//!
//! HKDF is computed here directly (RFC 5869 §2.2–2.3): for a 32-byte output it is exactly
//! `PRK = HMAC(0^32, IKM)`, `OKM = HMAC(PRK, info || 0x01)`. Every HMAC key is at most one
//! block long, so it goes through the infallible `KeyInit::new` with the key zero-padded to
//! the block size, which is what HMAC does with a short key anyway. Pinned by an OpenSSL
//! known-answer test (`tests/secret_box.rs`).

use hmac::digest::Key;
use hmac::{Hmac, KeyInit, Mac};
use sha2_011::Sha256;
use zeroize::{Zeroize, Zeroizing};

use super::MASTER_KEY_LEN;

type HmacSha256 = Hmac<Sha256>;

pub(crate) const MAC_KEY_LEN: usize = 32;
const HKDF_INFO: &[u8] = b"deskhaven/hmac/v1";

/// `k_hmac` for `master`.
pub(crate) fn derive_mac_key(master: &[u8; MASTER_KEY_LEN]) -> Zeroizing<[u8; MAC_KEY_LEN]> {
    let prk = hmac_sha256(&[0; MAC_KEY_LEN], &[master]);
    hmac_sha256(&prk, &[HKDF_INFO, &[1]])
}

/// `HMAC-SHA256(key, parts concatenated)`.
pub(crate) fn hmac_sha256(key: &[u8; MAC_KEY_LEN], parts: &[&[u8]]) -> Zeroizing<[u8; 32]> {
    let mut padded = Key::<HmacSha256>::default();
    padded[..MAC_KEY_LEN].copy_from_slice(key);
    let mut mac = HmacSha256::new(&padded);
    padded.as_mut_slice().zeroize();
    for part in parts {
        mac.update(part);
    }
    let mut out = Zeroizing::new([0u8; 32]);
    out.copy_from_slice(&mac.finalize().into_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_sha256_matches_rfc_4231_test_case_1() {
        // RFC 4231 test case 1 uses a 20-byte key of 0x0b; padded to 32 bytes with zeros it
        // is the same HMAC key (keys shorter than a block are zero-padded).
        let mut key = [0u8; MAC_KEY_LEN];
        key[..20].copy_from_slice(&[0x0b; 20]);

        let tag = hmac_sha256(&key, &[b"Hi ", b"There"]);

        let hex: String = tag.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }
}
