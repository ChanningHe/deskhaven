//! TOTP (RFC 6238 over RFC 4226 HOTP) with the parameters every authenticator app assumes
//! when a provisioning URI does not say otherwise: HMAC-SHA1, 6 digits, 30-second steps
//! from the Unix epoch (06 §3.3).
//!
//! Pure functions; replay protection (one use per step and user) is the service's job, fed
//! by [`matching_step`]'s result.

use std::fmt;

use data_encoding::BASE32_NOPAD;
use hmac::digest::Key;
use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, Zeroizing};

use crate::crypto::{CryptoError, fill_random};

type HmacSha1 = Hmac<Sha1>;

/// Digits of a code.
pub const DIGITS: usize = 6;
/// Length of one time step.
pub const PERIOD_MS: i64 = 30_000;
/// Steps accepted on either side of the current one (clock drift, typing time).
pub const SKEW_STEPS: i64 = 1;
/// Bytes of a generated seed (RFC 4226 §4 recommends 160 bits).
pub const SECRET_LEN: usize = 20;
/// Longest seed accepted: one HMAC-SHA1 block, so the key needs no pre-hashing.
pub const MAX_SECRET_LEN: usize = 64;
const MODULUS: u32 = 1_000_000;

/// A TOTP seed. Wiped on drop; `Debug` never shows it.
pub struct TotpSecret(Zeroizing<Vec<u8>>);

impl TotpSecret {
    /// A fresh random seed of [`SECRET_LEN`] bytes.
    pub(crate) fn generate() -> Result<Self, CryptoError> {
        let mut bytes = Zeroizing::new(vec![0u8; SECRET_LEN]);
        fill_random(&mut bytes)?;
        Ok(Self(bytes))
    }

    /// `None` when empty or longer than [`MAX_SECRET_LEN`].
    pub(crate) fn from_bytes(bytes: Zeroizing<Vec<u8>>) -> Option<Self> {
        (!bytes.is_empty() && bytes.len() <= MAX_SECRET_LEN).then(|| Self(bytes))
    }

    /// Parses the base32 form an authenticator app is given (RFC 4648, unpadded, any case).
    pub fn from_base32(text: &str) -> Option<Self> {
        let upper = Zeroizing::new(text.to_ascii_uppercase());
        let bytes = Zeroizing::new(BASE32_NOPAD.decode(upper.as_bytes()).ok()?);
        Self::from_bytes(bytes)
    }

    /// RFC 4648 base32, unpadded, upper case: the `secret` of a provisioning URI.
    pub fn to_base32(&self) -> Zeroizing<String> {
        Zeroizing::new(BASE32_NOPAD.encode(&self.0))
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for TotpSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TotpSecret(<redacted>)")
    }
}

/// The time step containing `unix_ms`.
pub fn step_at(unix_ms: i64) -> i64 {
    unix_ms.div_euclid(PERIOD_MS)
}

/// The code of `step` as an authenticator shows it (zero-padded).
pub fn code_at_step(secret: &TotpSecret, step: i64) -> String {
    format!("{:0width$}", hotp(secret, step), width = DIGITS)
}

/// The code an authenticator shows at `unix_ms`.
pub fn code_at(secret: &TotpSecret, unix_ms: i64) -> String {
    code_at_step(secret, step_at(unix_ms))
}

/// The step `code` belongs to, within [`SKEW_STEPS`] of `now_ms` and strictly after
/// `last_step` (a step already used, or an older one, is never accepted again); the
/// earliest such step when several match. `None` for anything but exactly [`DIGITS`] ASCII
/// digits.
pub(crate) fn matching_step(
    secret: &TotpSecret,
    code: &str,
    now_ms: i64,
    last_step: Option<i64>,
) -> Option<i64> {
    let wanted = parse_code(code)?;
    let now = step_at(now_ms);
    // Every step of the window is computed and compared in constant time; only then is
    // the earliest match picked.
    let matches: Vec<(i64, bool)> = (now - SKEW_STEPS..=now + SKEW_STEPS)
        .map(|step| (step, bool::from(hotp(secret, step).ct_eq(&wanted))))
        .collect();
    matches
        .into_iter()
        .filter(|(step, _)| last_step.is_none_or(|last| *step > last))
        .find_map(|(step, matched)| matched.then_some(step))
}

/// Whether `code` has the shape of a TOTP code (as opposed to a recovery code).
pub fn is_code_shaped(code: &str) -> bool {
    parse_code(code).is_some()
}

fn parse_code(code: &str) -> Option<u32> {
    if code.len() != DIGITS || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    code.parse().ok()
}

/// RFC 4226 §5.3: HMAC-SHA1 over the big-endian counter, dynamic truncation, mod 10^6.
fn hotp(secret: &TotpSecret, counter: i64) -> u32 {
    // `from_bytes` keeps seeds within one block: zero-padding to the block size is what
    // HMAC does with a short key, and the infallible `new` needs the full block.
    let mut key = Key::<HmacSha1>::default();
    key[..secret.bytes().len()].copy_from_slice(secret.bytes());
    let mut mac = HmacSha1::new(&key);
    key.as_mut_slice().zeroize();
    mac.update(&counter.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = usize::from(digest[digest.len() - 1] & 0x0f);
    let word = [
        digest[offset],
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ];
    (u32::from_be_bytes(word) & 0x7fff_ffff) % MODULUS
}

/// `otpauth://totp/<issuer>:<account>?secret=…&issuer=…&algorithm=SHA1&digits=6&period=30`
/// (the Key Uri Format authenticator apps read from a QR code). Contains the seed: show it
/// to its owner once, never log it.
pub fn provisioning_uri(secret: &TotpSecret, issuer: &str, account: &str) -> Zeroizing<String> {
    let (issuer, account) = (percent_encode(issuer), percent_encode(account));
    let secret = secret.to_base32();
    Zeroizing::new(format!(
        "otpauth://totp/{issuer}:{account}?secret={}&issuer={issuer}&algorithm=SHA1&digits={DIGITS}&period={}",
        secret.as_str(),
        PERIOD_MS / 1000
    ))
}

/// RFC 3986 percent-encoding of everything but the unreserved characters.
fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 6238 appendix B, SHA-1 seed.
    fn rfc_secret() -> TotpSecret {
        TotpSecret::from_bytes(Zeroizing::new(b"12345678901234567890".to_vec())).unwrap()
    }

    const SECOND: i64 = 1000;

    #[test]
    fn rfc_6238_sha1_vectors_truncated_to_six_digits() {
        // Appendix B lists 8 digits; 6 digits are the same value mod 10^6.
        for (unix_s, code) in [
            (59, "287082"),
            (1_111_111_109, "081804"),
            (1_111_111_111, "050471"),
            (1_234_567_890, "005924"),
            (2_000_000_000, "279037"),
            (20_000_000_000, "353130"),
        ] {
            assert_eq!(code_at(&rfc_secret(), unix_s * SECOND), code, "T={unix_s}");
        }
    }

    #[test]
    fn rfc_4226_hotp_vectors() {
        // Appendix D: counter 0..=9.
        let expected = [
            755_224, 287_082, 359_152, 969_429, 338_314, 254_676, 287_922, 162_583, 399_871,
            520_489,
        ];
        for (counter, code) in expected.into_iter().enumerate() {
            assert_eq!(
                hotp(&rfc_secret(), counter as i64),
                code,
                "counter {counter}"
            );
        }
    }

    #[test]
    fn accepts_one_step_either_side_and_nothing_further() {
        let secret = rfc_secret();
        let now = 1_234_567_890 * SECOND;
        let step = step_at(now);

        for delta in -1..=1 {
            let code = code_at_step(&secret, step + delta);
            assert_eq!(
                matching_step(&secret, &code, now, None),
                Some(step + delta),
                "{delta}"
            );
        }
        for delta in [-3, -2, 2, 3] {
            let code = code_at_step(&secret, step + delta);
            assert_eq!(matching_step(&secret, &code, now, None), None, "{delta}");
        }
    }

    #[test]
    fn a_used_or_older_step_is_never_accepted_again() {
        let secret = rfc_secret();
        let now = 1_234_567_890 * SECOND;
        let step = step_at(now);
        let current = code_at_step(&secret, step);
        let previous = code_at_step(&secret, step - 1);
        let next = code_at_step(&secret, step + 1);

        assert_eq!(matching_step(&secret, &current, now, Some(step)), None);
        assert_eq!(matching_step(&secret, &previous, now, Some(step)), None);
        assert_eq!(
            matching_step(&secret, &next, now, Some(step)),
            Some(step + 1)
        );
        assert_eq!(
            matching_step(&secret, &current, now, Some(step - 1)),
            Some(step)
        );
    }

    #[test]
    fn step_boundaries_follow_the_epoch() {
        assert_eq!(step_at(0), 0);
        assert_eq!(step_at(29_999), 0);
        assert_eq!(step_at(30_000), 1);
        assert_eq!(step_at(-1), -1);
    }

    #[test]
    fn only_six_ascii_digits_are_codes() {
        let secret = rfc_secret();
        let now = 59 * SECOND;
        assert_eq!(matching_step(&secret, "287082", now, None), Some(1));
        for bad in [
            "",
            "28708",
            "2870820",
            " 287082",
            "28708a",
            "２８７０８２",
            "+28708",
        ] {
            assert_eq!(matching_step(&secret, bad, now, None), None, "{bad:?}");
            assert!(!is_code_shaped(bad), "{bad:?}");
        }
        assert!(is_code_shaped("000000"));
    }

    #[test]
    fn seeds_round_trip_through_base32_and_bad_ones_are_refused() {
        let secret = TotpSecret::generate().unwrap();
        let text = secret.to_base32();

        assert_eq!(text.len(), 32);
        let parsed = TotpSecret::from_base32(&text.to_lowercase()).unwrap();
        assert_eq!(parsed.bytes(), secret.bytes());
        assert!(TotpSecret::from_base32("").is_none());
        assert!(TotpSecret::from_base32("not base32!").is_none());
        let too_long = BASE32_NOPAD.encode(&[1u8; MAX_SECRET_LEN + 1]);
        assert!(TotpSecret::from_base32(&too_long).is_none());
        let longest = BASE32_NOPAD.encode(&[1u8; MAX_SECRET_LEN]);
        assert!(TotpSecret::from_base32(&longest).is_some());
    }

    #[test]
    fn generated_seeds_differ_and_debug_hides_them() {
        let (a, b) = (
            TotpSecret::generate().unwrap(),
            TotpSecret::generate().unwrap(),
        );
        assert_ne!(a.bytes(), b.bytes());
        assert_eq!(format!("{a:?}"), "TotpSecret(<redacted>)");
    }

    #[test]
    fn provisioning_uri_follows_the_key_uri_format() {
        let secret = TotpSecret::from_base32("JBSWY3DPEHPK3PXP").unwrap();

        let uri = provisioning_uri(&secret, "deskhaven", "alice@example.com");

        assert_eq!(
            uri.as_str(),
            "otpauth://totp/deskhaven:alice%40example.com?secret=JBSWY3DPEHPK3PXP\
             &issuer=deskhaven&algorithm=SHA1&digits=6&period=30"
        );
    }
}
