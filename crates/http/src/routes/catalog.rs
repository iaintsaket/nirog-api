use axum::Json;
use axum::extract::{Query, State};
use axum::http::header;
use axum::response::IntoResponse;

use crate::AppState;
use crate::dto::{CompareParams, ComparisonResponse, ErrorBody, SearchParams, SearchResponse};
use crate::error::ApiError;

/// Public catalogue data: cacheable for 5 minutes.
const CACHE: (header::HeaderName, &str) = (header::CACHE_CONTROL, "public, max-age=300");

#[utoipa::path(get, path = "/v1/search", tag = "catalog", params(SearchParams),
    responses((status = 200, body = SearchResponse), (status = 400, body = ErrorBody)))]
pub async fn search(
    State(s): State<AppState>,
    Query(p): Query<SearchParams>,
) -> Result<impl IntoResponse, ApiError> {
    let groups = s.catalog.search(&p.q).await?;
    Ok(([CACHE], Json(SearchResponse::from(groups))))
}

#[utoipa::path(get, path = "/v1/compare", tag = "catalog", params(CompareParams),
    responses((status = 200, body = ComparisonResponse), (status = 400, body = ErrorBody), (status = 404, body = ErrorBody)))]
pub async fn compare(
    State(s): State<AppState>,
    Query(p): Query<CompareParams>,
) -> Result<impl IntoResponse, ApiError> {
    let view = s.catalog.compare(&p.key, p.searched.as_deref()).await?;
    Ok(([CACHE], Json(ComparisonResponse::from(view))))
}
