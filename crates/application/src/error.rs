use thiserror::Error;

/// Failure inside a repository adapter. The message is for logs, never for users.
#[derive(Debug, Error)]
#[error("repository error: {0}")]
pub struct RepoError(pub String);

#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("identity provider rejected the request: {0}")]
    Rejected(String),
    #[error("identity provider unavailable: {0}")]
    Unavailable(String),
}

/// Errors a use case can return. The HTTP layer maps each to a status code.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,
    #[error("not signed in")]
    Unauthorized,
    #[error("invalid input: {0}")]
    Invalid(&'static str),
    #[error("limit reached: {0}")]
    Limit(&'static str),
    #[error(transparent)]
    Identity(#[from] IdentityError),
    #[error(transparent)]
    Repo(#[from] RepoError),
}
