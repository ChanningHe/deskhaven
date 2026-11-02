//! `POST /api/ab/personal`, `/api/ab/settings`, `/api/ab/shared/profiles`.

use std::sync::Arc;

use axum::Json;
use axum::extract::{RawQuery, State};
use deskhaven_core::addressbooks::{Profile, Rule};
use deskhaven_core::state::AppState;
use serde::Serialize;

use super::{ab_error, query, service};
use crate::{BearerUser, ClientApiError};

#[derive(Serialize)]
pub struct PersonalReply {
    guid: String,
}

#[derive(Serialize)]
pub struct SettingsReply {
    max_peer_one_ab: u32,
}

/// `{total, data}` (03 §1.3).
#[derive(Serialize)]
pub struct PageReply<T> {
    pub total: u64,
    pub data: Vec<T>,
}

/// `AbProfile.fromJson` (`flutter/lib/common/hbbs/hbbs.dart:268-274`); `info` is left
/// out (the client keeps it as an opaque `dynamic`).
#[derive(Serialize)]
pub struct ProfileReply {
    guid: String,
    name: String,
    /// Owner's username; `""` for an admin-managed book.
    owner: String,
    note: String,
    rule: u8,
}

impl From<Profile> for ProfileReply {
    fn from(p: Profile) -> Self {
        Self {
            guid: p.book.guid,
            name: p.book.name,
            owner: p.owner.unwrap_or_default(),
            note: p.book.note,
            rule: rule_number(p.rule),
        }
    }
}

/// `ShareRule` values (`flutter/lib/common/hbbs/hbbs.dart:210-216`).
fn rule_number(rule: Rule) -> u8 {
    match rule {
        Rule::ReadOnly => 1,
        Rule::ReadWrite => 2,
        Rule::FullControl => 3,
    }
}

/// The caller's personal book, created on first access.
pub async fn personal(
    BearerUser(principal): BearerUser,
    State(state): State<Arc<AppState>>,
) -> Result<Json<PersonalReply>, ClientApiError> {
    let book = service(&state)
        .personal(principal.user_id)
        .await
        .map_err(ab_error)?;
    Ok(Json(PersonalReply { guid: book.guid }))
}

/// `addressbook.max_peers`.
pub async fn settings(
    BearerUser(_): BearerUser,
    State(state): State<Arc<AppState>>,
) -> Json<SettingsReply> {
    Json(SettingsReply {
        max_peer_one_ab: service(&state).max_peers(),
    })
}

/// One page of the shared books the caller reaches, with the caller's rule.
pub async fn shared_profiles(
    BearerUser(principal): BearerUser,
    State(state): State<Arc<AppState>>,
    RawQuery(raw): RawQuery,
) -> Result<Json<PageReply<ProfileReply>>, ClientApiError> {
    let page = query::page(raw.as_deref())?;
    let profiles = service(&state)
        .shared_profiles(principal.user_id, page)
        .await
        .map_err(ab_error)?;
    Ok(Json(PageReply {
        total: profiles.total,
        data: profiles.items.into_iter().map(ProfileReply::from).collect(),
    }))
}
