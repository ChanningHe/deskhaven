//! hbbs key files (05-config §5): `id_ed25519` + `id_ed25519.pub`, byte
//! compatible with upstream so an OSS hbbs key can be copied in unchanged.
//!
//! Mirrors upstream `gen_sk` (rustdesk-server src/common.rs): contents are
//! trimmed on read, written without a trailing newline; an existing `.pub`
//! must match the private key; a missing `.pub` is recreated; a `.pub`
//! without its private key blocks generation. Unlike upstream, permissive
//! private key modes are reported as warnings instead of being chmod-ed.

use std::path::{Path, PathBuf};

use deskhaven_config::bootstrap::Bootstrap;

use super::fs::{self, Visibility};
use super::{KeyError, KeyWarning, Loaded, SignalKeyPair, render_chain};

pub const SIGNAL_KEY_FILE: &str = "id_ed25519";
const PUBLIC_SUFFIX: &str = ".pub";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalKeyPaths {
    secret: PathBuf,
    public: PathBuf,
}

impl SignalKeyPaths {
    /// `key_file` empty = `{data_dir}/id_ed25519`; the public key is
    /// `<key_file>.pub`, as upstream.
    pub fn resolve(key_file: &Path, data_dir: &Path) -> Self {
        let secret = if key_file.as_os_str().is_empty() {
            data_dir.join(SIGNAL_KEY_FILE)
        } else {
            key_file.to_path_buf()
        };
        let mut public = secret.clone().into_os_string();
        public.push(PUBLIC_SUFFIX);
        Self {
            secret,
            public: public.into(),
        }
    }

    pub fn from_bootstrap(config: &Bootstrap) -> Self {
        Self::resolve(&config.signal.key_file, &config.data_dir)
    }

    pub fn secret(&self) -> &Path {
        &self.secret
    }

    pub fn public(&self) -> &Path {
        &self.public
    }
}

enum PublicFile {
    Matches,
    Missing,
}

/// The existing key pair, or `None` if neither file exists. Never writes.
pub fn read_signal_key(paths: &SignalKeyPaths) -> Result<Option<Loaded<SignalKeyPair>>, KeyError> {
    Ok(read_existing(paths)?.map(|(loaded, _)| loaded))
}

/// Like [`read_signal_key`], but generates the pair when absent and
/// recreates a missing `.pub`. Existing files are never overwritten.
pub fn load_or_generate_signal_key(
    paths: &SignalKeyPaths,
) -> Result<Loaded<SignalKeyPair>, KeyError> {
    match read_existing(paths)? {
        Some((loaded, PublicFile::Matches)) => Ok(loaded),
        Some((loaded, PublicFile::Missing)) => restore_public(paths, loaded),
        None => generate(paths),
    }
}

fn read_existing(
    paths: &SignalKeyPaths,
) -> Result<Option<(Loaded<SignalKeyPair>, PublicFile)>, KeyError> {
    let Some(text) = fs::read_optional(&paths.secret)? else {
        if fs::exists(&paths.public)? {
            return Err(KeyError::OrphanPublicKey {
                path: paths.public.clone(),
                secret: paths.secret.clone(),
            });
        }
        return Ok(None);
    };
    let pair =
        SignalKeyPair::from_secret_base64(&text).map_err(|reason| KeyError::InvalidSignalKey {
            path: paths.secret.clone(),
            reason,
        })?;
    let public = check_public(paths, &pair.public_key_base64())?;
    let warnings = fs::permission_warning(&paths.secret)?.into_iter().collect();
    Ok(Some((Loaded::new(pair, warnings), public)))
}

fn check_public(paths: &SignalKeyPaths, public_key: &str) -> Result<PublicFile, KeyError> {
    match fs::read_optional(&paths.public)? {
        None => Ok(PublicFile::Missing),
        Some(text) if text.trim() == public_key => Ok(PublicFile::Matches),
        Some(_) => Err(KeyError::PublicKeyMismatch {
            path: paths.public.clone(),
            secret: paths.secret.clone(),
        }),
    }
}

/// Upstream tolerates an unwritable `.pub` beside an existing private key
/// (read-only provisioning); so do we, with a warning.
fn restore_public(
    paths: &SignalKeyPaths,
    mut loaded: Loaded<SignalKeyPair>,
) -> Result<Loaded<SignalKeyPair>, KeyError> {
    let public_key = loaded.value.public_key_base64();
    let Err(err) = fs::create_new(&paths.public, public_key.as_bytes(), Visibility::Public) else {
        return Ok(loaded);
    };
    // A concurrent writer may have won the race: accept only a matching file.
    if let PublicFile::Missing = check_public(paths, &public_key)? {
        loaded.warnings.push(KeyWarning::PublicKeyNotWritten {
            path: paths.public.clone(),
            reason: render_chain(&err),
        });
    }
    Ok(loaded)
}

fn generate(paths: &SignalKeyPaths) -> Result<Loaded<SignalKeyPair>, KeyError> {
    let pair = SignalKeyPair::generate().map_err(KeyError::Generate)?;
    fs::ensure_parent_dir(&paths.secret)?;
    fs::create_new(
        &paths.secret,
        pair.secret_base64().as_bytes(),
        Visibility::Private,
    )?;
    fs::create_new(
        &paths.public,
        pair.public_key_base64().as_bytes(),
        Visibility::Public,
    )?;
    Ok(Loaded::new(pair, Vec::new()))
}
