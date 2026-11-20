pub mod auth;
pub mod catalog;
pub mod saved;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use utoipa::OpenApi;

use crate::dto::Health;
use crate::error::body;

/// Liveness check. Exempt from the proxy secret so the platform can probe it.
#[utoipa::path(get, path = "/v1/health", responses((status = 200, body = Health)))]
pub async fn health() -> Json<Health> {
    Json(Health {
        status: "ok".into(),
    })
}

pub async fn openapi_json() -> Response {
    Json(crate::ApiDoc::openapi()).into_response()
}

pub async fn not_found() -> Response {
    body(StatusCode::NOT_FOUND, "not_found", "Not found")
}
