use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use nirog_application::{AppError, IdentityError};

use crate::dto::ErrorBody;

/// Maps use-case errors to HTTP. Internal details are logged, never returned.
#[derive(Debug)]
pub struct ApiError(pub AppError);

impl From<AppError> for ApiError {
    fn from(e: AppError) -> Self {
        Self(e)
    }
}

pub(crate) fn body(status: StatusCode, code: &'static str, message: &str) -> Response {
    (
        status,
        Json(ErrorBody {
            error: code.into(),
            message: message.into(),
        }),
    )
        .into_response()
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self.0 {
            AppError::NotFound => body(StatusCode::NOT_FOUND, "not_found", "Not found"),
            AppError::Unauthorized => {
                body(StatusCode::UNAUTHORIZED, "unauthorized", "Please sign in")
            }
            AppError::Invalid(why) => body(StatusCode::BAD_REQUEST, "invalid", why),
            AppError::Limit(why) => body(StatusCode::CONFLICT, "limit", why),
            AppError::Identity(IdentityError::Rejected(e)) => {
                tracing::warn!(error = %e, "identity rejected");
                body(
                    StatusCode::UNAUTHORIZED,
                    "unauthorized",
                    "Sign-in was not accepted",
                )
            }
            AppError::Identity(IdentityError::Unavailable(e)) => {
                tracing::error!(error = %e, "identity provider unavailable");
                body(
                    StatusCode::BAD_GATEWAY,
                    "upstream",
                    "Sign-in is unavailable, try again",
                )
            }
            AppError::Repo(e) => {
                tracing::error!(error = %e, "repository failure");
                body(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "Something went wrong",
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nirog_application::RepoError;

    #[test]
    fn every_app_error_maps_to_a_status_without_leaking_details() {
        let cases = [
            (AppError::NotFound, StatusCode::NOT_FOUND),
            (AppError::Unauthorized, StatusCode::UNAUTHORIZED),
            (AppError::Invalid("x"), StatusCode::BAD_REQUEST),
            (AppError::Limit("x"), StatusCode::CONFLICT),
            (
                AppError::Identity(IdentityError::Rejected("x".into())),
                StatusCode::UNAUTHORIZED,
            ),
            (
                AppError::Identity(IdentityError::Unavailable("x".into())),
                StatusCode::BAD_GATEWAY,
            ),
            (
                AppError::Repo(RepoError("password=hunter2".into())),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ];
        for (err, status) in cases {
            assert_eq!(ApiError::from(err).into_response().status(), status);
        }
    }
}
