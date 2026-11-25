//! Nirog API server. Cloud Run sets `PORT`; see `config.rs` for every variable.

mod config;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use nirog_application::ports::{Clock, SystemClock};
use nirog_application::sellers::DefaultSellerLinks;
use nirog_application::services::{
    AuthService, CatalogService, CloseMatchService, PagesService, RecentService, SavedMatchService,
    StatsService,
};
use nirog_http::AppState;
use nirog_identity_google::{GoogleConfig, GoogleIdentityProvider, Secret};
use nirog_persistence::{
    PgLoginStateRepository, PgMedicinePages, PgOfferRepository, PgRecentRepository,
    PgSavedMatchRepository, PgSessionRepository, PgStatsRepository, PgUserRepository,
    PgVendorListings,
};
use nirog_sources::aliases::BundledAliases;
use tracing_subscriber::EnvFilter;

use crate::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let config = Config::from_env(|k| std::env::var(k).ok())?;
    tracing::info!(?config, "starting");

    let pool = nirog_persistence::connect(&config.database_url, config.db_max_connections).await?;
    nirog_persistence::migrate(&pool).await?;

    let google = GoogleIdentityProvider::new(GoogleConfig::new(
        config.google_client_id.clone(),
        Secret::new(config.google_client_secret.clone()),
        config.google_redirect_uri.clone(),
    )?)?;
    let offers = Arc::new(PgOfferRepository::new(pool.clone()));
    let sessions = PgSessionRepository::new(pool.clone());
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let state = AppState {
        catalog: CatalogService::new(
            offers.clone(),
            Arc::new(DefaultSellerLinks),
            Arc::new(BundledAliases::bundled()),
        )
        .with_stock(Arc::new(PgVendorListings::new(pool.clone())), clock.clone()),
        auth: AuthService::new(
            Arc::new(PgUserRepository::new(pool.clone())),
            Arc::new(sessions.clone()),
            Arc::new(PgLoginStateRepository::new(pool.clone())),
            Arc::new(google),
            clock.clone(),
            time::Duration::days(30),
        ),
        saved: SavedMatchService::new(
            Arc::new(PgSavedMatchRepository::new(pool.clone())),
            offers.clone(),
        ),
        recent: RecentService::new(
            Arc::new(PgRecentRepository::new(pool.clone())),
            offers.clone(),
        ),
        close: CloseMatchService::new(offers.clone()),
        pages: PagesService::new(
            Arc::new(PgMedicinePages::new(pool.clone())),
            Arc::new(DefaultSellerLinks),
        ),
        stats: StatsService::new(Arc::new(PgStatsRepository::new(pool)), clock.clone()),
        config: Arc::new(config.http.clone()),
    };

    // Housekeeping: drop expired sessions and login states hourly.
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tick.tick().await;
            match sessions.purge_expired(clock.now()).await {
                Ok(n) => tracing::info!(removed = n, "purged expired sessions"),
                Err(e) => tracing::warn!(error = %e, "session purge failed"),
            }
        }
    });

    let app = nirog_http::router(state).into_make_service_with_connect_info::<SocketAddr>();
    let listener =
        tokio::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], config.port))).await?;
    tracing::info!(port = config.port, "listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

/// Cloud Run sends SIGTERM before stopping an instance.
async fn shutdown() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { () = ctrl_c => {}, () = term => {} }
    tracing::info!("shutting down");
}
