//! `GET /strategy-preview?device_id=`: the strategy a device ends up with, each option
//! with the layer it comes from, as the next heartbeat delivers it (secret values withheld).

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use deskhaven_core::state::AppState;

use super::Problem;
use super::view::PreviewQuery;
use crate::devices::{EffectiveStrategyView, effective_strategy};
use crate::{AdminApiError, ValidQuery};

/// The merged strategy of one device with the source of every option.
#[utoipa::path(
    get,
    path = "/strategy-preview",
    tag = "strategies",
    operation_id = "preview_strategy",
    security(("console_session" = ["admin"])),
    params(PreviewQuery),
    responses(
        (status = 200, description = "The effective strategy", body = EffectiveStrategyView),
        (status = 404, description = "No such device", body = Problem),
    )
)]
pub async fn preview_strategy(
    State(state): State<Arc<AppState>>,
    ValidQuery(query): ValidQuery<PreviewQuery>,
) -> Result<Json<EffectiveStrategyView>, AdminApiError> {
    let summary = state.devices.summary(query.device_id).await?;
    Ok(Json(effective_strategy(&state, &summary.record).await?))
}
