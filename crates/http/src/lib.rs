//! HTTP adapter. Turns use cases into a versioned JSON API under `/v1`.
//!
//! Security model: the browser talks to the API through a same-origin proxy,
//! sessions live in an HttpOnly cookie holding a random token (only its hash is
//! stored), writes must carry the exact web `Origin`, and every response gets
//! strict security headers.

mod dto;
mod error;
mod extract;
mod middleware;
mod openapi;
mod routes;

use std::sync::Arc;
use std::time::Duration;

use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderName, HeaderValue, Method, StatusCode, header};
use axum::routing::{delete, get, post};
use axum::{Router, middleware::from_fn_with_state};
use nirog_application::services::{
    AuthService, CatalogService, CloseMatchService, SavedMatchService, StatsService,
};
use tower_governor::GovernorLayer;
use tower_governor::governor::GovernorConfigBuilder;
use tower_http::cors::CorsLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

pub use dto::*;
pub use middleware::ClientIp;
pub use openapi::ApiDoc;

/// Largest request body accepted anywhere (the biggest real body is a saved match).
pub const BODY_LIMIT: usize = 16 * 1024;
pub const PROXY_KEY_HEADER: &str = "x-nirog-proxy-key";
pub const CLIENT_IP_HEADER: &str = "x-nirog-client-ip";

#[derive(Debug, Clone, Copy)]
pub struct RateLimit {
    /// One request is replenished every `replenish_ms` milliseconds.
    pub replenish_ms: u64,
    pub burst: u32,
}

#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// Exact origin of the website, e.g. `https://nirog.pages.dev`. No trailing slash.
    pub web_origin: String,
    /// `true` in production: `Secure` + `__Host-` cookies. `false` only for local http.
    pub secure_cookies: bool,
    /// When set, every request except `/v1/health` must carry it in
    /// `x-nirog-proxy-key`, so the API is reachable only through the web proxy.
    pub proxy_secret: Option<String>,
    pub rate_limit: Option<RateLimit>,
    pub request_timeout: Duration,
}

#[derive(Clone)]
pub struct AppState {
    pub catalog: CatalogService,
    pub auth: AuthService,
    pub saved: SavedMatchService,
    pub stats: StatsService,
    pub close: CloseMatchService,
    pub config: Arc<HttpConfig>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl AppState {
    pub(crate) fn session_cookie(&self) -> &'static str {
        if self.config.secure_cookies {
            "__Host-nirog_session"
        } else {
            "nirog_session"
        }
    }
    pub(crate) fn login_cookie(&self) -> &'static str {
        if self.config.secure_cookies {
            "__Host-nirog_login"
        } else {
            "nirog_login"
        }
    }
}

fn header_layer(name: &'static str, value: &'static str) -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::if_not_present(
        HeaderName::from_static(name),
        HeaderValue::from_static(value),
    )
}

/// Builds the full application router with every security layer applied.
pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(routes::health))
        .route("/openapi.json", get(routes::openapi_json))
        .route("/search", get(routes::catalog::search))
        .route("/compare", get(routes::catalog::compare))
        .route("/stats", get(routes::catalog::stats))
        .route("/close", get(routes::catalog::close))
        .route("/auth/google/start", get(routes::auth::start))
        .route("/auth/google/callback", get(routes::auth::callback))
        .route("/auth/logout", post(routes::auth::logout))
        .route("/me", get(routes::auth::me).delete(routes::auth::delete_me))
        .route(
            "/saved",
            get(routes::saved::list).post(routes::saved::create),
        )
        .route("/saved/{id}", delete(routes::saved::remove));

    let config = state.config.clone();
    let cors = HeaderValue::from_str(&config.web_origin)
        .ok()
        .map(|origin| {
            CorsLayer::new()
                .allow_origin(origin)
                .allow_credentials(true)
                .allow_methods([Method::GET, Method::POST, Method::DELETE])
                .allow_headers([header::CONTENT_TYPE])
                .max_age(Duration::from_secs(600))
        });

    let mut app = Router::new()
        .nest("/v1", api)
        .fallback(routes::not_found)
        .layer(from_fn_with_state(state.clone(), middleware::origin_guard))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .layer(header_layer("x-content-type-options", "nosniff"))
        .layer(header_layer("x-frame-options", "DENY"))
        .layer(header_layer("referrer-policy", "no-referrer"))
        .layer(header_layer(
            "content-security-policy",
            "default-src 'none'; frame-ancestors 'none'",
        ))
        .layer(header_layer(
            "strict-transport-security",
            "max-age=63072000; includeSubDomains",
        ))
        .layer(header_layer("cross-origin-resource-policy", "same-origin"))
        .layer(header_layer("cache-control", "no-store"))
        .with_state(state.clone());
    if let Some(cors) = cors {
        app = app.layer(cors);
    }
    app = app.layer(from_fn_with_state(state, middleware::proxy_guard));
    if let Some(rl) = config.rate_limit
        && let Some(governor) = GovernorConfigBuilder::default()
            .per_millisecond(rl.replenish_ms)
            .burst_size(rl.burst)
            .key_extractor(ClientIp {
                trust_header: config.proxy_secret.is_some(),
            })
            .finish()
    {
        app = app.layer(GovernorLayer::new(governor));
    }
    app.layer(TimeoutLayer::with_status_code(
        StatusCode::REQUEST_TIMEOUT,
        config.request_timeout,
    ))
    .layer(TraceLayer::new_for_http())
    .layer(PropagateRequestIdLayer::x_request_id())
    .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
}
