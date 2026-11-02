//! Cross-key rules, checked on a whole candidate snapshot.

use serde_json::{Value as Json, json};

use super::def::AnySetting;
use super::registry::{
    AUTH_EMAIL_CODE_ENABLED, AUTH_LOCKOUT_DURATION, AUTH_LOCKOUT_WINDOW, AUTH_SESSION_IDLE_TTL,
    AUTH_SESSION_TTL, LOCKOUT_GLOBAL_FACTOR, SMTP_FROM, SMTP_HOST,
};
use super::snapshot::Snapshot;
use super::update::Problem;

struct Rule {
    /// The first key is the one a violation is reported against.
    keys: &'static [&'static dyn AnySetting],
    message: &'static str,
    holds: fn(&Snapshot) -> bool,
}

static RULES: &[Rule] = &[
    Rule {
        keys: &[&AUTH_EMAIL_CODE_ENABLED, &SMTP_HOST, &SMTP_FROM],
        message: "auth.email_code.enabled requires smtp.host and smtp.from to be set",
        holds: email_code_has_smtp,
    },
    Rule {
        keys: &[&AUTH_SESSION_IDLE_TTL, &AUTH_SESSION_TTL],
        message: "auth.session_idle_ttl must not exceed auth.session_ttl",
        holds: idle_ttl_within_session_ttl,
    },
    Rule {
        keys: &[&AUTH_LOCKOUT_WINDOW, &AUTH_LOCKOUT_DURATION],
        message: "auth.lockout.window must not exceed 4 × auth.lockout.duration (otherwise a \
                  single source can trip the per-username global lockout)",
        holds: lockout_window_within_global_reach,
    },
];

fn email_code_has_smtp(s: &Snapshot) -> bool {
    !s.get(&AUTH_EMAIL_CODE_ENABLED)
        || !(s.get(&SMTP_HOST).is_empty() || s.get(&SMTP_FROM).is_empty())
}

fn idle_ttl_within_session_ttl(s: &Snapshot) -> bool {
    s.get(&AUTH_SESSION_IDLE_TTL) <= s.get(&AUTH_SESSION_TTL)
}

fn lockout_window_within_global_reach(s: &Snapshot) -> bool {
    let duration = s.get(&AUTH_LOCKOUT_DURATION).as_std();
    s.get(&AUTH_LOCKOUT_WINDOW).as_std() <= duration.saturating_mul(LOCKOUT_GLOBAL_FACTOR)
}

/// Rules that do not hold on `snapshot`; with `changed`, only rules that
/// involve at least one changed key (so a violation caused by pins does not
/// block unrelated updates).
pub(crate) fn violations<'a>(
    snapshot: &'a Snapshot,
    changed: Option<&'a [&'a str]>,
) -> impl Iterator<Item = Problem> + 'a {
    RULES
        .iter()
        .filter(move |rule| match changed {
            Some(keys) => rule.keys.iter().any(|s| keys.contains(&s.key())),
            None => true,
        })
        .filter(|rule| !(rule.holds)(snapshot))
        .map(|rule| Problem {
            key: rule.keys.first().map_or("", |s| s.key()).to_owned(),
            reason: rule.message.to_owned(),
        })
}

/// Exported form: `[{"keys": [...], "message": "..."}]`.
pub(crate) fn schema() -> Json {
    RULES
        .iter()
        .map(|rule| {
            let keys: Vec<&str> = rule.keys.iter().map(|s| s.key()).collect();
            json!({ "keys": keys, "message": rule.message })
        })
        .collect()
}
