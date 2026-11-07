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

/// Where a flow completes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewTarget {
    /// A client flow: confirmed on the callback page, for this device.
    Device(DeviceSummary),
    /// A console login: completed by the console page that started it.
    Console,
    /// A link flow: confirmed and completed on the console's link page.
    Link,
}

/// The pending flow behind a callback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallbackPreview {
    /// The provider's display name.
    pub provider: String,
    pub target: PreviewTarget,
}

impl OidcService {
    /// The live flow of `state`, without touching it. [`CallbackError::InvalidState`] for
    /// an unknown, expired or already used state.
    pub async fn preview_callback(
        &self,
        state: Option<&str>,
    ) -> Result<CallbackPreview, CallbackError> {
        let row = self.live_flow(state).await?;
        let target = match row.flow {
            Flow::Console => PreviewTarget::Console,
            Flow::Link => PreviewTarget::Link,
            Flow::Client => PreviewTarget::Device(device_summary(&row.context)?),
        };
        Ok(CallbackPreview {
            provider: self.provider_display_name(row.provider_id).await?,
            target,
        })
    }

    async fn provider_display_name(&self, provider_id: i64) -> Result<String, CallbackError> {
        let provider = oidc_provider::Entity::find_by_id(provider_id)
            .one(&self.inner.db)
            .await?
            .ok_or(CallbackError::InvalidState)?;
        Ok(provider.display_name)
    }
}

fn device_summary(raw: &str) -> Result<DeviceSummary, CallbackError> {
    let device = context::client_device(raw)
        .ok_or_else(|| CallbackError::Internal("corrupt OIDC flow context".into()))?;
    let text = |key: &str| {
        device
            .info
            .as_ref()
            .and_then(|info| info.get(key))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    Ok(DeviceSummary {
        name: text("name"),
        os: text("os"),
        rustdesk_id: device.rustdesk_id,
    })
}
