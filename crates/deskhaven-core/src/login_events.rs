//! Never a password, token or request body. The attempted username and device id are what the
//! client sent, so they are stored cleaned like other self-reported text.

use std::net::IpAddr;

use deskhaven_db::entity::{login_event, user};
use deskhaven_db::time::{self, TimestampMs};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter};

pub use deskhaven_db::entity::login_event::{Client as LoginClient, Method as LoginMethod};

use crate::clock::SharedClock;
use crate::text;

/// Real usernames are at most 64; the rest is room for what people paste into the field.
pub const MAX_USERNAME_ATTEMPTED_CHARS: usize = 128;

pub const MAX_RUSTDESK_ID_CHARS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginResult {
    Success,
    /// One label, so the log does not enumerate accounts either.
    InvalidCredentials,
    Disabled,
    Unverified,
    /// The next ones are refused unrecorded.
    Locked,
    SecondFactorRequired,
    SecondFactorSetupRequired,
    InvalidCode,
    InvalidChallenge,
    OidcDenied,
    OidcUnavailable,
    OidcInvalidToken,
    NoLinkedAccount,
    ProvisioningFailed,
}

impl LoginResult {
    /// Every other outcome, including ones added later, counts as a failure.
    pub const NOT_FAILURES: [Self; 3] = [
        Self::Success,
        Self::SecondFactorRequired,
        Self::SecondFactorSetupRequired,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::InvalidCredentials => "invalid_credentials",
            Self::Disabled => "disabled",
            Self::Unverified => "unverified",
            Self::Locked => "locked",
            Self::SecondFactorRequired => "tfa_required",
            Self::SecondFactorSetupRequired => "tfa_setup_required",
            Self::InvalidCode => "invalid_code",
            Self::InvalidChallenge => "invalid_challenge",
            Self::OidcDenied => "oidc_denied",
            Self::OidcUnavailable => "oidc_unavailable",
            Self::OidcInvalidToken => "oidc_invalid_token",
            Self::NoLinkedAccount => "no_linked_account",
            Self::ProvisioningFailed => "provisioning_failed",
        }
    }
}

impl From<crate::oidc::Failure> for LoginResult {
    fn from(failure: crate::oidc::Failure) -> Self {
        use crate::oidc::Failure;
        match failure {
            Failure::Denied => Self::OidcDenied,
            Failure::Unavailable => Self::OidcUnavailable,
            Failure::InvalidToken => Self::OidcInvalidToken,
            Failure::NoLinkedAccount => Self::NoLinkedAccount,
            Failure::Disabled => Self::Disabled,
            Failure::Unverified => Self::Unverified,
            Failure::ProvisioningFailed => Self::ProvisioningFailed,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewLoginEvent<'a> {
    pub user_id: Option<i64>,
    pub username_attempted: &'a str,
    pub method: LoginMethod,
    pub result: LoginResult,
    pub client: LoginClient,
    pub rustdesk_id: Option<&'a str>,
    pub ip: Option<IpAddr>,
}

#[derive(Clone)]
pub struct LoginEventLog {
    db: DatabaseConnection,
    clock: SharedClock,
}

impl LoginEventLog {
    pub fn new(db: DatabaseConnection, clock: SharedClock) -> Self {
        Self { db, clock }
    }

    pub async fn record(&self, event: &NewLoginEvent<'_>) -> Result<(), DbErr> {
        let now = self.clock.now_ms();
        let created_at = time::from_millis(now)
            .ok_or_else(|| DbErr::Custom(format!("timestamp {now} out of range")))?;
        let row = login_event::ActiveModel {
            user_id: Set(event.user_id),
            username_attempted: Set(text::clean(
                event.username_attempted,
                MAX_USERNAME_ATTEMPTED_CHARS,
                false,
            )),
            method: Set(event.method),
            result: Set(event.result.as_str().to_owned()),
            client: Set(event.client),
            rustdesk_id: Set(event
                .rustdesk_id
                .map(|id| text::clean(id, MAX_RUSTDESK_ID_CHARS, false))),
            ip: Set(event.ip.map(|ip| ip.to_string())),
            created_at: Set(created_at),
            ..Default::default()
        };
        row.insert(&self.db).await?;
        match (event.result, event.user_id) {
            (LoginResult::Success, Some(user_id)) => {
                self.stamp_last_login(user_id, created_at).await
            }
            _ => Ok(()),
        }
    }

    /// Every completed login ends in a `Success` event, so this is the one place that sees them all.
    async fn stamp_last_login(&self, user_id: i64, at: TimestampMs) -> Result<(), DbErr> {
        user::Entity::update_many()
            .col_expr(user::Column::LastLoginAt, Expr::value(at))
            .filter(user::Column::Id.eq(user_id))
            .exec(&self.db)
            .await
            .map(drop)
    }
}
