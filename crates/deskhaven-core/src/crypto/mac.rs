//! For low-entropy secrets looked up by equality: a database dump alone does not allow an offline
//! brute force. HKDF (RFC 5869) is computed directly; every HMAC key fits one block, so the
//! infallible `KeyInit::new` with a zero-padded key is what HMAC does anyway.

use hmac::digest::Key;
use hmac::{Hmac, KeyInit, Mac};
use sha2_011::Sha256;
use zeroize::{Zeroize, Zeroizing};

use super::MASTER_KEY_LEN;

type HmacSha256 = Hmac<Sha256>;

pub(crate) const MAC_KEY_LEN: usize = 32;
const HKDF_INFO: &[u8] = b"deskhaven/hmac/v1";

pub(crate) fn derive_mac_key(master: &[u8; MASTER_KEY_LEN]) -> Zeroizing<[u8; MAC_KEY_LEN]> {
    let prk = hmac_sha256(&[0; MAC_KEY_LEN], &[master]);
    hmac_sha256(&prk, &[HKDF_INFO, &[1]])
}

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
