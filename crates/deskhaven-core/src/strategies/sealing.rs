//! Secret catalog options at rest (06 §5): each value of an option the catalog marks secret
//! ([`catalog::is_secret`]) is sealed on its own inside the stored JSON object, under the
//! context `strategies.options:<strategy id>:<key>`, so a database copy holds no proxy or
//! address book password in clear and a ciphertext moved to another strategy or option no
//! longer opens. An empty value (the client deletes the option) is no secret and stays as
//! it is. Values are opened whenever a row is read: the heartbeat delivers them in clear,
//! and the admin API withholds them (`secret_options`).
//!
//! Rows written before sealing hold their secrets in clear. Such a value is not a sealed
//! string, so it is served as stored, and [`StrategyService::seal_plaintext_secrets`] (run
//! at startup) seals it in place; every write seals anyway. A sealed value that does not
//! open (another row or key, a master key not held, damage) makes the strategy unreadable
//! (`options_corrupt`), like damaged JSON.

use deskhaven_db::begin_write;
use deskhaven_db::entity::strategy;
use sea_orm::ActiveValue::Set;
use sea_orm::{ActiveModelTrait, EntityTrait, QuerySelect};

use super::options::{self, Options};
use super::{StrategyError, StrategyService, catalog};
use crate::crypto::{CryptoError, SecretBox};

/// A stored row's options with their secrets opened.
pub(super) struct Opened {
    pub options: Options,
    /// Some secret was stored in clear (a row from before sealing).
    pub has_plaintext: bool,
}

/// Sealing context of option `key` of strategy `id`.
fn context(id: i64, key: &str) -> String {
    format!("strategies.options:{id}:{key}")
}

fn is_sealed_at_rest(key: &str, value: &str) -> bool {
    !value.is_empty() && catalog::is_secret(key)
}

/// The stored form of strategy `id`'s (validated) `options`, secrets sealed.
pub(super) fn encode(
    crypto: &SecretBox,
    id: i64,
    options: &Options,
) -> Result<String, StrategyError> {
    let stored = options
        .iter()
        .map(|(key, value)| {
            let stored = match is_sealed_at_rest(key, value) {
                true => crypto.seal(&context(id, key), value.as_bytes())?,
                false => value.clone(),
            };
            Ok((key.clone(), stored))
        })
        .collect::<Result<Options, CryptoError>>()
        .map_err(StrategyError::Crypto)?;
    options::encode(&stored)
}

/// `None` when `stored` is not a JSON object of strings or a sealed value does not open.
pub(super) fn decode(crypto: &SecretBox, id: i64, stored: &str) -> Option<Opened> {
    let mut has_plaintext = false;
    let mut opened = Options::new();
    for (key, value) in options::decode(stored)? {
        let value = match is_sealed_at_rest(&key, &value) {
            true => match open(crypto, id, &key, &value)? {
                Some(plain) => plain,
                None => {
                    has_plaintext = true;
                    value
                }
            },
            false => value,
        };
        opened.insert(key, value);
    }
    Some(Opened {
        options: opened,
        has_plaintext,
    })
}

/// `Some(None)`: `value` is not a sealed string (stored in clear). `None`: it is sealed but
/// does not open.
fn open(crypto: &SecretBox, id: i64, key: &str, value: &str) -> Option<Option<String>> {
    match crypto.open(&context(id, key), value) {
        Ok(plain) => String::from_utf8(plain.to_vec()).ok().map(Some),
        Err(CryptoError::Malformed) => Some(None),
        Err(_) => None,
    }
}

impl StrategyService {
    /// Seals the secret options stored in clear (rows from before sealing) in place; returns
    /// how many strategies were rewritten. Values do not change, so neither do effective
    /// strategies or `updated_at`. Strategies are few and written by administrators: one
    /// transaction over all of them.
    pub async fn seal_plaintext_secrets(&self) -> Result<usize, StrategyError> {
        self.detached(move |inner| async move {
            let tx = begin_write(&inner.db).await?;
            let rows = strategy::Entity::find().lock_exclusive().all(&tx).await?;
            let mut sealed = 0;
            for row in rows {
                let Some(opened) = decode(&inner.crypto, row.id, &row.options) else {
                    continue;
                };
                if !opened.has_plaintext {
                    continue;
                }
                let stored = encode(&inner.crypto, row.id, &opened.options)?;
                let mut changes: strategy::ActiveModel = row.into();
                changes.options = Set(stored);
                changes.update(&tx).await?;
                sealed += 1;
            }
            tx.commit().await?;
            Ok(sealed)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::MasterKey;

    fn crypto() -> SecretBox {
        SecretBox::new(&MasterKey::from_bytes(&[3; 32]))
    }

    fn one(key: &str, value: &str) -> Options {
        Options::from([(key.to_owned(), value.to_owned())])
    }

    #[test]
    fn secrets_round_trip_sealed_and_other_values_stay_in_clear() {
        let crypto = crypto();
        let options: Options = [("proxy-password", "pw"), ("k", "v"), ("ice-servers", "")]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();

        let stored = encode(&crypto, 4, &options).unwrap();
        let opened = decode(&crypto, 4, &stored).unwrap();

        assert!(!stored.contains("\"pw\""), "{stored}");
        assert!(stored.contains("\"k\":\"v\"") && stored.contains("\"ice-servers\":\"\""));
        assert_eq!(opened.options, options);
        assert!(!opened.has_plaintext);
        assert!(decode(&crypto, 5, &stored).is_none(), "bound to the row");
    }

    #[test]
    fn a_clear_secret_is_served_and_flagged_and_other_json_is_refused() {
        let crypto = crypto();
        let stored = options::encode(&one("proxy-password", "v1:not-sealed")).unwrap();

        let opened = decode(&crypto, 1, &stored).unwrap();

        assert_eq!(opened.options, one("proxy-password", "v1:not-sealed"));
        assert!(opened.has_plaintext);
        assert!(decode(&crypto, 1, "[1]").is_none());
    }
}
