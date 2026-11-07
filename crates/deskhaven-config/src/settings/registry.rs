//! The settings registry: every runtime setting, declared exactly once
//! (05-config §3.6). Defaults, validation, the exported schema and the read
//! APIs all derive from these declarations.

use super::def::{AnySetting, Setting, settings, strict};
use super::enums::{AcceptPresets, AclAction, LinkEmailMaxRole, Registration, SmtpTls, TfaEnforce};
use super::kind::{SettingType, max_items, range};
use super::text::{TextRule, max_len};
use super::value::HostPort;
use crate::{Duration, Secret};

const fn secs(n: u64) -> Duration {
    Duration::from_std(std::time::Duration::from_secs(n))
}
const fn minutes(n: u64) -> Duration {
    secs(n * 60)
}
const fn hours(n: u64) -> Duration {
    minutes(n * 60)
}
const fn days(n: u64) -> Duration {
    hours(n * 24)
}

/// Upper bound for audit retention; `0` keeps records forever.
const MAX_RETENTION: Duration = days(3650);
/// RFC 5321 limits: 255-octet domain, 254-octet path.
const SMTP_TEXT: TextRule = max_len(254);
const MAX_RELAYS: usize = 32;

/// The login lockout's per-username global row locks after this many times
/// `auth.lockout.max_failures` (deskhaven-core `auth::limiter`). The cross-key rule
/// `auth.lockout.window <= LOCKOUT_GLOBAL_FACTOR × auth.lockout.duration` keeps a single
/// source, which is locked out for at least `duration` after each burst, from ever reaching
/// that threshold within one window on its own.
pub const LOCKOUT_GLOBAL_FACTOR: u32 = 4;

settings! {
    /// Absolute lifetime of client and console sessions.
    AUTH_SESSION_TTL: Duration = "auth.session_ttl" {
        default: days(30),
        rule: range(minutes(5), days(365)),
    }
    /// Sliding idle timeout of client and console sessions.
    AUTH_SESSION_IDLE_TTL: Duration = "auth.session_idle_ttl" {
        default: days(7),
        rule: range(minutes(5), days(365)),
    }
    /// Most usable client and console sessions one user holds; signing in once more ends
    /// the user's least recently used session. Every RustDesk client signed in to an
    /// account holds one session, so an account shared by more devices than this logs the
    /// least recently used device out.
    AUTH_MAX_SESSIONS_PER_USER: u32 = "auth.max_sessions_per_user" {
        default: 1000,
        rule: range(1, 10_000),
    }
    /// Minimum password length (the maximum is fixed at 256).
    AUTH_PASSWORD_MIN_LENGTH: u32 = "auth.password.min_length" {
        default: 10,
        rule: range(8, 256),
    }
    /// Failed logins per account within the window before it is locked.
    AUTH_LOCKOUT_MAX_FAILURES: u32 = "auth.lockout.max_failures" {
        default: 5,
        rule: range(1, 100),
    }
    /// Window in which failed logins are counted.
    AUTH_LOCKOUT_WINDOW: Duration = "auth.lockout.window" {
        default: minutes(15),
        rule: range(minutes(1), hours(24)),
    }
    /// Base lockout duration (exponential backoff is capped at 24h).
    AUTH_LOCKOUT_DURATION: Duration = "auth.lockout.duration" {
        default: minutes(15),
        rule: range(minutes(1), hours(24)),
    }
    /// Requests per minute per IP on login-type endpoints.
    AUTH_IP_RATE_LIMIT_PER_MINUTE: u32 = "auth.ip_rate_limit.per_minute" {
        default: 20,
        rule: range(1, 10_000),
    }
    /// Self-service registration mode.
    AUTH_REGISTRATION: Registration = "auth.registration" {
        default: Registration::Disabled,
        on_bad_row: strict(Registration::Disabled),
    }
    /// Which users must enroll a second factor for password sign-ins (OIDC sign-ins never ask
    /// for it). A covered user without one is refused by the RustDesk client until they enroll
    /// in the web console, which asks them to at their next console sign-in.
    AUTH_TFA_ENFORCE: TfaEnforce = "auth.tfa.enforce" {
        default: TfaEnforce::Off,
        // Stricter than `off` without forcing every user to enroll at once.
        on_bad_row: strict(TfaEnforce::Admins),
    }
    /// Allow email one-time codes as a second factor (requires SMTP).
    AUTH_EMAIL_CODE_ENABLED: bool = "auth.email_code.enabled" {
        default: false,
    }
    /// Highest role an OIDC login may reach by linking an existing account by email (both
    /// sides verified, no TOTP, not LDAP); `off` never links by email. `admin` and `owner`
    /// let the identity provider take those accounts over by email: only with an IdP whose
    /// users cannot change their own email address.
    AUTH_OIDC_LINK_EMAIL_MAX_ROLE: LinkEmailMaxRole = "auth.oidc.link_email_max_role" {
        default: LinkEmailMaxRole::User,
        on_bad_row: strict(LinkEmailMaxRole::Off),
    }
    /// SMTP server host name; empty = SMTP not configured.
    SMTP_HOST: String = "smtp.host" {
        default: "",
        rule: SMTP_TEXT.host(),
    }
    /// SMTP server port.
    SMTP_PORT: u32 = "smtp.port" {
        default: 587,
        rule: range(1, 65_535),
    }
    /// SMTP transport security.
    SMTP_TLS: SmtpTls = "smtp.tls" {
        default: SmtpTls::Starttls,
    }
    /// SMTP user name; empty = no authentication.
    SMTP_USERNAME: String = "smtp.username" {
        default: "",
        rule: SMTP_TEXT,
    }
    /// SMTP password.
    SMTP_PASSWORD: Secret = "smtp.password" {
        default: (),
    }
    /// Sender address of outgoing mail.
    SMTP_FROM: String = "smtp.from" {
        default: "",
        rule: SMTP_TEXT.email(),
    }
    /// Device endpoints check that id and uuid match the registered device.
    DEVICES_VERIFY_UUID: bool = "devices.verify_uuid" {
        default: true,
        on_bad_row: strict(true),
    }
    /// Accept `rustdesk --deploy` registrations. Refusing devices that were not deployed
    /// (`NOT_DEPLOYED`) is not in effect yet.
    DEVICES_REQUIRE_DEPLOY: bool = "devices.require_deploy" {
        default: false,
    }
    /// Interval for batch-writing device last-seen times.
    DEVICES_LAST_SEEN_FLUSH: Duration = "devices.last_seen_flush" {
        default: secs(60),
        rule: range(secs(5), hours(1)),
    }
    /// Apply the `preset-*` fields of a device's first sysinfo: `row` = owner, device group,
    /// note and strategy; `all` = also the address book entry (written as the owner). Presets
    /// let a device choose its own placement: enable only for controlled custom-client
    /// rollouts. Never applied while `devices.verify_uuid` is off.
    DEVICES_ACCEPT_PRESETS: AcceptPresets = "devices.accept_presets" {
        default: AcceptPresets::Off,
        on_bad_row: strict(AcceptPresets::Off),
    }
    /// Maximum peers per address book returned to clients (`max_peer_one_ab`).
    ADDRESSBOOK_MAX_PEERS: u32 = "addressbook.max_peers" {
        default: 1000,
        rule: range(1, 100_000),
    }
    /// Retention of connection audit records; 0 = forever.
    AUDIT_RETENTION_CONNECTION: Duration = "audit.retention.connection" {
        default: days(365),
        rule: range(days(1), MAX_RETENTION).or_zero(),
    }
    /// Retention of file-transfer audit records; 0 = forever.
    AUDIT_RETENTION_FILE: Duration = "audit.retention.file" {
        default: days(365),
        rule: range(days(1), MAX_RETENTION).or_zero(),
    }
    /// Retention of alarm audit records; 0 = forever.
    AUDIT_RETENTION_ALARM: Duration = "audit.retention.alarm" {
        default: days(365),
        rule: range(days(1), MAX_RETENTION).or_zero(),
    }
    /// Retention of login audit records; 0 = forever.
    AUDIT_RETENTION_LOGIN: Duration = "audit.retention.login" {
        default: days(180),
        rule: range(days(1), MAX_RETENTION).or_zero(),
    }
    /// Retention of console audit records; 0 = forever.
    AUDIT_RETENTION_CONSOLE: Duration = "audit.retention.console" {
        default: days(730),
        rule: range(days(1), MAX_RETENTION).or_zero(),
    }
    /// Relay servers handed to clients by the signal server.
    SIGNAL_RELAY_SERVERS: Vec<HostPort> = "signal.relay_servers" {
        default: (),
        rule: max_items(MAX_RELAYS),
    }
    /// Force all connections through a relay.
    SIGNAL_ALWAYS_USE_RELAY: bool = "signal.always_use_relay" {
        default: false,
    }
    /// Refuse the signal KeyExchange of clients from before version 1 (one key for both
    /// directions); clients that skip the exchange are not affected.
    SIGNAL_REQUIRE_KX_V1: bool = "signal.require_kx_v1" {
        default: false,
    }
    /// Connection ACL decision when no rule matches (not in effect yet).
    SIGNAL_ACL_DEFAULT: AclAction = "signal.acl.default" {
        default: AclAction::Allow,
        on_bad_row: strict(AclAction::Deny),
    }
    /// Connection ACL decision when the control plane fails (fail closed; not in effect yet).
    SIGNAL_ACL_ON_ERROR: AclAction = "signal.acl.on_error" {
        default: AclAction::Deny,
        on_bad_row: strict(AclAction::Deny),
    }
}

/// Every declared setting, in declaration order.
pub fn all() -> &'static [&'static dyn AnySetting] {
    ALL
}

/// The setting declared under `key`.
pub fn lookup(key: &str) -> Option<&'static dyn AnySetting> {
    ALL.iter().copied().find(|s| s.key() == key)
}

/// Keys that existed once and were replaced: `(old key, what to configure instead)`. A pin
/// under an old key fails startup with this hint instead of "unknown setting"; a stored row
/// was converted by its migration.
const REPLACED: &[(&str, &str)] = &[(
    "auth.oidc.link_verified_email",
    "replaced by auth.oidc.link_email_max_role (true: \"user\", false: \"off\")",
)];

/// The hint for a replaced `key`, if it is one.
pub fn replaced(key: &str) -> Option<&'static str> {
    REPLACED
        .iter()
        .find(|(old, _)| *old == key)
        .map(|(_, hint)| *hint)
}
