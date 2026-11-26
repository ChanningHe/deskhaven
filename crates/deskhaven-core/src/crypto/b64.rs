use base64_022::alphabet;
use base64_022::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

/// Padded on output exactly like upstream hbbs; padding is optional on input so hand-edited
/// values still load.
pub(crate) const KEY_B64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

pub(crate) const SEALED_B64: GeneralPurpose = base64_022::engine::general_purpose::STANDARD_NO_PAD;
