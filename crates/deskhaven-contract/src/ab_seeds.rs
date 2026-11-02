//! Address book seeds (03 §5.1). Each runs after `client-session` (user `alice`) and writes
//! through the real `AddressBookService` over the app's state.

use std::sync::Arc;

use anyhow::Context;
use deskhaven_config::settings::{ADDRESSBOOK_MAX_PEERS, Change};
use deskhaven_core::addressbooks::{
    Actor, AddressBookService, NewSharedBook, PeerPatch, Rule, Subject, Tag,
};
use deskhaven_core::settings::UpdateRequest;
use deskhaven_core::state::AppState;
use deskhaven_core::users::{Credential, NewUser, Password};
use serde_json::{Value, json};

use crate::seeds::{SEED_PASSWORD, SEED_USERNAME, SeedOutputs};

/// Peer of `ab-personal`.
pub const PERSONAL_PEER: &str = "111111111";
/// Peer of each `ab-shared-*` book, and its password.
pub const SHARED_PEER: &str = "222222222";
pub const SHARED_PASSWORD: &str = "shared-Peer-pw";
/// The owner of the `ab-foreign` books.
pub const FOREIGN_USERNAME: &str = "bob";

fn service(state: &AppState) -> AddressBookService {
    AddressBookService::new(
        state.db.clone(),
        state.settings.clone(),
        Arc::clone(&state.crypto),
    )
}

async fn alice(state: &AppState) -> anyhow::Result<i64> {
    let user = state.users.get_by_username(SEED_USERNAME).await;
    Ok(user
        .context("address book seeds run after client-session")?
        .id)
}

fn peer(value: Value) -> anyhow::Result<PeerPatch> {
    Ok(PeerPatch::from_json(value)?)
}

/// Personal book of `alice` with peer [`PERSONAL_PEER`] (hash `seed-hash`, alias
/// `office`, tag `work`, relay forced, unknown field `futureField`) and tag `work`
/// (`0xFF0000FF`). Outputs `personal_guid`.
pub async fn personal(state: &AppState) -> anyhow::Result<SeedOutputs> {
    let books = service(state);
    let owner = alice(state).await?;
    let book = books.personal(owner).await?;
    let me = Actor::User(owner);
    let tag = Tag {
        name: "work".to_owned(),
        color: 0xFF00_00FF,
    };
    books.add_tag(me, &book.guid, tag).await?;
    let sent = json!({
        "id": PERSONAL_PEER, "hash": "seed-hash", "alias": "office", "tags": ["work"],
        "forceAlwaysRelay": "true", "futureField": { "kept": true }
    });
    books.add_peer(me, &book.guid, peer(sent)?).await?;
    Ok(SeedOutputs::from([("personal_guid".to_owned(), book.guid)]))
}

/// An admin-managed shared book `name` granted to `alice` at `rule`, holding peer
/// [`SHARED_PEER`] (password [`SHARED_PASSWORD`]) and tag `ops`.
async fn shared(state: &AppState, name: &str, rule: Rule) -> anyhow::Result<String> {
    let books = service(state);
    let owner = alice(state).await?;
    let new = NewSharedBook {
        name: name.to_owned(),
        note: format!("{name} note"),
        owner_user_id: None,
    };
    let book = books.create_shared(new).await?;
    books
        .set_share(Actor::Admin, &book.guid, Subject::User(owner), rule)
        .await?;
    let tag = Tag {
        name: "ops".to_owned(),
        color: 0xFF00_FF00,
    };
    books.add_tag(Actor::Admin, &book.guid, tag).await?;
    let sent = json!({ "id": SHARED_PEER, "password": SHARED_PASSWORD, "alias": "db" });
    books
        .add_peer(Actor::Admin, &book.guid, peer(sent)?)
        .await?;
    Ok(book.guid)
}

/// `Team Read` (rule 1). Outputs `read_guid`.
pub async fn shared_read(state: &AppState) -> anyhow::Result<SeedOutputs> {
    let guid = shared(state, "Team Read", Rule::ReadOnly).await?;
    Ok(SeedOutputs::from([("read_guid".to_owned(), guid)]))
}

/// `Team Write` (rule 2). Outputs `write_guid`.
pub async fn shared_write(state: &AppState) -> anyhow::Result<SeedOutputs> {
    let guid = shared(state, "Team Write", Rule::ReadWrite).await?;
    Ok(SeedOutputs::from([("write_guid".to_owned(), guid)]))
}

/// `Team Full` (rule 3). Outputs `full_guid`.
pub async fn shared_full(state: &AppState) -> anyhow::Result<SeedOutputs> {
    let guid = shared(state, "Team Full", Rule::FullControl).await?;
    Ok(SeedOutputs::from([("full_guid".to_owned(), guid)]))
}

/// User [`FOREIGN_USERNAME`] with a personal book and an owned shared book `Bob's`, neither
/// reachable by `alice`. Outputs `foreign_personal_guid`, `foreign_shared_guid`.
pub async fn foreign(state: &AppState) -> anyhow::Result<SeedOutputs> {
    let books = service(state);
    let password = Password::new(SEED_PASSWORD);
    let new = NewUser::new(FOREIGN_USERNAME, Credential::Local { password });
    let bob = state.users.create(new).await?.id;
    let personal = books.personal(bob).await?;
    let new = NewSharedBook {
        name: "Bob's".to_owned(),
        note: String::new(),
        owner_user_id: Some(bob),
    };
    let shared = books.create_shared(new).await?;
    Ok(SeedOutputs::from([
        ("foreign_personal_guid".to_owned(), personal.guid),
        ("foreign_shared_guid".to_owned(), shared.guid),
    ]))
}

/// `addressbook.max_peers` = 1 (with `ab-personal`, alice's personal book is full).
pub async fn one_peer_max(state: &AppState) -> anyhow::Result<SeedOutputs> {
    let key = ADDRESSBOOK_MAX_PEERS.key();
    let version = state
        .settings
        .snapshot()
        .view(key)
        .context("addressbook.max_peers is registered")?
        .version;
    let change = Change {
        key: key.to_owned(),
        value: json!(1),
        version,
    };
    state
        .settings
        .update(UpdateRequest {
            changes: vec![change],
            actor: None,
            reason: None,
        })
        .await?;
    Ok(SeedOutputs::new())
}
