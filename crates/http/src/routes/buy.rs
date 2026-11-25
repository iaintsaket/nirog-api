use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use nirog_application::AppError;

use crate::AppState;
use crate::dto::ErrorBody;
use crate::error::ApiError;

/// Sends a buyer to an online pharmacy: the offer's product page there if one is stored, else
/// the pharmacy's search for the offer's name. Destinations come only from the database, so
/// nothing in the request can choose where it goes. Each redirect is counted per source and day.
#[utoipa::path(get, path = "/v1/go/{offer_id}/{source_id}", tag = "catalog",
    params(
        ("offer_id" = String, Path, description = "Offer id"),
        ("source_id" = String, Path, description = "Source id: source_a, source_b, source_c or source_d"),
    ),
    responses(
        (status = 302, description = "Redirect to the pharmacy",
            headers(("Location" = String, description = "The pharmacy's https page"))),
        (status = 400, body = ErrorBody), (status = 404, body = ErrorBody)))]
pub async fn go(
    State(s): State<AppState>,
    Path((offer_id, source_id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let url = s.buy.go(&offer_id, &source_id).await?;
    let location = HeaderValue::try_from(url).map_err(|_| ApiError(AppError::NotFound))?;
    Ok((
        StatusCode::FOUND,
        [
            (header::LOCATION, location),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (
                header::REFERRER_POLICY,
                HeaderValue::from_static("no-referrer"),
            ),
        ],
    )
        .into_response())
}
