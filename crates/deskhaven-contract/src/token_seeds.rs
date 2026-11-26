use anyhow::Context;
use deskhaven_config::settings::{Change, DEVICES_REQUIRE_DEPLOY};
use deskhaven_core::api_tokens::{NewApiToken, Scope};
use deskhaven_core::settings::UpdateRequest;
use deskhaven_core::state::AppState;

use crate::seeds::SeedOutputs;

/// `api-token`: a system token (no creator) with both device scopes. Outputs `api_token`.
pub async fn deploy_and_assign(state: &AppState) -> anyhow::Result<SeedOutputs> {
    issue(state, &[Scope::Deploy, Scope::Assign]).await
}

/// `api-token-deploy-only`: outputs `api_token`, which lacks `assign`.
pub async fn deploy_only(state: &AppState) -> anyhow::Result<SeedOutputs> {
    issue(state, &[Scope::Deploy]).await
}

/// `api-token-assign-only`: outputs `api_token`, which lacks `deploy`.
pub async fn assign_only(state: &AppState) -> anyhow::Result<SeedOutputs> {
    issue(state, &[Scope::Assign]).await
}

async fn issue(state: &AppState, scopes: &[Scope]) -> anyhow::Result<SeedOutputs> {
    let new = NewApiToken {
        name: "contract".to_owned(),
        scopes: scopes.iter().copied().collect(),
        created_by: None,
        expires_at: None,
    };
    let (_, secret) = state.api_tokens.create(new).await?;
    Ok(SeedOutputs::from([(
        "api_token".to_owned(),
        secret.expose().to_owned(),
    )]))
}

/// Sets `devices.require_deploy` (off by default: `--deploy` then answers `NOT_ENABLED`).
pub async fn deploy_required(state: &AppState) -> anyhow::Result<SeedOutputs> {
    let key = DEVICES_REQUIRE_DEPLOY.key();
    let version = state
        .settings
        .snapshot()
        .view(key)
        .with_context(|| format!("unknown setting {key:?}"))?
        .version;
    let change = Change {
        key: key.to_owned(),
        value: serde_json::json!(true),
        version,
    };
    let request = UpdateRequest {
        changes: vec![change],
        actor: None,
        reason: None,
    };
    state.settings.update(request).await?;
    Ok(SeedOutputs::new())
}
