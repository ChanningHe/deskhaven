//! Master key resolution (05-config §2.2): `security.master_key` if set,
//! else `{data_dir}/master.key` (generated on first start, 0600).

use std::path::{Path, PathBuf};

use deskhaven_config::Secret;
use zeroize::Zeroizing;

use super::fs::{self, Visibility};
use super::read::{Sensitivity, read_key_file};
use super::{KeyError, KeyWarning, Loaded};
use crate::crypto::MasterKey;

pub const MASTER_KEY_FILE: &str = "master.key";

pub fn master_key_path(data_dir: &Path) -> PathBuf {
    data_dir.join(MASTER_KEY_FILE)
}

/// The configured or on-disk master key; `None` if neither exists. Never
/// writes. A configured key wins; a differing `master.key` is a warning.
pub fn read_master_key(
    configured: &Secret,
    data_dir: &Path,
) -> Result<Option<Loaded<MasterKey>>, KeyError> {
    let path = master_key_path(data_dir);
    if configured.is_empty() {
        return read_file(&path);
    }
    let key = MasterKey::from_base64(configured.expose()).map_err(KeyError::ConfiguredMasterKey)?;
    let warnings = conflict_warning(&key, &path).into_iter().collect();
    Ok(Some(Loaded::new(key, warnings)))
}

/// Like [`read_master_key`], but generates `master.key` when absent, then
/// reads back whatever is on disk (a concurrent winner's key included). A
/// present but unreadable or malformed file is an error, never replaced:
/// a new key would orphan every existing ciphertext.
pub fn load_or_generate_master_key(
    configured: &Secret,
    data_dir: &Path,
) -> Result<Loaded<MasterKey>, KeyError> {
    if let Some(loaded) = read_master_key(configured, data_dir)? {
        return Ok(loaded);
    }
    let path = master_key_path(data_dir);
    let key = MasterKey::generate().map_err(KeyError::Generate)?;
    fs::ensure_parent_dir(&path)?;
    fs::create_new(&path, file_contents(&key).as_bytes(), Visibility::Private)?;
    read_file(&path)?.ok_or_else(|| KeyError::vanished(&path))
}

/// `<base64>\n`, built without reallocation copies of the key text.
fn file_contents(key: &MasterKey) -> Zeroizing<String> {
    let encoded = key.to_base64();
    let mut text = Zeroizing::new(String::with_capacity(encoded.len() + 1));
    text.push_str(&encoded);
    text.push('\n');
    text
}

/// Any `master.key` that is not the configured key (including one that
/// cannot be read) is ignored, which the operator must know about.
fn conflict_warning(configured: &MasterKey, path: &Path) -> Option<KeyWarning> {
    match read_file(path) {
        Ok(None) => None,
        Ok(Some(file)) if file.value.key_id() == configured.key_id() => None,
        Ok(Some(_)) | Err(_) => Some(KeyWarning::ConflictingMasterKeyFile {
            path: path.to_path_buf(),
        }),
    }
}

fn read_file(path: &Path) -> Result<Option<Loaded<MasterKey>>, KeyError> {
    let Some(file) = read_key_file(path, Sensitivity::Private)? else {
        return Ok(None);
    };
    let key = MasterKey::from_base64(&file.text).map_err(|source| KeyError::MasterKeyFile {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(Some(Loaded::new(key, file.warnings)))
}
