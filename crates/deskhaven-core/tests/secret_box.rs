//! SecretBox: XChaCha20-Poly1305 with `v1:<key_id>:<nonce>:<ct>` strings and
//! AAD = `"v1:<key_id>:" || context`.

#![allow(clippy::unwrap_used, clippy::panic)]

use deskhaven_core::crypto::{CryptoError, MasterKey, SecretBox};

const KEY_A: [u8; 32] = [7; 32];
const KEY_B: [u8; 32] = [9; 32];
const CTX: &str = "users.totp_secret_enc:42";

/// Computed independently with libsodium (`crypto_aead_xchacha20poly1305_ietf_encrypt`)
/// and hashlib, not with this crate: key [7; 32], nonce 0..=23, context
/// `users.totp_secret_enc:42`, plaintext `JBSWY3DPEHPK3PXP`. Pins the string
/// format, the AAD layout and the key id derivation (KEY_ID_DOMAIN).
const KAT_SEALED: &str =
    "v1:330d29e5:AAECAwQFBgcICQoLDA0ODxAREhMUFRYX:4lX40qMvH/Vgj9EM1bL3/OfB04v47eYPmIy3Zv2ltmM";
const KAT_PLAINTEXT: &[u8] = b"JBSWY3DPEHPK3PXP";

const MAX_PLAINTEXT_LEN: usize = 512 * 1024;
const MAX_SEALED_LEN: usize = 1024 * 1024;

fn boxed(key: [u8; 32]) -> SecretBox {
    SecretBox::new(&MasterKey::from_bytes(&key))
}

fn parts(sealed: &str) -> Vec<String> {
    sealed.split(':').map(str::to_owned).collect()
}

#[test]
fn known_answer_vector_opens() {
    assert_eq!(
        boxed(KEY_A).open(CTX, KAT_SEALED).unwrap().as_slice(),
        KAT_PLAINTEXT
    );
}

#[test]
fn known_key_ids_are_pinned() {
    assert_eq!(
        MasterKey::from_bytes(&KEY_A).key_id().to_string(),
        "330d29e5"
    );
    assert_eq!(
        MasterKey::from_bytes(&KEY_B).key_id().to_string(),
        "02b408d5"
    );
}

#[test]
fn seal_then_open_round_trips() {
    let sb = boxed(KEY_A);
    let sealed = sb.seal(CTX, b"hunter2").unwrap();
    assert_eq!(sb.open(CTX, &sealed).unwrap().as_slice(), b"hunter2");
}

#[test]
fn empty_plaintext_round_trips() {
    let sb = boxed(KEY_A);
    let sealed = sb.seal(CTX, b"").unwrap();
    assert!(sb.open(CTX, &sealed).unwrap().is_empty());
}

#[test]
fn other_context_fails_authentication() {
    let sb = boxed(KEY_A);
    let sealed = sb.seal("users.totp_secret_enc:42", b"seed").unwrap();
    for other in ["users.totp_secret_enc:43", "oidc.client_secret_enc:42", ""] {
        assert_eq!(
            sb.open(other, &sealed),
            Err(CryptoError::Authentication),
            "{other}"
        );
    }
    assert_eq!(
        sb.open("users.totp_secret_enc:43", KAT_SEALED),
        Err(CryptoError::Authentication)
    );
}

#[test]
fn sealed_format_is_v1_keyid_nonce_ct() {
    let key = MasterKey::from_bytes(&KEY_A);
    let sealed = SecretBox::new(&key).seal(CTX, b"x").unwrap();
    let p = parts(&sealed);
    assert_eq!(p.len(), 4, "{sealed}");
    assert_eq!(p[0], "v1");
    assert_eq!(p[1], key.key_id().to_string());
    // 24-byte nonce, unpadded base64; the context is not part of the string.
    assert_eq!(p[2].len(), 32);
    assert!(!sealed.contains('=') && !sealed.contains("users"));
}

#[test]
fn key_id_is_stable_and_distinguishes_keys() {
    let a = MasterKey::from_bytes(&KEY_A);
    assert_eq!(a.key_id(), MasterKey::from_bytes(&KEY_A).key_id());
    assert_ne!(a.key_id(), MasterKey::from_bytes(&KEY_B).key_id());
}

#[test]
fn every_seal_uses_a_fresh_nonce() {
    let sb = boxed(KEY_A);
    let first = sb.seal(CTX, b"same").unwrap();
    let second = sb.seal(CTX, b"same").unwrap();
    assert_ne!(parts(&first)[2], parts(&second)[2]);
}

fn flip_first_char(field: &str) -> String {
    let mut bytes = field.as_bytes().to_vec();
    bytes[0] = if bytes[0] == b'A' { b'B' } else { b'A' };
    String::from_utf8(bytes).unwrap()
}

#[test]
fn tampered_ciphertext_or_nonce_fails_authentication() {
    let sb = boxed(KEY_A);
    for field in [2, 3] {
        let mut p = parts(KAT_SEALED);
        p[field] = flip_first_char(&p[field]);
        assert_eq!(
            sb.open(CTX, &p.join(":")),
            Err(CryptoError::Authentication),
            "{field}"
        );
    }
}

#[test]
fn other_master_key_reports_unknown_key_id() {
    assert_eq!(
        boxed(KEY_B).open(CTX, KAT_SEALED),
        Err(CryptoError::UnknownKeyId)
    );
}

#[test]
fn forged_key_id_reports_unknown_key_id() {
    let mut p = parts(KAT_SEALED);
    p[1] = "00000000".to_owned();
    assert_eq!(
        boxed(KEY_A).open(CTX, &p.join(":")),
        Err(CryptoError::UnknownKeyId)
    );
}

#[test]
fn relabelled_to_another_known_key_fails_authentication() {
    let b = MasterKey::from_bytes(&KEY_B);
    let sb = boxed(KEY_A).with_previous(&b).unwrap();
    let mut p = parts(KAT_SEALED);
    p[1] = b.key_id().to_string();
    assert_eq!(sb.open(CTX, &p.join(":")), Err(CryptoError::Authentication));
}

#[test]
fn malformed_inputs_are_rejected_as_malformed() {
    let sb = boxed(KEY_A);
    let p = parts(KAT_SEALED);
    let cases = [
        String::new(),
        "v1".to_owned(),
        format!("v2:{}:{}:{}", p[1], p[2], p[3]),
        format!("v1:{}:{}", p[1], p[2]),
        format!("{KAT_SEALED}:extra"),
        format!("v1:XYZ:{}:{}", p[1], p[3]),
        format!("v1:{}:{}:{}", p[1].to_uppercase(), p[2], p[3]),
        format!("v1:{}:not*base64:{}", p[1], p[3]),
        format!("v1:{}:{}:{}", p[1], &p[2][..20], p[3]),
        format!("v1:{}:{}:{}=", p[1], p[2], p[3]),
        format!("v1:{}:{}:AAAA", p[1], p[2]),
    ];
    for case in cases {
        assert_eq!(sb.open(CTX, &case), Err(CryptoError::Malformed), "{case:?}");
    }
}

#[test]
fn oversized_inputs_are_refused_before_decoding() {
    let sb = boxed(KEY_A);
    let huge = format!("{KAT_SEALED}{}", "A".repeat(MAX_SEALED_LEN));
    assert_eq!(sb.open(CTX, &huge), Err(CryptoError::Malformed));

    let at_limit = vec![0u8; MAX_PLAINTEXT_LEN];
    let sealed = sb.seal(CTX, &at_limit).unwrap();
    assert!(sealed.len() <= MAX_SEALED_LEN);
    assert_eq!(sb.open(CTX, &sealed).unwrap().len(), MAX_PLAINTEXT_LEN);
    let over = vec![0u8; MAX_PLAINTEXT_LEN + 1];
    assert_eq!(sb.seal(CTX, &over), Err(CryptoError::PlaintextTooLarge));
}

#[test]
fn rotation_seals_with_primary_and_opens_previous() {
    let old = MasterKey::from_bytes(&KEY_A);
    let new = MasterKey::from_bytes(&KEY_B);
    let rotated = SecretBox::new(&new).with_previous(&old).unwrap();

    assert_eq!(
        rotated.open(CTX, KAT_SEALED).unwrap().as_slice(),
        KAT_PLAINTEXT
    );
    let fresh = rotated.seal(CTX, b"fresh").unwrap();
    assert_eq!(parts(&fresh)[1], new.key_id().to_string());
}

#[test]
fn duplicate_key_ids_are_rejected() {
    let a = MasterKey::from_bytes(&KEY_A);
    let b = MasterKey::from_bytes(&KEY_B);
    assert_eq!(
        SecretBox::new(&a).with_previous(&a).err(),
        Some(CryptoError::DuplicateKeyId)
    );
    let twice = SecretBox::new(&a)
        .with_previous(&b)
        .unwrap()
        .with_previous(&b);
    assert_eq!(twice.err(), Some(CryptoError::DuplicateKeyId));
}

/// Drives every real error path with a planted secret and checks that no
/// rendering (Display, Debug, source chain) leaks it, the key, or input.
#[test]
fn error_paths_and_debug_never_leak_planted_material() {
    const PLANTED: &str = "PLANTED-SECRET-c0ffee";
    let key = MasterKey::from_bytes(&KEY_A);
    let sb = SecretBox::new(&key);
    let sealed = sb.seal(CTX, PLANTED.as_bytes()).unwrap();
    let mut p = parts(&sealed);
    p[3] = flip_first_char(&p[3]);
    let tampered = p.join(":");

    let errors = [
        sb.open("other:1", &sealed).unwrap_err(),
        sb.open(CTX, &tampered).unwrap_err(),
        boxed(KEY_B).open(CTX, &sealed).unwrap_err(),
        sb.open(CTX, &format!("{sealed}:{PLANTED}")).unwrap_err(),
        sb.open(CTX, PLANTED).unwrap_err(),
        MasterKey::from_base64(PLANTED).unwrap_err(),
        MasterKey::from_base64("UExBTlRFRC1TRUNSRVQ=").unwrap_err(),
    ];
    let mut rendered: Vec<String> = errors
        .iter()
        .flat_map(|e| [e.to_string(), format!("{e:?}")])
        .collect();
    rendered.push(format!("{sb:?}"));
    rendered.push(format!("{key:?}"));

    let key_b64 = key.to_base64();
    let ct_field = parts(&sealed)[3].clone();
    for text in rendered {
        for needle in [
            PLANTED,
            "UExBTlRFRC1TRUNSRVQ",
            key_b64.as_str(),
            &ct_field,
            "[7, 7",
        ] {
            assert!(!text.contains(needle), "{needle:?} leaked in {text:?}");
        }
    }
    assert!(format!("{key:?}").contains("330d29e5"));
}

#[test]
fn master_key_base64_round_trips() {
    let key = MasterKey::from_bytes(&KEY_A);
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
fn master_key_rejects_wrong_length_or_encoding() {
    let err = MasterKey::from_base64("c2hvcnQta2V5LXZhbHVl").unwrap_err();
    assert!(matches!(err, CryptoError::InvalidMasterKey(_)), "{err:?}");
    assert!(err.to_string().contains("32"), "{err}");
    let err = MasterKey::from_base64("not base64 !!").unwrap_err();
    assert!(matches!(err, CryptoError::InvalidMasterKey(_)), "{err:?}");
}

#[test]
fn generated_master_keys_differ() {
    let a = MasterKey::generate().unwrap();
    let b = MasterKey::generate().unwrap();
    assert_ne!(a.key_id(), b.key_id());
}

// --- keyed MAC (06 §3.3: k_hmac = HKDF(master key, "deskhaven/hmac/v1")) ------------------

/// Computed independently with OpenSSL, not with this crate:
/// `openssl kdf -keylen 32 -kdfopt digest:SHA256 -kdfopt hexkey:07…07
/// -kdfopt info:deskhaven/hmac/v1 HKDF` (no salt) gives k_hmac; then
/// `openssl dgst -sha256 -mac HMAC -macopt hexkey:<k_hmac>` over
/// `u64be(27) || "recovery_codes.code_hash:42" || "abcde23456"`, standard base64 unpadded.
const KAT_MAC: &str = "v1:330d29e5:+SPsmDaIQ81ziO4r9L0v2HEcGtbk6nCSt+1NQRjvL/I";
const MAC_CTX: &str = "recovery_codes.code_hash:42";

#[test]
fn mac_matches_the_known_answer_vector() {
    assert_eq!(boxed(KEY_A).mac(MAC_CTX, b"abcde23456"), KAT_MAC);
}

#[test]
fn mac_is_deterministic_and_bound_to_key_context_and_data() {
    let sb = boxed(KEY_A);
    let tag = sb.mac(MAC_CTX, b"abcde23456");

    assert_eq!(sb.mac(MAC_CTX, b"abcde23456"), tag);
    assert_ne!(boxed(KEY_B).mac(MAC_CTX, b"abcde23456"), tag);
    assert_ne!(sb.mac("recovery_codes.code_hash:43", b"abcde23456"), tag);
    assert_ne!(sb.mac(MAC_CTX, b"abcde23457"), tag);
    // The context is length-prefixed: moving bytes between context and data changes the tag.
    assert_ne!(sb.mac("recovery_codes.code_hash:4", b"2abcde23456"), tag);
}

#[test]
fn mac_uses_the_primary_key_after_rotation() {
    let old = MasterKey::from_bytes(&KEY_A);
    let new = MasterKey::from_bytes(&KEY_B);
    let rotated = SecretBox::new(&new).with_previous(&old).unwrap();

    assert_eq!(rotated.mac(MAC_CTX, b"x"), boxed(KEY_B).mac(MAC_CTX, b"x"));
    assert!(rotated.mac(MAC_CTX, b"x").starts_with("v1:02b408d5:"));
}
