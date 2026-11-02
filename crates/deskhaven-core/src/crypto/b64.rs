//! Base64 engines shared by key files and sealed strings.

use base64_022::alphabet;
use base64_022::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

/// Key material (master key, hbbs key files): standard alphabet, padded on
/// output exactly like upstream hbbs (`base64` 0.13 `encode`); padding is
/// optional on input so hand-edited values still load.
pub(crate) const KEY_B64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// Sealed-secret fields: standard alphabet, unpadded, strict on input.
pub(crate) const SEALED_B64: GeneralPurpose = base64_022::engine::general_purpose::STANDARD_NO_PAD;
