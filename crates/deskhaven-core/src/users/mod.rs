//! No operation may leave the instance without an active owner: those that could first lock all
//! active owners in id order, so two concurrent demotions cannot both pass the check and do not
//! deadlock.

mod admin;
mod auth;
mod error;
mod events;
pub(crate) mod lookup;
mod model;
mod normalize;
mod password;
mod policy;
mod write;

use std::future::Future;
use std::sync::Arc;

use deskhaven_config::settings::AUTH_PASSWORD_MIN_LENGTH;
use deskhaven_db::entity::{group_member, user};
use deskhaven_db::time::TimestampMs;
use sea_orm::sea_query::{Expr, ExprTrait, Func};
use sea_orm::{
    ColumnTrait, Condition, DatabaseConnection, EntityTrait, Order, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, QueryTrait,
};
use tokio::sync::broadcast;

pub use admin::{AccountEdit, BatchError, MAX_BATCH, RowError, TEMPORARY_PASSWORD_LEN};
pub use auth::{Authenticated, CredentialStamp};
pub use deskhaven_db::entity::user::{AuthSource, Role, Status};
pub use error::{AuthError, UserError};
pub use events::{CommitHook, UserChange, UserEvent};
pub use model::{Credential, NewUser, ProfileUpdate, User, UserQuery, UserSort, UserSortField};
pub use password::{HashLimits, HashParams, Password, PasswordError, PasswordHasher};
pub use policy::{MAX_PASSWORD_LEN, PolicyViolation, check_password_policy};
// Also keys the login lockout; OIDC derives usernames by it.
pub use normalize::username_norm;
pub(crate) use normalize::{USERNAME_MAX_LEN, is_username_char};
pub(crate) use write::lock_active_owners;

use crate::clock::{SharedClock, SystemClock};
use crate::commit_hooks::CommitHooks;
pub use crate::commit_hooks::Fallback;
use crate::pagination::Page;
use crate::settings::SettingsService;
use crate::text::contains_pattern;

#[derive(Clone)]
pub struct UserService {
    inner: Arc<Inner>,
}

struct Inner {
    db: DatabaseConnection,
    settings: SettingsService,
    hasher: PasswordHasher,
    clock: SharedClock,
    hooks: CommitHooks<UserChange>,
    events: broadcast::Sender<UserEvent>,
}

impl UserService {
    pub fn new(db: DatabaseConnection, settings: SettingsService, hasher: PasswordHasher) -> Self {
        Self::with_clock(db, settings, hasher, Arc::new(SystemClock))
    }

    pub fn with_clock(
        db: DatabaseConnection,
        settings: SettingsService,
        hasher: PasswordHasher,
        clock: SharedClock,
    ) -> Self {
        let (events, _) = broadcast::channel(events::EVENT_CAPACITY);
        let inner = Inner {
            db,
            settings,
            hasher,
            clock,
            hooks: CommitHooks::default(),
            events,
        };
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<UserEvent> {
        self.inner.events.subscribe()
    }

    pub fn register_commit_hook(&self, hook: CommitHook) {
        self.inner.hooks.register(hook);
    }

    /// `on_panic` runs whenever `hook` panics, so a cache can fail closed.
    pub fn register_commit_hook_with_fallback(&self, hook: CommitHook, on_panic: Fallback) {
        self.inner.hooks.register_with_fallback(hook, on_panic);
    }

    pub async fn get(&self, id: i64) -> Result<User, UserError> {
        let found = user::Entity::find_by_id(id).one(&self.inner.db).await?;
        found.map(User::from).ok_or(UserError::NotFound)
    }

    pub async fn get_by_username(&self, username: &str) -> Result<User, UserError> {
        let found = self.inner.find_by_username(username).await?;
        found.map(User::from).ok_or(UserError::NotFound)
    }

    pub async fn list(&self, query: &UserQuery) -> Result<Page<User>, UserError> {
        let index = query.page.index().map_err(|p| UserError::Invalid {
            field: p.field,
            reason: p.reason,
        })?;
        let mut select = user::Entity::find();
        if let Some(status) = query.status {
            select = select.filter(user::Column::Status.eq(status));
        }
        if let Some(role) = query.role {
            select = select.filter(user::Column::Role.eq(role));
        }
        if let Some(source) = query.auth_source {
            select = select.filter(user::Column::AuthSource.eq(source));
        }
        if let Some(group_id) = query.group_id {
            select = select.filter(user::Column::Id.in_subquery(member_ids(group_id)));
        }
        if let Some(group_id) = query.not_group_id {
            select = select.filter(user::Column::Id.not_in_subquery(member_ids(group_id)));
        }
        if let Some(q) = query.q.as_deref().filter(|q| !q.is_empty()) {
            select = select.filter(matches_text(q));
        }
        let mut tie = Order::Asc;
        if let Some(sort) = query.sort {
            let order = if sort.descending {
                Order::Desc
            } else {
                Order::Asc
            };
            select = select.order_by(sort.field.column(), order.clone());
            tie = order;
        }
        let paginator = select
            .order_by(user::Column::Id, tie)
            .paginate(&self.inner.db, query.page.page_size);
        let total = paginator.num_items().await?;
        let items = paginator.fetch_page(index).await?;
        Ok(Page {
            items: items.into_iter().map(User::from).collect(),
            total,
            page: query.page.page,
            page_size: query.page.page_size,
        })
    }

    /// Dropping the caller's future can then neither interrupt a commit nor lose the event that follows
    /// it.
    async fn detached<T, F, Fut>(&self, op: F) -> Result<T, UserError>
    where
        F: FnOnce(Arc<Inner>) -> Fut,
        Fut: Future<Output = Result<T, UserError>> + Send + 'static,
        T: Send + 'static,
    {
        tokio::spawn(op(Arc::clone(&self.inner)))
            .await
            .map_err(|_| UserError::Aborted)?
    }
}

impl Inner {
    async fn find_by_username(
        &self,
        username: &str,
    ) -> Result<Option<user::Model>, sea_orm::DbErr> {
        lookup::find_by_username(&self.db, username).await
    }

    fn min_password_length(&self) -> u32 {
        self.settings.snapshot().get(&AUTH_PASSWORD_MIN_LENGTH)
    }

    fn now(&self) -> TimestampMs {
        self.clock.now()
    }

    /// No receiver is not an error: nobody needs to react.
    fn committed(&self, change: UserChange) {
        self.hooks.run(&change);
        if let UserChange::Event(event) = change {
            let _ = self.events.send(event);
        }
    }
}

fn member_ids(group_id: i64) -> sea_orm::sea_query::SelectStatement {
    group_member::Entity::find()
        .select_only()
        .column(group_member::Column::UserId)
        .filter(group_member::Column::GroupId.eq(group_id))
        .into_query()
}

/// Non-ASCII letters match as typed; SQLite's `lower()` folds ASCII only, so non-ASCII display
/// names match case-sensitively there.
fn matches_text(q: &str) -> Condition {
    let norm = || contains_pattern(&q.to_ascii_lowercase());
    let lower = || contains_pattern(&q.to_lowercase());
    let display_name = Expr::expr(Func::lower(Expr::col(user::Column::DisplayName)));
    Condition::any()
        .add(user::Column::UsernameNorm.like(norm()))
        .add(user::Column::EmailNorm.like(norm()))
        .add(display_name.like(lower()))
}
