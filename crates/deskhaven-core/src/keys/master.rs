//! Master key resolution (05-config §2.2): `security.master_key` if set,
//! else `{data_dir}/master.key` (generated on first start, 0600).

use std::path::{Path, PathBuf};

use deskhaven_config::Secret;
use zeroize::Zeroizing;

use super::fs::{self, Visibility};
use super::{KeyError, Loaded};
use crate::crypto::MasterKey;

pub const MASTER_KEY_FILE: &str = "master.key";

pub fn master_key_path(data_dir: &Path) -> PathBuf {
    data_dir.join(MASTER_KEY_FILE)
}

/// The configured or on-disk master key; `None` if neither exists. Never writes.
pub fn read_master_key(
    configured: &Secret,
    data_dir: &Path,
) -> Result<Option<Loaded<MasterKey>>, KeyError> {
    if !configured.is_empty() {
        let key =
            MasterKey::from_base64(configured.expose()).map_err(KeyError::ConfiguredMasterKey)?;
        return Ok(Some(Loaded::new(key, Vec::new())));
    }
    read_file(&master_key_path(data_dir))
}

/// Like [`read_master_key`], but generates `master.key` when absent. A
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
    let contents = Zeroizing::new(format!("{}\n", key.to_base64().as_str()));
    fs::ensure_parent_dir(&path)?;
    fs::create_new(&path, contents.as_bytes(), Visibility::Private)?;
    Ok(Loaded::new(key, Vec::new()))
}

fn read_file(path: &Path) -> Result<Option<Loaded<MasterKey>>, KeyError> {
    let Some(text) = fs::read_optional(path)? else {
        return Ok(None);
    };
    let key = MasterKey::from_base64(&text).map_err(|source| KeyError::MasterKeyFile {
        path: path.to_path_buf(),
        source,
    })?;
    let warnings = fs::permission_warning(path)?.into_iter().collect();
    Ok(Some(Loaded::new(key, warnings)))
}
