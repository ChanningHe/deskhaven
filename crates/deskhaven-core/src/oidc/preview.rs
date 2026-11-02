//! What the callback confirmation page shows before the user completes a flow.
//!
//! Defence against cross-device consent phishing: an attacker can start a flow on their own
//! device and send its authorization URL to a victim already signed in at the IdP; the IdP
//! would redirect straight back and log the attacker's device in as the victim. So the
//! redirect only shows which device is being signed in (from the flow, i.e. server-side
//! data) and completes nothing; the user has to confirm (`complete_callback`). Reading the
//! preview never consumes the flow.

use deskhaven_db::entity::oidc_flow::Flow;
use deskhaven_db::entity::oidc_provider;
use sea_orm::EntityTrait;
use serde_json::Value;

use super::OidcService;
use super::context;
use super::error::CallbackError;

/// The device a client flow will sign in, as it described itself at `/api/oidc/auth`.
/// Client-supplied text: escape it before display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceSummary {
    pub rustdesk_id: String,
    /// `deviceInfo.name` (the host name).
    pub name: Option<String>,
    /// `deviceInfo.os`.
    pub os: Option<String>,
}

/// The pending flow behind a callback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallbackPreview {
    /// The provider's display name.
    pub provider: String,
    /// `None` for a console flow.
    pub device: Option<DeviceSummary>,
}

impl OidcService {
    /// The live flow of `state`, without touching it. [`CallbackError::InvalidState`] for
    /// an unknown, expired or already used state.
    pub async fn preview_callback(
        &self,
        state: Option<&str>,
    ) -> Result<CallbackPreview, CallbackError> {
        let row = self.live_flow(state).await?;
        let provider = oidc_provider::Entity::find_by_id(row.provider_id)
            .one(&self.inner.db)
            .await?
            .ok_or(CallbackError::InvalidState)?;
        let device = match row.flow {
            Flow::Console => None,
            Flow::Client => {
                let device = context::client_device(&row.context)
                    .ok_or_else(|| CallbackError::Internal("corrupt OIDC flow context".into()))?;
                let text = |key: &str| {
                    device
                        .info
                        .as_ref()
                        .and_then(|info| info.get(key))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                };
                Some(DeviceSummary {
                    name: text("name"),
                    os: text("os"),
                    rustdesk_id: device.rustdesk_id,
                })
            }
        };
        Ok(CallbackPreview {
            provider: provider.display_name,
            device,
        })
    }
}
