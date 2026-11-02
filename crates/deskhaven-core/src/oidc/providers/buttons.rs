//! The login buttons of `/api/login-options`, cached: the client also calls that endpoint
//! as an unauthenticated TLS probe (03 §3.1), so it must not cost a query per request.
//! Provider writes invalidate the cache (single writer process, 05 §3.6); a load that raced
//! an invalidation is not stored.

use std::sync::{Arc, Mutex, PoisonError};

use deskhaven_db::entity::oidc_provider::{self, Column};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use super::LoginButton;
use crate::oidc::OidcService;

#[derive(Default)]
pub(crate) struct ButtonCache {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    generation: u64,
    buttons: Option<Arc<[LoginButton]>>,
}

impl ButtonCache {
    pub(crate) fn invalidate(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.generation = state.generation.wrapping_add(1);
        state.buttons = None;
    }

    fn cached(&self) -> Result<Arc<[LoginButton]>, u64> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.buttons.clone().ok_or(state.generation)
    }

    fn store(&self, generation: u64, buttons: Arc<[LoginButton]>) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.generation == generation {
            state.buttons = Some(buttons);
        }
    }
}

impl OidcService {
    /// The enabled providers as login buttons, by name. Empty without `public_url` (no
    /// login can complete) and, logged, when the providers cannot be read: this answer
    /// never fails.
    pub async fn login_buttons(&self) -> Arc<[LoginButton]> {
        if self.inner.redirect_uri.is_none() {
            return Arc::from([]);
        }
        let cache = &self.inner.buttons;
        let generation = match cache.cached() {
            Ok(buttons) => return buttons,
            Err(generation) => generation,
        };
        let loaded = oidc_provider::Entity::find()
            .filter(Column::Enabled.eq(true))
            .order_by_asc(Column::Name)
            .all(&self.inner.db)
            .await;
        match loaded {
            Ok(rows) => {
                let buttons: Arc<[LoginButton]> = rows
                    .into_iter()
                    .map(|p| LoginButton {
                        name: p.name,
                        icon: p.icon,
                    })
                    .collect();
                cache.store(generation, Arc::clone(&buttons));
                buttons
            }
            Err(err) => {
                tracing::error!(error = %err, "cannot read OIDC providers for the login options");
                Arc::from([])
            }
        }
    }
}
