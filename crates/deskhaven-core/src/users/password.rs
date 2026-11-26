//! A semaphore bounds concurrent computations, so a login flood cannot fill tokio's blocking pool
//! and take the process down; callers that cannot get a slot in time get [`PasswordError::Busy`].

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use argon2::password_hash::{PasswordHash, PasswordHasher as _, PasswordVerifier as _, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

use crate::crypto::fill_random;

const SALT_LEN: usize = 16;
/// Its value is irrelevant: the output is discarded.
const DUMMY_SALT: [u8; SALT_LEN] = [0x5a; SALT_LEN];
const MIN_DEFAULT_CONCURRENCY: usize = 2;
const DEFAULT_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

/// Zeroed on drop; `Debug` never shows it and there is no `Display`. No Unicode normalization:
/// clients send what the input method produces consistently.
#[derive(Clone)]
pub struct Password(Zeroizing<String>);

impl Password {
    pub fn new(value: impl Into<String>) -> Self {
        Self(Zeroizing::new(value.into()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn char_len(&self) -> usize {
        self.0.chars().count()
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(<redacted>)")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HashParams {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl HashParams {
    /// The OWASP baseline. Changing it makes every older hash need a rehash at its next login.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HashLimits {
    pub max_concurrent: usize,
    pub acquire_timeout: Duration,
}

impl Default for HashLimits {
    fn default() -> Self {
        let cpus = std::thread::available_parallelism().map_or(1, usize::from);
        Self {
            max_concurrent: cpus.max(MIN_DEFAULT_CONCURRENCY),
            acquire_timeout: DEFAULT_ACQUIRE_TIMEOUT,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PasswordError {
    #[error("invalid argon2 parameters or limits")]
    Params,

    #[error("stored password hash is malformed")]
    MalformedHash,

    #[error("password hashing failed")]
    Hash,

    #[error("the operating system random number generator failed")]
    Random,

    #[error("too many concurrent password computations")]
    Busy,

    #[error("password hashing task did not complete")]
    Aborted,
}

/// Clones share the concurrency limit, so a process should build one and hand out clones.
#[derive(Clone, Debug)]
pub struct PasswordHasher {
    params: Arc<Params>,
    permits: Arc<Semaphore>,
    max_concurrent: usize,
    acquire_timeout: Duration,
}

impl Default for PasswordHasher {
    /// argon2's own default is the same OWASP baseline; a unit test keeps the two equal, so this needs
    /// no fallible constructor.
    fn default() -> Self {
        Self::build(Params::default(), HashLimits::default())
    }
}

impl PasswordHasher {
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

    pub fn available_slots(&self) -> usize {
        self.permits.available_permits()
    }

    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }

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

    /// A malformed `phc` is an error, not a mismatch.
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

    /// Makes unknown or non-local accounts fail as slowly as a wrong password. Known gap: after a
    /// parameter change, accounts not yet rehashed verify at a different cost until their next login.
    /// Accepted: it reveals only "exists, has not logged in since", and parameter changes are rare.
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

    /// The slot is held by the blocking task itself, so it is released when the computation ends, even
    /// if the caller's future was dropped.
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

    use tokio::sync::oneshot;

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

    fn held() -> (
        impl FnOnce() -> Result<(), PasswordError> + Send + 'static,
        oneshot::Receiver<()>,
        std::sync::mpsc::Sender<()>,
    ) {
        let (started_tx, started) = oneshot::channel();
        let (release, released) = std::sync::mpsc::channel::<()>();
        let work = move || {
            let _ = started_tx.send(());
            let _ = released.recv();
            Ok(())
        };
        (work, started, release)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn busy_when_no_slot_frees_up_within_the_timeout() {
        let hasher = hasher(1, WORK / 5);
        let (work, started, release) = held();
        let holder = hasher.clone();
        let hold = tokio::spawn(async move { holder.limited(work).await });
        started.await.unwrap();

        let result = hasher.verify_dummy(&Password::new("x")).await;

        assert!(matches!(result, Err(PasswordError::Busy)), "{result:?}");
        release.send(()).unwrap();
        hold.await.unwrap().unwrap();
        assert!(hasher.verify_dummy(&Password::new("x")).await.is_ok());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn slot_is_released_when_a_cancelled_computation_ends() {
        let hasher = hasher(1, Duration::from_secs(30));
        let (work, started, release) = held();

        // The caller gives up while the computation is running.
        tokio::select! {
            _ = hasher.limited(work) => panic!("ended before it was released"),
            started = started => started.unwrap(),
        }
        assert_eq!(hasher.permits.available_permits(), 0, "still computing");

        release.send(()).unwrap();
        let freed = tokio::time::timeout(Duration::from_secs(5), hasher.permits.acquire()).await;
        assert!(
            freed.is_ok(),
            "the slot comes back when the computation ends"
        );
    }
}
