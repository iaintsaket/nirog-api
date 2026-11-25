use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum_extra::extract::CookieJar;
use nirog_application::{AppError, User};

use crate::AppState;
use crate::error::ApiError;

/// The signed-in user, resolved from the session cookie. Rejects with 401.
#[derive(Debug)]
pub struct CurrentUser {
    pub user: User,
}

pub(crate) fn cookie(parts_headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    CookieJar::from_headers(parts_headers)
        .get(name)
        .map(|c| c.value().to_owned())
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = cookie(&parts.headers, state.session_cookie())
            .ok_or(ApiError(AppError::Unauthorized))?;
        let user = state.auth.current_user(&token).await?;
        Ok(Self { user })
    }
}
