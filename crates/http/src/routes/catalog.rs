use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::AppState;
use crate::dto::{
    BrandAliasDto, CloseParams, CloseResponse, CompareParams, ComparisonResponse, ErrorBody,
    PagesParams, PagesResponse, SearchParams, SearchResponse, StatsResponse,
};
use crate::error::{ApiError, body};
use crate::middleware::constant_eq;

/// Public catalogue data: cacheable for 5 minutes.
const CACHE: (header::HeaderName, &str) = (header::CACHE_CONTROL, "public, max-age=300");
/// Stats change only when prices are imported: browsers keep them 5 minutes, the edge cache an hour.
const STATS_CACHE: (header::HeaderName, &str) =
    (header::CACHE_CONTROL, "public, max-age=300, s-maxage=3600");

#[utoipa::path(get, path = "/v1/search", tag = "catalog", params(SearchParams),
    responses((status = 200, body = SearchResponse), (status = 400, body = ErrorBody)))]
pub async fn search(
    State(s): State<AppState>,
    Query(p): Query<SearchParams>,
) -> Result<impl IntoResponse, ApiError> {
    let groups = s.catalog.search(&p.q).await?;
    let mut body = SearchResponse::from(groups);
    if body.groups.is_empty() {
        if let Some((alias, groups)) = s.catalog.same_salt_as_brand(&p.q).await? {
            body = SearchResponse::from(groups);
            body.same_salt_as = Some(BrandAliasDto::from(&alias));
        } else {
            body.did_you_mean = s.catalog.closest(&p.q).await?;
        }
    }
    Ok(([CACHE], Json(body)))
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

#[utoipa::path(get, path = "/v1/stats", tag = "catalog",
    responses((status = 200, body = StatsResponse)))]
pub async fn stats(State(s): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    Ok((
        [STATS_CACHE],
        Json(StatsResponse::from(s.stats.stats().await?)),
    ))
}

#[utoipa::path(get, path = "/v1/close", tag = "catalog", params(CloseParams),
    responses((status = 200, body = CloseResponse), (status = 400, body = ErrorBody)))]
pub async fn close(
    State(s): State<AppState>,
    Query(p): Query<CloseParams>,
) -> Result<impl IntoResponse, ApiError> {
    Ok((
        [CACHE],
        Json(CloseResponse::from(s.close.close(&p.key).await?)),
    ))
}

/// Every medicine with a substitute, in batches of 500, for building the website's medicine
/// pages. Called a few dozen times per website deploy, never by browsers. With a pages token
/// configured it answers only `Authorization: Bearer <token>`, and it is never shared-cached.
#[utoipa::path(get, path = "/v1/pages", tag = "catalog", params(PagesParams),
    responses((status = 200, body = PagesResponse), (status = 400, body = ErrorBody),
        (status = 403, body = ErrorBody)))]
pub async fn pages(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(p): Query<PagesParams>,
) -> Result<Response, ApiError> {
    if let Some(token) = s.config.pages_token.as_deref() {
        let presented = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.as_bytes().strip_prefix(b"Bearer "))
            .is_some_and(|t| constant_eq(t, token.as_bytes()));
        if !presented {
            return Ok(body(
                StatusCode::FORBIDDEN,
                "forbidden",
                "This endpoint is for the website build only",
            ));
        }
    }
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(PagesResponse::from(
            s.pages.batch(p.after.as_deref()).await?,
        )),
    )
        .into_response())
}
