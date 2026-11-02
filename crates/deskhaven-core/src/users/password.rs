//! Argon2id password hashing (06 §3.1).
//!
//! Hashes are PHC strings. Every hash and verification runs on the blocking pool: one
//! computation at the default parameters takes tens of milliseconds of CPU.

use std::fmt;
use std::sync::Arc;

use argon2::password_hash::{PasswordHash, PasswordHasher as _, PasswordVerifier as _, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use zeroize::Zeroizing;

use crate::crypto::fill_random;

/// Salt length in bytes (the PHC recommendation, 128 bits).
const SALT_LEN: usize = 16;
/// Salt of [`PasswordHasher::verify_dummy`]. Its value is irrelevant: the output is discarded.
const DUMMY_SALT: [u8; SALT_LEN] = [0x5a; SALT_LEN];

/// A plaintext password. Zeroed on drop; `Debug` never shows it and there is no `Display`.
#[derive(Clone)]
pub struct Password(Zeroizing<String>);

impl Password {
    pub fn new(value: impl Into<String>) -> Self {
        Self(Zeroizing::new(value.into()))
    }

    /// The plaintext. Call only where the value is actually used.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Length in Unicode scalar values (what the policy counts).
    pub fn char_len(&self) -> usize {
        self.0.chars().count()
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(<redacted>)")
    }
}

/// Argon2id cost parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HashParams {
    /// Memory cost in KiB.
    pub memory_kib: u32,
    /// Number of passes.
    pub iterations: u32,
    /// Degree of parallelism (lanes).
    pub parallelism: u32,
}

impl HashParams {
    /// m=19 MiB, t=2, p=1: the OWASP baseline (06 §3.1). Changing it makes every older hash
    /// [`needs_rehash`](PasswordHasher::needs_rehash), upgraded at the next login.
    pub const DEFAULT: Self = Self {
        memory_kib: 19 * 1024,
        iterations: 2,
        parallelism: 1,
    };
}

impl Default for HashParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Failures of hashing and verification. Never carries the password.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PasswordError {
    #[error("invalid argon2 parameters")]
    Params,

    /// A stored hash that is not a usable PHC string (data corruption, foreign format).
    #[error("stored password hash is malformed")]
    MalformedHash,

    #[error("password hashing failed")]
    Hash,

    #[error("the operating system random number generator failed")]
    Random,

    /// The blocking task panicked or the runtime is shutting down.
    #[error("password hashing task did not complete")]
    Aborted,
}

/// Hashes and verifies passwords with fixed Argon2id parameters. Cheap to clone.
#[derive(Clone, Debug)]
pub struct PasswordHasher {
    params: Arc<Params>,
}

impl Default for PasswordHasher {
    /// [`HashParams::DEFAULT`] (argon2's own default is the same OWASP baseline; a unit test
    /// keeps the two equal, so this needs no fallible constructor).
    fn default() -> Self {
        Self {
            params: Arc::new(Params::default()),
        }
    }
}

impl PasswordHasher {
    pub fn new(params: HashParams) -> Result<Self, PasswordError> {
        let params = Params::new(
            params.memory_kib,
            params.iterations,
            params.parallelism,
            None,
        )
        .map_err(|_| PasswordError::Params)?;
        Ok(Self {
            params: Arc::new(params),
        })
    }

    /// A new PHC string for `password`, with a fresh random salt.
    pub async fn hash(&self, password: &Password) -> Result<String, PasswordError> {
        let mut salt = [0u8; SALT_LEN];
        fill_random(&mut salt).map_err(|_| PasswordError::Random)?;
        let salt = SaltString::encode_b64(&salt).map_err(|_| PasswordError::Hash)?;
        self.blocking(password, move |argon2, pwd| {
            argon2
                .hash_password(pwd, &salt)
                .map(|hash| hash.to_string())
                .map_err(|_| PasswordError::Hash)
        })
        .await
    }

    /// Whether `password` matches the PHC string `phc`, computed with the parameters stored
    /// in `phc`. A malformed `phc` is an error, not a mismatch.
    pub async fn verify(&self, password: &Password, phc: &str) -> Result<bool, PasswordError> {
        let phc = phc.to_owned();
        self.blocking(password, move |argon2, pwd| {
            let hash = PasswordHash::new(&phc).map_err(|_| PasswordError::MalformedHash)?;
            // Without these `verify_password` reports a plain mismatch.
            if hash.salt.is_none() || hash.hash.is_none() {
                return Err(PasswordError::MalformedHash);
            }
            match argon2.verify_password(pwd, &hash) {
                Ok(()) => Ok(true),
                Err(argon2::password_hash::Error::Password) => Ok(false),
                Err(_) => Err(PasswordError::MalformedHash),
            }
        })
        .await
    }

    /// Spends the cost of one verification at the current parameters and discards the
    /// result: for unknown or non-local accounts, so their failures take as long as a wrong
    /// password (06 §3.6).
    pub async fn verify_dummy(&self, password: &Password) -> Result<(), PasswordError> {
        let salt = SaltString::encode_b64(&DUMMY_SALT).map_err(|_| PasswordError::Hash)?;
        self.blocking(password, move |argon2, pwd| {
            argon2
                .hash_password(pwd, &salt)
                .map(drop)
                .map_err(|_| PasswordError::Hash)
        })
        .await
    }

    /// Whether `phc` was not produced by this hasher's algorithm, version and parameters
    /// (including anything unparsable), so it should be replaced at the next login.
    pub fn needs_rehash(&self, phc: &str) -> bool {
        let Ok(hash) = PasswordHash::new(phc) else {
            return true;
        };
        let output_len = |p: &Params| p.output_len().unwrap_or(Params::DEFAULT_OUTPUT_LEN);
        let same_params = Params::try_from(&hash).is_ok_and(|p| {
            p.m_cost() == self.params.m_cost()
                && p.t_cost() == self.params.t_cost()
                && p.p_cost() == self.params.p_cost()
                && output_len(&p) == output_len(&self.params)
        });
        hash.algorithm != argon2::ARGON2ID_IDENT
            || hash.version != Some(Version::V0x13.into())
            || hash.salt.is_none()
            || !same_params
    }

    /// Runs `op` with an Argon2id instance and a zeroizing copy of the password on the
    /// blocking pool.
    async fn blocking<T, F>(&self, password: &Password, op: F) -> Result<T, PasswordError>
    where
        T: Send + 'static,
        F: FnOnce(&Argon2<'static>, &[u8]) -> Result<T, PasswordError> + Send + 'static,
    {
        let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, (*self.params).clone());
        let password = password.clone();
        tokio::task::spawn_blocking(move || op(&argon2, password.expose().as_bytes()))
            .await
            .map_err(|_| PasswordError::Aborted)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_hasher_uses_the_documented_params() {
        let documented = HashParams::DEFAULT;
        let params = PasswordHasher::default().params;

        assert_eq!(params.m_cost(), documented.memory_kib);
        assert_eq!(params.t_cost(), documented.iterations);
        assert_eq!(params.p_cost(), documented.parallelism);
    }
}
