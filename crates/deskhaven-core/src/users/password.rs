//! Argon2id password hashing (06 §3.1).
//!
//! Hashes are PHC strings. Every hash and verification runs on the blocking pool: one
//! computation at the default parameters takes tens of milliseconds of CPU and 19 MiB of
//! memory. A semaphore bounds how many run at once, so a login flood cannot fill tokio's
//! blocking pool (512 threads ≈ 9.7 GiB) and take the process, hbbs included, down; callers
//! that cannot get a slot in time get [`PasswordError::Busy`] (APIs: 503).

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use argon2::password_hash::{PasswordHash, PasswordHasher as _, PasswordVerifier as _, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

use crate::crypto::fill_random;

/// Salt length in bytes (the PHC recommendation, 128 bits).
const SALT_LEN: usize = 16;
/// Salt of [`PasswordHasher::verify_dummy`]. Its value is irrelevant: the output is discarded.
const DUMMY_SALT: [u8; SALT_LEN] = [0x5a; SALT_LEN];
/// Lower bound of [`HashLimits::default`]'s concurrency, for hosts reporting one CPU.
const MIN_DEFAULT_CONCURRENCY: usize = 2;
/// How long [`HashLimits::default`] waits for a computation slot.
const DEFAULT_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

/// A plaintext password. Zeroed on drop; `Debug` never shows it and there is no `Display`.
///
/// No Unicode normalization (NFC / NFKC) is applied: the same visible password typed with
/// differently composed characters hashes differently. Accepted trade-off: normalization
/// would need an extra dependency, and clients send what the user's input method produces
/// consistently.
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

/// Bounds on concurrent Argon2 computations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HashLimits {
    /// Computations allowed to run at once (at least 1).
    pub max_concurrent: usize,
    /// How long a caller waits for a slot before [`PasswordError::Busy`].
    pub acquire_timeout: Duration,
}

impl Default for HashLimits {
    /// One computation per CPU (at least 2), waiting up to 5 s.
    fn default() -> Self {
        let cpus = std::thread::available_parallelism().map_or(1, usize::from);
        Self {
            max_concurrent: cpus.max(MIN_DEFAULT_CONCURRENCY),
            acquire_timeout: DEFAULT_ACQUIRE_TIMEOUT,
        }
    }
}

/// Failures of hashing and verification. Never carries the password.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PasswordError {
    #[error("invalid argon2 parameters or limits")]
    Params,

    /// A stored hash that is not a usable PHC string (data corruption, foreign format).
    #[error("stored password hash is malformed")]
    MalformedHash,

    #[error("password hashing failed")]
    Hash,

    #[error("the operating system random number generator failed")]
    Random,

    /// Every computation slot stayed taken for the whole acquire timeout (overload; APIs
    /// answer 503).
    #[error("too many concurrent password computations")]
    Busy,

    /// The blocking task panicked or the runtime is shutting down.
    #[error("password hashing task did not complete")]
    Aborted,
}

/// Hashes and verifies passwords with fixed Argon2id parameters. Cheap to clone; clones share
/// the concurrency limit, so a process should build one and hand out clones.
#[derive(Clone, Debug)]
pub struct PasswordHasher {
    params: Arc<Params>,
    permits: Arc<Semaphore>,
    max_concurrent: usize,
    acquire_timeout: Duration,
}

impl Default for PasswordHasher {
    /// [`HashParams::DEFAULT`] (argon2's own default is the same OWASP baseline; a unit test
    /// keeps the two equal, so this needs no fallible constructor) and [`HashLimits::default`].
    fn default() -> Self {
        Self::build(Params::default(), HashLimits::default())
    }
}

impl PasswordHasher {
    /// `params` with [`HashLimits::default`].
    pub fn new(params: HashParams) -> Result<Self, PasswordError> {
        Self::with_limits(params, HashLimits::default())
    }

    pub fn with_limits(params: HashParams, limits: HashLimits) -> Result<Self, PasswordError> {
        if limits.max_concurrent == 0 || limits.max_concurrent > Semaphore::MAX_PERMITS {
            return Err(PasswordError::Params);
        }
        let params = Params::new(
            params.memory_kib,
            params.iterations,
            params.parallelism,
            None,
        )
        .map_err(|_| PasswordError::Params)?;
        Ok(Self::build(params, limits))
    }

    fn build(params: Params, limits: HashLimits) -> Self {
        Self {
            params: Arc::new(params),
            permits: Arc::new(Semaphore::new(limits.max_concurrent)),
            max_concurrent: limits.max_concurrent,
            acquire_timeout: limits.acquire_timeout,
        }
    }

    /// Computation slots free right now (shared by all clones). For load metrics and for
    /// tests that need to know a computation is running.
    pub fn available_slots(&self) -> usize {
        self.permits.available_permits()
    }

    /// Computation slots in total ([`HashLimits::max_concurrent`]): how many computations a
    /// batch may usefully run at once.
    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }

    /// A new PHC string for `password`, with a fresh random salt.
    pub async fn hash(&self, password: &Password) -> Result<String, PasswordError> {
        let mut salt = [0u8; SALT_LEN];
        fill_random(&mut salt).map_err(|_| PasswordError::Random)?;
        let salt = SaltString::encode_b64(&salt).map_err(|_| PasswordError::Hash)?;
        self.argon2(password, move |argon2, pwd| {
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
        self.argon2(password, move |argon2, pwd| {
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
    /// password (06 §3.6). Goes through the same concurrency limit as real work.
    ///
    /// Known gap: [`verify`](Self::verify) costs what the *stored* hash's parameters cost.
    /// After a parameter change, accounts whose hash is not yet upgraded (no login since)
    /// verify faster or slower than this dummy, so timing can tell them apart from unknown
    /// usernames until their next successful login rehashes them. Accepted: it reveals only
    /// "exists, has not logged in since the upgrade", and parameter changes are rare.
    pub async fn verify_dummy(&self, password: &Password) -> Result<(), PasswordError> {
        let salt = SaltString::encode_b64(&DUMMY_SALT).map_err(|_| PasswordError::Hash)?;
        self.argon2(password, move |argon2, pwd| {
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

    /// Runs `op` with an Argon2id instance and a zeroizing copy of the password.
    async fn argon2<T, F>(&self, password: &Password, op: F) -> Result<T, PasswordError>
    where
        T: Send + 'static,
        F: FnOnce(&Argon2<'static>, &[u8]) -> Result<T, PasswordError> + Send + 'static,
    {
        let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, (*self.params).clone());
        let password = password.clone();
        self.limited(move || op(&argon2, password.expose().as_bytes()))
            .await
    }

    /// Runs `work` on the blocking pool once a slot is free. The slot is held by the blocking
    /// task itself, so it is released when the computation ends, even if the caller's future
    /// was dropped meanwhile.
    async fn limited<T, W>(&self, work: W) -> Result<T, PasswordError>
    where
        T: Send + 'static,
        W: FnOnce() -> Result<T, PasswordError> + Send + 'static,
    {
        let acquire = Arc::clone(&self.permits).acquire_owned();
        let permit = tokio::time::timeout(self.acquire_timeout, acquire)
            .await
            .map_err(|_| PasswordError::Busy)?
            .map_err(|_| PasswordError::Aborted)?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            work()
        })
        .await
        .map_err(|_| PasswordError::Aborted)?
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    const WORK: Duration = Duration::from_millis(50);

    fn hasher(max_concurrent: usize, acquire_timeout: Duration) -> PasswordHasher {
        let limits = HashLimits {
            max_concurrent,
            acquire_timeout,
        };
        PasswordHasher::with_limits(HashParams::DEFAULT, limits).unwrap()
    }

    #[test]
    fn default_hasher_uses_the_documented_params() {
        let documented = HashParams::DEFAULT;
        let params = PasswordHasher::default().params;

        assert_eq!(params.m_cost(), documented.memory_kib);
        assert_eq!(params.t_cost(), documented.iterations);
        assert_eq!(params.p_cost(), documented.parallelism);
    }

    #[test]
    fn default_limits_allow_at_least_two_computations() {
        let limits = HashLimits::default();

        assert!(limits.max_concurrent >= MIN_DEFAULT_CONCURRENCY);
        assert_eq!(limits.acquire_timeout, DEFAULT_ACQUIRE_TIMEOUT);
    }

    #[test]
    fn zero_concurrency_is_rejected() {
        let limits = HashLimits {
            max_concurrent: 0,
            acquire_timeout: WORK,
        };

        let result = PasswordHasher::with_limits(HashParams::DEFAULT, limits);

        assert!(matches!(result, Err(PasswordError::Params)));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn permits_bound_concurrent_computations() {
        const LIMIT: usize = 2;
        let hasher = hasher(LIMIT, Duration::from_secs(30));
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        let jobs = (0..12).map(|_| {
            let (hasher, running, peak) = (hasher.clone(), running.clone(), peak.clone());
            tokio::spawn(async move {
                hasher
                    .limited(move || {
                        let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(now, Ordering::SeqCst);
                        std::thread::sleep(WORK / 5);
                        running.fetch_sub(1, Ordering::SeqCst);
                        Ok(())
                    })
                    .await
            })
        });
        for job in jobs.collect::<Vec<_>>() {
            job.await.unwrap().unwrap();
        }

        assert_eq!(peak.load(Ordering::SeqCst), LIMIT);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn busy_when_no_slot_frees_up_within_the_timeout() {
        let hasher = hasher(1, WORK / 5);
        let holder = hasher.clone();
        let hold = tokio::spawn(async move {
            holder
                .limited(|| {
                    std::thread::sleep(WORK * 4);
                    Ok(())
                })
                .await
        });
        tokio::time::sleep(WORK / 5).await;

        let result = hasher.verify_dummy(&Password::new("x")).await;

        assert!(matches!(result, Err(PasswordError::Busy)), "{result:?}");
        hold.await.unwrap().unwrap();
        assert!(hasher.verify_dummy(&Password::new("x")).await.is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn slot_is_released_when_a_cancelled_computation_ends() {
        let hasher = hasher(1, Duration::from_secs(30));
        let finished = Arc::new(AtomicUsize::new(0));
        let done = finished.clone();

        // The caller gives up while the computation is running.
        let cancelled = tokio::time::timeout(
            WORK / 5,
            hasher.limited(move || {
                std::thread::sleep(WORK);
                done.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
        )
        .await;
        assert!(cancelled.is_err());
        assert_eq!(hasher.permits.available_permits(), 0, "still computing");

        tokio::time::sleep(WORK * 2).await;
        assert_eq!(finished.load(Ordering::SeqCst), 1);
        assert_eq!(hasher.permits.available_permits(), 1);
    }
}
