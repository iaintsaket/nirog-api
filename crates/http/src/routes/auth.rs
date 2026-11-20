use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header::SET_COOKIE};
use axum::response::{AppendHeaders, IntoResponse, Redirect, Response};

use crate::AppState;
use crate::dto::{CallbackParams, ErrorBody, UserDto};
use crate::error::ApiError;
use crate::extract::{CurrentUser, cookie};
use crate::middleware::constant_eq;

const LOGIN_COOKIE_SECONDS: i64 = 600;

/// `Set-Cookie` value. Tokens are base64url, so no escaping is needed.
fn set_cookie(state: &AppState, name: &str, value: &str, max_age: i64) -> String {
    let secure = if state.config.secure_cookies {
        "; Secure"
    } else {
        ""
    };
    format!("{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}")
}

fn clear_cookie(state: &AppState, name: &str) -> String {
    set_cookie(state, name, "", 0)
}

/// Starts Google sign-in. Binds the OAuth `state` to this browser with a short-lived cookie.
#[utoipa::path(get, path = "/v1/auth/google/start", tag = "auth",
    responses((status = 303, description = "Redirect to Google"), (status = 500, body = ErrorBody)))]
pub async fn start(State(s): State<AppState>) -> Result<Response, ApiError> {
    let login = s.auth.begin_login().await?;
    let cookie = set_cookie(&s, s.login_cookie(), &login.state, LOGIN_COOKIE_SECONDS);
    Ok((
        AppendHeaders([(SET_COOKIE, cookie)]),
        Redirect::to(&login.redirect_url),
    )
        .into_response())
}

/// Google redirects here. On success sets the session cookie and returns to the website.
#[utoipa::path(get, path = "/v1/auth/google/callback", tag = "auth", params(CallbackParams),
    responses((status = 303, description = "Back to the website, signed in or with ?error=")))]
pub async fn callback(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(p): Query<CallbackParams>,
) -> Response {
    let clear_login = clear_cookie(&s, s.login_cookie());
    let fail = |s: &AppState| {
        let to = format!("{}/signin?error=login_failed", s.config.web_origin);
        (
            AppendHeaders([(SET_COOKIE, clear_login.clone())]),
            Redirect::to(&to),
        )
            .into_response()
    };
    let (Some(code), Some(state), None) = (p.code, p.state, p.error) else {
        return fail(&s);
    };
    // The state must come back to the same browser that started the login.
    let bound = cookie(&headers, s.login_cookie())
        .is_some_and(|c| constant_eq(c.as_bytes(), state.as_bytes()));
    if !bound {
        return fail(&s);
    }
    match s.auth.complete_login(&code, &state).await {
        Ok(token) => {
            let max_age = s.auth.session_ttl().whole_seconds();
            let session = set_cookie(&s, s.session_cookie(), &token.0, max_age);
            let to = format!("{}/saved", s.config.web_origin);
            (
                AppendHeaders([(SET_COOKIE, clear_login), (SET_COOKIE, session)]),
                Redirect::to(&to),
            )
                .into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "login failed");
            fail(&s)
        }
    }
}

#[utoipa::path(post, path = "/v1/auth/logout", tag = "auth", responses((status = 204), (status = 403, body = ErrorBody)))]
pub async fn logout(State(s): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    if let Some(token) = cookie(&headers, s.session_cookie()) {
        s.auth.logout(&token).await?;
    }
    Ok((
        StatusCode::NO_CONTENT,
        AppendHeaders([(SET_COOKIE, clear_cookie(&s, s.session_cookie()))]),
    )
        .into_response())
}

#[utoipa::path(get, path = "/v1/me", tag = "auth", responses((status = 200, body = UserDto), (status = 401, body = ErrorBody)))]
pub async fn me(me: CurrentUser) -> Json<UserDto> {
    Json(me.user.into())
}

/// Deletes the account, its sessions and saved matches (DPDP right to erasure).
#[utoipa::path(delete, path = "/v1/me", tag = "auth",
    responses((status = 204), (status = 401, body = ErrorBody), (status = 403, body = ErrorBody)))]
pub async fn delete_me(State(s): State<AppState>, me: CurrentUser) -> Result<Response, ApiError> {
    s.auth.delete_account(me.user.id).await?;
    Ok((
        StatusCode::NO_CONTENT,
        AppendHeaders([(SET_COOKIE, clear_cookie(&s, s.session_cookie()))]),
    )
        .into_response())
}
