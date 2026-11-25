use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

use crate::AppState;
use crate::dto::{ErrorBody, SaveMatchRequest};
use crate::error::ApiError;
use crate::extract::CurrentUser;

/// Records that the user opened a medicine; it moves to the top of `recent` on `/v1/me`.
#[utoipa::path(post, path = "/v1/me/recent", tag = "auth", request_body = SaveMatchRequest,
    responses((status = 204), (status = 400, body = ErrorBody), (status = 401, body = ErrorBody),
              (status = 404, body = ErrorBody)))]
pub async fn record(
    State(s): State<AppState>,
    me: CurrentUser,
    Json(req): Json<SaveMatchRequest>,
) -> Result<StatusCode, ApiError> {
    s.recent
        .record(me.user.id, &req.key, req.offer_id.as_deref(), &req.label)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Clears the user's recently opened medicines.
#[utoipa::path(delete, path = "/v1/me/recent", tag = "auth",
    responses((status = 204), (status = 401, body = ErrorBody)))]
pub async fn clear(State(s): State<AppState>, me: CurrentUser) -> Result<StatusCode, ApiError> {
    s.recent.clear(me.user.id).await?;
    Ok(StatusCode::NO_CONTENT)
}
