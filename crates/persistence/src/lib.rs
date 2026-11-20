//! Postgres adapters for the application ports. Each repository is a thin DAO
//! over a shared `PgPool`; SQL stays here and never leaks into use cases.

mod offers;
mod saved;
mod sessions;
mod stats;
mod users;

pub use offers::PgOfferRepository;
pub use saved::PgSavedMatchRepository;
pub use sessions::{PgLoginStateRepository, PgSessionRepository};
pub use stats::PgStatsRepository;
pub use users::PgUserRepository;

use nirog_application::RepoError;
pub use sqlx::PgPool;
use sqlx::migrate::Migrator;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

/// Migrations embedded at compile time, so the server binary carries its schema.
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Opens a pool sized for a small Cloud Run instance talking to Neon.
pub async fn connect(url: &str, max_connections: u32) -> Result<PgPool, RepoError> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(Duration::from_secs(5))
        .connect(url)
        .await
        .map_err(db)
}

pub async fn migrate(pool: &PgPool) -> Result<(), RepoError> {
    MIGRATOR
        .run(pool)
        .await
        .map_err(|e| RepoError(e.to_string()))
}

pub(crate) fn db(e: sqlx::Error) -> RepoError {
    RepoError(e.to_string())
}
