//! Defence against cross-device consent phishing: an attacker can send the authorization URL of a
//! flow started on their own device to a victim signed in at the IdP. So the redirect only shows
//! which device is being signed in and completes nothing until the user confirms.

use deskhaven_db::entity::oidc_flow::Flow;
use deskhaven_db::entity::oidc_provider;
use sea_orm::EntityTrait;
use serde_json::Value;

use super::OidcService;
use super::context;
use super::error::CallbackError;

/// Client-supplied text: escape it before display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceSummary {
    pub rustdesk_id: String,
    /// Never shown.
    pub uuid: String,
    pub name: Option<String>,
    pub os: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewTarget {
    Device(DeviceSummary),
    Console,
    Link,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallbackPreview {
    pub provider: String,
    pub target: PreviewTarget,
}

impl OidcService {
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
        uuid: device.uuid,
    })
}
