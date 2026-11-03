//! The login challenge: issued after the password (step 1), claimed and consumed by the
//! second step. See the module docs of `tfa` for the columns.
//!
//! The secret is a session-format token (256 random bits); only its SHA-256 is stored.
//! The `context` binds the challenge to what step 1 established, so a stolen secret is
//! useless elsewhere: where the login runs ([`LoginBinding`]: the client device `id` +
//! `uuid`, or the web console), the lockout subject (whose name is the normalized username)
//! and the password's credential stamp. A console challenge never verifies for a client
//! login and a client challenge never for the console.

use std::fmt;

use deskhaven_db::entity::auth_challenge::{self, Column, Kind};
use deskhaven_db::time;
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::{Expr, ExprTrait};
use sea_orm::{ActiveModelTrait, ColumnTrait, Condition, EntityTrait, QueryFilter};
use serde_json::{Value, json};

use super::{CHALLENGE_TTL, ChallengeError, Inner, MAX_CHALLENGE_ATTEMPTS, TfaError, TfaService};
use crate::auth::limiter::PendingSubject;
use crate::jobs::{Doomed, Plain, Purge};
use crate::sessions::{ClientDevice, SessionToken, TokenHash};
use crate::users::{CredentialStamp, username_norm};

/// `context` marker of a console challenge (instead of a device).
const CONSOLE_KEY: &str = "console";

/// The `secret` handed to the client with the challenge. Exposed once; `Debug` never
/// shows it.
pub struct ChallengeSecret(SessionToken);

impl ChallengeSecret {
    pub fn expose(&self) -> &str {
        self.0.expose()
    }
}

impl fmt::Debug for ChallengeSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ChallengeSecret(<redacted>)")
    }
}

/// Where a two-step login runs: both steps must name the same place.
#[derive(Clone, Copy, Debug)]
pub enum LoginBinding<'a> {
    /// A RustDesk client login, from this device.
    Device(&'a ClientDevice),
    /// A web console login (the session becomes a cookie, not bound to a device).
    Console,
}

/// Step 1 passed the password and the user needs a second factor.
#[derive(Debug)]
pub struct PendingLogin<'a> {
    pub user_id: i64,
    pub stamp: &'a CredentialStamp,
    /// Where the login runs; step 2 must run there too.
    pub binding: LoginBinding<'a>,
    /// From `LoginThrottle::on_password_ok_pending_2fa`.
    pub lockout: &'a PendingSubject,
}

/// The second-step request. `Debug` shows neither the secret nor the code.
pub struct SecondStep<'a> {
    pub secret: &'a str,
    /// Must name the user of the challenge (case-insensitively).
    pub username: &'a str,
    pub binding: LoginBinding<'a>,
    /// A TOTP code or a recovery code. Never logged.
    pub code: &'a str,
}

impl fmt::Debug for SecondStep<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecondStep")
            .field("username", &self.username)
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

/// Where the challenge was issued, read back from `context`.
#[derive(Debug, PartialEq, Eq)]
enum BoundBinding {
    Device { rustdesk_id: String, uuid: String },
    Console,
}

impl BoundBinding {
    fn matches(&self, presented: LoginBinding<'_>) -> bool {
        match (self, presented) {
            (Self::Device { rustdesk_id, uuid }, LoginBinding::Device(device)) => {
                *rustdesk_id == device.rustdesk_id && *uuid == device.uuid
            }
            (Self::Console, LoginBinding::Console) => true,
            _ => false,
        }
    }
}

/// What the challenge carries from step 1, read back from `context`.
pub(super) struct Bound {
    binding: BoundBinding,
    pub(super) lockout: PendingSubject,
    pub(super) stamp: CredentialStamp,
}

impl TfaService {
    /// Stores a TOTP challenge for `login`, valid for [`CHALLENGE_TTL`] and
    /// [`MAX_CHALLENGE_ATTEMPTS`] codes.
    pub async fn issue_challenge(
        &self,
        login: &PendingLogin<'_>,
    ) -> Result<ChallengeSecret, TfaError> {
        let inner = &self.inner;
        let secret = SessionToken::generate()?;
        let now = inner.clock.now_ms();
        let ttl = i64::try_from(CHALLENGE_TTL.as_millis()).unwrap_or(i64::MAX);
        let (name, source) = login.lockout.parts();
        let mut context = json!({
            "lockout_name": name,
            "lockout_source": source,
            "password_changed_at": login.stamp.to_stored(),
        });
        if let Value::Object(fields) = &mut context {
            match login.binding {
                LoginBinding::Device(device) => {
                    fields.insert("rustdesk_id".to_owned(), json!(device.rustdesk_id));
                    fields.insert("uuid".to_owned(), json!(device.uuid));
                }
                LoginBinding::Console => {
                    fields.insert(CONSOLE_KEY.to_owned(), Value::Bool(true));
                }
            }
        }
        auth_challenge::ActiveModel {
            secret_hash: Set(secret.hash().as_str().to_owned()),
            user_id: Set(login.user_id),
            kind: Set(Kind::Totp),
            code_hash: Set(None),
            attempts: Set(0),
            context: Set(context.to_string()),
            created_at: Set(timestamp(now)?),
            expires_at: Set(timestamp(now.saturating_add(ttl))?),
            consumed_at: Set(None),
            ..Default::default()
        }
        .insert(&inner.db)
        .await?;
        Ok(ChallengeSecret(secret))
    }

    /// Deletes expired and consumed challenges in batches; returns how many. The expiry job
    /// (M2-20) deletes [`finished_challenges`] with its own batch settings.
    pub async fn cleanup_challenges(&self) -> Result<u64, TfaError> {
        let doomed = finished_challenges(self.inner.now());
        let deleted = Purge::default()
            .run(&self.inner.db, &doomed, &mut Plain)
            .await?;
        Ok(deleted)
    }
}

/// Challenges expired at `now` or consumed: what cleanup deletes.
pub(crate) fn finished_challenges(now: time::TimestampMs) -> Doomed<auth_challenge::Entity> {
    Doomed {
        key: Column::Id,
        order: Column::Id,
        cond: Condition::any()
            .add(Column::ExpiresAt.lte(now))
            .add(Column::ConsumedAt.is_not_null()),
    }
}

impl Inner {
    /// Spends one attempt of the live challenge `secret` names, and checks it was issued
    /// for `step`'s username and binding. Every failure is [`ChallengeError::Invalid`]; a
    /// bad binding still spends the attempt (the secret was presented).
    pub(super) async fn claim(
        &self,
        step: &SecondStep<'_>,
    ) -> Result<(auth_challenge::Model, Bound), ChallengeError> {
        let hash = TokenHash::of_presented(step.secret).ok_or(ChallengeError::Invalid)?;
        let claimed = auth_challenge::Entity::update_many()
            .col_expr(Column::Attempts, Expr::col(Column::Attempts).add(1))
            .filter(Column::SecretHash.eq(hash.as_str()))
            .filter(Column::ConsumedAt.is_null())
            .filter(Column::ExpiresAt.gt(self.now()))
            .filter(Column::Attempts.lt(MAX_CHALLENGE_ATTEMPTS))
            .exec_with_returning(&self.db)
            .await?;
        let row = claimed.into_iter().next().ok_or(ChallengeError::Invalid)?;
        let bound = parse_context(&row)?;
        let same_user = username_norm(step.username).as_deref() == Some(bound.lockout.parts().0);
        if !(same_user && bound.binding.matches(step.binding)) {
            return Err(ChallengeError::Invalid);
        }
        Ok((row, bound))
    }
}

fn parse_context(row: &auth_challenge::Model) -> Result<Bound, ChallengeError> {
    let corrupt = || {
        tracing::error!(
            challenge_id = row.id,
            "login challenge context is unreadable"
        );
        ChallengeError::Internal(TfaError::Corrupt)
    };
    let value: Value = serde_json::from_str(&row.context).map_err(|_| corrupt())?;
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    let (Some(name), Some(source)) = (text("lockout_name"), text("lockout_source")) else {
        return Err(corrupt());
    };
    let binding = match (text("rustdesk_id"), text("uuid"), value.get(CONSOLE_KEY)) {
        (Some(rustdesk_id), Some(uuid), None) => BoundBinding::Device { rustdesk_id, uuid },
        (None, None, Some(Value::Bool(true))) => BoundBinding::Console,
        _ => return Err(corrupt()),
    };
    let lockout = PendingSubject::from_parts(&name, &source).ok_or_else(corrupt)?;
    let stamp = match value.get("password_changed_at") {
        Some(Value::Null) => CredentialStamp::from_stored(None),
        Some(ms) => CredentialStamp::from_stored(Some(ms.as_i64().ok_or_else(corrupt)?)),
        None => return Err(corrupt()),
    };
    Ok(Bound {
        binding,
        lockout,
        stamp,
    })
}

fn timestamp(ms: i64) -> Result<time::TimestampMs, TfaError> {
    time::from_millis(ms).ok_or_else(|| {
        TfaError::Db(sea_orm::DbErr::Custom(format!(
            "timestamp {ms} out of range"
        )))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> ClientDevice {
        ClientDevice {
            rustdesk_id: "123456789".to_owned(),
            uuid: "dXVpZA==".to_owned(),
            info: None,
        }
    }

    #[test]
    fn debug_never_shows_the_secret_or_the_code() {
        let device = device();
        let step = SecondStep {
            secret: "s3cret-challenge-value",
            username: "alice",
            binding: LoginBinding::Device(&device),
            code: "987654",
        };

        let shown = format!("{step:?}");

        assert!(shown.contains("alice"), "{shown}");
        assert!(
            !shown.contains("s3cret-challenge-value") && !shown.contains("987654"),
            "{shown}"
        );
    }

    #[test]
    fn a_binding_matches_only_its_own_kind_and_device() {
        let device = device();
        let other = ClientDevice {
            uuid: "other".to_owned(),
            ..device.clone()
        };
        let bound_device = BoundBinding::Device {
            rustdesk_id: device.rustdesk_id.clone(),
            uuid: device.uuid.clone(),
        };

        assert!(bound_device.matches(LoginBinding::Device(&device)));
        assert!(!bound_device.matches(LoginBinding::Device(&other)));
        assert!(!bound_device.matches(LoginBinding::Console));
        assert!(BoundBinding::Console.matches(LoginBinding::Console));
        assert!(!BoundBinding::Console.matches(LoginBinding::Device(&device)));
    }
}
