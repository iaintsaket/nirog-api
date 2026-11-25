use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use uuid::Uuid;

use crate::AppState;
use crate::dto::{ErrorBody, SaveMatchRequest, SavedMatchDto};
use crate::error::ApiError;
use crate::extract::CurrentUser;

#[utoipa::path(get, path = "/v1/saved", tag = "saved",
    responses((status = 200, body = Vec<SavedMatchDto>), (status = 401, body = ErrorBody)))]
pub async fn list(
    State(s): State<AppState>,
    me: CurrentUser,
) -> Result<Json<Vec<SavedMatchDto>>, ApiError> {
    let items = s.saved.list(me.user.id).await?;
    Ok(Json(items.into_iter().map(SavedMatchDto::from).collect()))
}

/// Saves a match; saving the same key again updates its label.
#[utoipa::path(post, path = "/v1/saved", tag = "saved", request_body = SaveMatchRequest,
    responses((status = 201, body = SavedMatchDto), (status = 400, body = ErrorBody), (status = 401, body = ErrorBody),
              (status = 404, body = ErrorBody), (status = 409, body = ErrorBody)))]
pub async fn create(
    State(s): State<AppState>,
    me: CurrentUser,
    Json(req): Json<SaveMatchRequest>,
) -> Result<(StatusCode, Json<SavedMatchDto>), ApiError> {
    let saved = s
        .saved
        .save(me.user.id, &req.key, req.offer_id.as_deref(), &req.label)
        .await?;
    Ok((StatusCode::CREATED, Json(saved.into())))
}

#[utoipa::path(delete, path = "/v1/saved/{id}", tag = "saved", params(("id" = Uuid, Path)),
    responses((status = 204), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody)))]
pub async fn remove(
    State(s): State<AppState>,
    me: CurrentUser,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    s.saved.delete(me.user.id, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
