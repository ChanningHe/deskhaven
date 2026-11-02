//! SecretBox: XChaCha20-Poly1305 with `v1:<key_id>:<nonce>:<ct>` strings.

#![allow(clippy::unwrap_used, clippy::panic)]

use deskhaven_core::crypto::{CryptoError, MasterKey, SecretBox};

const KEY_A: [u8; 32] = [7; 32];
const KEY_B: [u8; 32] = [9; 32];

fn boxed(key: [u8; 32]) -> SecretBox {
    SecretBox::new(&MasterKey::from_bytes(key))
}

fn parts(sealed: &str) -> Vec<String> {
    sealed.split(':').map(str::to_owned).collect()
}

#[test]
fn seal_then_open_round_trips() {
    let sb = boxed(KEY_A);
    let sealed = sb.seal(b"hunter2").unwrap();
    assert_eq!(sb.open(&sealed).unwrap().as_slice(), b"hunter2");
}

#[test]
fn empty_plaintext_round_trips() {
    let sb = boxed(KEY_A);
    let sealed = sb.seal(b"").unwrap();
    assert!(sb.open(&sealed).unwrap().is_empty());
}

#[test]
fn sealed_format_is_v1_keyid_nonce_ct() {
    let key = MasterKey::from_bytes(KEY_A);
    let sealed = SecretBox::new(&key).seal(b"x").unwrap();
    let p = parts(&sealed);
    assert_eq!(p.len(), 4, "{sealed}");
    assert_eq!(p[0], "v1");
    assert_eq!(p[1], key.key_id().to_string());
    assert_eq!(p[1].len(), 8);
    assert!(
        p[1].chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    );
    // 24-byte nonce, unpadded base64.
    assert_eq!(p[2].len(), 32);
    assert!(!sealed.contains('='));
}

#[test]
fn key_id_is_stable_and_distinguishes_keys() {
    let a = MasterKey::from_bytes(KEY_A);
    assert_eq!(a.key_id(), MasterKey::from_bytes(KEY_A).key_id());
    assert_ne!(a.key_id(), MasterKey::from_bytes(KEY_B).key_id());
}

#[test]
fn every_seal_uses_a_fresh_nonce() {
    let sb = boxed(KEY_A);
    let first = sb.seal(b"same").unwrap();
    let second = sb.seal(b"same").unwrap();
    assert_ne!(parts(&first)[2], parts(&second)[2]);
    assert_ne!(first, second);
}

#[test]
fn tampered_ciphertext_fails_authentication() {
    let sb = boxed(KEY_A);
    let mut p = parts(&sb.seal(b"secret value").unwrap());
    let mut ct = p[3].clone().into_bytes();
    ct[0] = if ct[0] == b'A' { b'B' } else { b'A' };
    p[3] = String::from_utf8(ct).unwrap();
    assert_eq!(sb.open(&p.join(":")), Err(CryptoError::Authentication));
}

#[test]
fn tampered_nonce_fails_authentication() {
    let sb = boxed(KEY_A);
    let mut p = parts(&sb.seal(b"secret value").unwrap());
    let mut nonce = p[2].clone().into_bytes();
    nonce[0] = if nonce[0] == b'A' { b'B' } else { b'A' };
    p[2] = String::from_utf8(nonce).unwrap();
    assert_eq!(sb.open(&p.join(":")), Err(CryptoError::Authentication));
}

#[test]
fn other_master_key_reports_unknown_key_id() {
    let sealed = boxed(KEY_A).seal(b"secret value").unwrap();
    assert_eq!(boxed(KEY_B).open(&sealed), Err(CryptoError::UnknownKeyId));
}

#[test]
fn forged_key_id_reports_unknown_key_id() {
    let sb = boxed(KEY_A);
    let mut p = parts(&sb.seal(b"secret value").unwrap());
    p[1] = "00000000".to_owned();
    assert_eq!(sb.open(&p.join(":")), Err(CryptoError::UnknownKeyId));
}

#[test]
fn relabelled_to_another_known_key_fails_authentication() {
    let a = MasterKey::from_bytes(KEY_A);
    let b = MasterKey::from_bytes(KEY_B);
    let sb = SecretBox::new(&a).with_previous(&b);
    let mut p = parts(&sb.seal(b"secret value").unwrap());
    p[1] = b.key_id().to_string();
    assert_eq!(sb.open(&p.join(":")), Err(CryptoError::Authentication));
}

#[test]
fn malformed_inputs_are_rejected_as_malformed() {
    let sb = boxed(KEY_A);
    let good = sb.seal(b"x").unwrap();
    let p = parts(&good);
    let cases = [
        String::new(),
        "v1".to_owned(),
        format!("v2:{}:{}:{}", p[1], p[2], p[3]),
        format!("v1:{}:{}", p[1], p[2]),
        format!("{good}:extra"),
        format!("v1:XYZ:{}:{}", p[2], p[3]),
        format!("v1:{}:{}:{}", p[1].to_uppercase(), p[2], p[3]),
        format!("v1:{}:not*base64:{}", p[1], p[3]),
        format!("v1:{}:{}:{}", p[1], &p[2][..20], p[3]),
        format!("v1:{}:{}:{}=", p[1], p[2], p[3]),
        format!("v1:{}:{}:AAAA", p[1], p[2]),
    ];
    for case in cases {
        assert_eq!(sb.open(&case), Err(CryptoError::Malformed), "{case:?}");
    }
}

#[test]
fn rotation_seals_with_primary_and_opens_previous() {
    let old = MasterKey::from_bytes(KEY_A);
    let new = MasterKey::from_bytes(KEY_B);
    let legacy = SecretBox::new(&old).seal(b"legacy").unwrap();

    let rotated = SecretBox::new(&new).with_previous(&old);
    assert_eq!(rotated.open(&legacy).unwrap().as_slice(), b"legacy");
    let fresh = rotated.seal(b"fresh").unwrap();
    assert_eq!(parts(&fresh)[1], new.key_id().to_string());
}

#[test]
fn errors_and_debug_never_contain_plaintext_or_key() {
    let key = MasterKey::from_bytes(KEY_A);
    let sb = SecretBox::new(&key);
    let rendered = [
        CryptoError::Malformed.to_string(),
        CryptoError::UnknownKeyId.to_string(),
        CryptoError::Authentication.to_string(),
        format!("{sb:?}"),
        format!("{key:?}"),
    ];
    let key_b64 = key.to_base64();
    for text in rendered {
        assert!(!text.contains(key_b64.as_str()), "{text}");
        assert!(!text.contains("[7, 7"), "{text}");
    }
    assert!(format!("{key:?}").contains(&key.key_id().to_string()));
}

#[test]
fn master_key_base64_round_trips() {
    let key = MasterKey::from_bytes(KEY_A);
    let parsed = MasterKey::from_base64(&key.to_base64()).unwrap();
    assert_eq!(parsed.key_id(), key.key_id());
    // Surrounding whitespace (e.g. a trailing newline from a _FILE secret) is tolerated.
    let padded = format!(" {}\n", key.to_base64().as_str());
    assert_eq!(
        MasterKey::from_base64(&padded).unwrap().key_id(),
        key.key_id()
    );
}

#[test]
fn master_key_rejects_wrong_length_without_echoing() {
    let short = "c2hvcnQta2V5LXZhbHVl"; // "short-key-value"
    let err = MasterKey::from_base64(short).unwrap_err();
    assert!(matches!(err, CryptoError::InvalidMasterKey(_)), "{err:?}");
    assert!(!err.to_string().contains(short));
    assert!(err.to_string().contains("32"), "{err}");

    let junk = "not base64 !!";
    let err = MasterKey::from_base64(junk).unwrap_err();
    assert!(matches!(err, CryptoError::InvalidMasterKey(_)), "{err:?}");
    assert!(!err.to_string().contains(junk));
}

#[test]
fn generated_master_keys_differ() {
    let a = MasterKey::generate().unwrap();
    let b = MasterKey::generate().unwrap();
    assert_ne!(a.key_id(), b.key_id());
}
