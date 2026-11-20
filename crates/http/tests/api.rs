//! End-to-end API tests: real router, real Postgres (sqlx::test), fake identity provider.
#![allow(clippy::unwrap_used, clippy::panic)] // test helpers: a failed unwrap is a failed test

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use nirog_application::ports::{IdentityProvider, OfferRepository, SystemClock};
use nirog_application::sellers::DefaultSellerLinks;
use nirog_application::services::{AuthService, CatalogService, SavedMatchService};
use nirog_application::{Identity, IdentityError};
use nirog_domain::parse::{describe_salts, detect_form, detect_release};
use nirog_domain::{MatchKey, Offer, OfferId, Pack, PackUnit, Paise, Salt, SourceKind};
use nirog_http::{AppState, HttpConfig, RateLimit, router};
use nirog_persistence::{
    PgLoginStateRepository, PgOfferRepository, PgSavedMatchRepository, PgSessionRepository,
    PgUserRepository,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

const WEB: &str = "https://nirog.test";

struct FakeGoogle;

#[async_trait]
impl IdentityProvider for FakeGoogle {
    fn authorize_url(&self, state: &str, challenge: &str) -> String {
        format!("https://idp.test/auth?state={state}&code_challenge={challenge}")
    }
    async fn exchange(&self, code: &str, _verifier: &str) -> Result<Identity, IdentityError> {
        match code {
            "good" => Ok(Identity {
                provider: "google",
                subject: "g-1".into(),
                email: "asha@example.com".into(),
                name: "Asha".into(),
            }),
            _ => Err(IdentityError::Rejected("bad code".into())),
        }
    }
}

fn offer(id: &str, source: SourceKind, name: &str, paise: i64, count: u32) -> Offer {
    let salts = [Salt::mg("paracetamol", 650_000)];
    Offer {
        id: OfferId(id.into()),
        source,
        name: name.into(),
        manufacturer: "Maker".into(),
        price: Paise(paise),
        pack: Pack {
            count,
            unit: PackUnit::Unit,
        },
        key: MatchKey::new(
            &salts,
            detect_form("tablet"),
            detect_release("tablet"),
            PackUnit::Unit,
        )
        .unwrap(),
        salts_label: describe_salts(&salts),
    }
}

fn config() -> HttpConfig {
    HttpConfig {
        web_origin: WEB.into(),
        secure_cookies: true,
        proxy_secret: None,
        rate_limit: None,
        request_timeout: Duration::from_secs(5),
    }
}

async fn app_with(pool: PgPool, config: HttpConfig) -> Router {
    let offers = Arc::new(PgOfferRepository::new(pool.clone()));
    offers
        .replace_source(
            SourceKind::JanAushadhi,
            &[offer(
                "ja:1",
                SourceKind::JanAushadhi,
                "Paracetamol Tablets IP 650 mg",
                1500,
                15,
            )],
        )
        .await
        .unwrap();
    offers
        .replace_source(
            SourceKind::Brand,
            &[offer(
                "brand:1",
                SourceKind::Brand,
                "Dolo 650 Tablet",
                3091,
                15,
            )],
        )
        .await
        .unwrap();
    let state = AppState {
        catalog: CatalogService::new(offers.clone(), Arc::new(DefaultSellerLinks)),
        auth: AuthService::new(
            Arc::new(PgUserRepository::new(pool.clone())),
            Arc::new(PgSessionRepository::new(pool.clone())),
            Arc::new(PgLoginStateRepository::new(pool.clone())),
            Arc::new(FakeGoogle),
            Arc::new(SystemClock),
            chrono::Duration::days(30),
        ),
        saved: SavedMatchService::new(Arc::new(PgSavedMatchRepository::new(pool)), offers),
        config: Arc::new(config),
    };
    router(state)
}

struct Res {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    json: Value,
}

impl Res {
    fn header(&self, name: &str) -> &str {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
    }
    fn cookies(&self) -> Vec<String> {
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().to_owned())
            .collect()
    }
}

async fn send(app: &Router, req: Request<Body>) -> Res {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Res {
        status,
        headers,
        json,
    }
}

fn get(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).unwrap()
}

fn with_cookie(mut req: Request<Body>, cookie: &str) -> Request<Body> {
    req.headers_mut()
        .insert(header::COOKIE, cookie.parse().unwrap());
    req
}

fn write(
    method: &str,
    uri: &str,
    body: Value,
    cookie: &str,
    origin: Option<&str>,
) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(o) = origin {
        b = b.header(header::ORIGIN, o);
    }
    with_cookie(b.body(Body::from(body.to_string())).unwrap(), cookie)
}

/// Runs the full sign-in dance and returns the session cookie pair ("name=value").
async fn sign_in(app: &Router) -> String {
    let start = send(app, get("/v1/auth/google/start")).await;
    assert_eq!(start.status, StatusCode::SEE_OTHER);
    let location = start.header("location").to_owned();
    let state = location
        .split("state=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .to_owned();
    let login_cookie = start.cookies()[0].split(';').next().unwrap().to_owned();
    assert!(login_cookie.starts_with("__Host-nirog_login="));

    let cb = send(
        app,
        with_cookie(
            get(&format!("/v1/auth/google/callback?code=good&state={state}")),
            &login_cookie,
        ),
    )
    .await;
    assert_eq!(cb.status, StatusCode::SEE_OTHER);
    assert_eq!(cb.header("location"), format!("{WEB}/saved"));
    let session = cb
        .cookies()
        .into_iter()
        .find(|c| c.starts_with("__Host-nirog_session="))
        .unwrap();
    assert!(
        session.contains("HttpOnly")
            && session.contains("Secure")
            && session.contains("SameSite=Lax")
    );
    session.split(';').next().unwrap().to_owned()
}

#[sqlx::test(migrator = "nirog_persistence::MIGRATOR")]
async fn health_has_security_headers_and_request_id(pool: PgPool) {
    let app = app_with(pool, config()).await;
    let res = send(&app, get("/v1/health")).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.json, json!({"status": "ok"}));
    assert_eq!(res.header("x-content-type-options"), "nosniff");
    assert_eq!(res.header("x-frame-options"), "DENY");
    assert_eq!(res.header("cache-control"), "no-store");
    assert!(
        res.header("content-security-policy")
            .contains("default-src 'none'")
    );
    assert!(!res.header("x-request-id").is_empty());

    let missing = send(&app, get("/nope")).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(missing.json["error"], "not_found");
    assert_eq!(
        send(&app, get("/v1/openapi.json")).await.json["info"]["title"],
        "Nirog API"
    );
}

#[sqlx::test(migrator = "nirog_persistence::MIGRATOR")]
async fn search_and_compare_find_the_cheapest(pool: PgPool) {
    let app = app_with(pool, config()).await;
    let res = send(&app, get("/v1/search?q=dolo")).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.header("cache-control"), "public, max-age=300");
    let key = res.json["groups"][0]["key"].as_str().unwrap().to_owned();
    assert_eq!(res.json["groups"][0]["matched"][0]["id"], "brand:1");

    assert_eq!(
        send(&app, get("/v1/search?q=a")).await.status,
        StatusCode::BAD_REQUEST
    );

    let enc = key
        .replace('|', "%7C")
        .replace(':', "%3A")
        .replace('+', "%2B");
    let cmp = send(
        &app,
        get(&format!("/v1/compare?key={enc}&searched=brand:1")),
    )
    .await;
    assert_eq!(cmp.status, StatusCode::OK);
    assert_eq!(cmp.json["offers"][0]["offer"]["source"], "jan_aushadhi");
    assert_eq!(cmp.json["offers"][0]["links"][0]["kind"], "store");
    assert_eq!(cmp.json["offers"][1]["links"][0]["kind"], "online");
    assert_eq!(cmp.json["searched_index"], 1);
    assert_eq!(cmp.json["saving_per_pack_paise"], 1591);

    assert_eq!(
        send(
            &app,
            get("/v1/compare?key=tablet%7Cstandard%7Cx%3A1mg%7Cunit")
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(&app, get("/v1/compare?key=bad")).await.status,
        StatusCode::BAD_REQUEST
    );
}

#[sqlx::test(migrator = "nirog_persistence::MIGRATOR")]
async fn sign_in_me_logout(pool: PgPool) {
    let app = app_with(pool, config()).await;
    assert_eq!(
        send(&app, get("/v1/me")).await.status,
        StatusCode::UNAUTHORIZED
    );
    let session = sign_in(&app).await;

    let me = send(&app, with_cookie(get("/v1/me"), &session)).await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(me.json["email"], "asha@example.com");

    // Logout is a write: it needs the web origin.
    assert_eq!(
        send(
            &app,
            write("POST", "/v1/auth/logout", json!({}), &session, None)
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    let out = send(
        &app,
        write("POST", "/v1/auth/logout", json!({}), &session, Some(WEB)),
    )
    .await;
    assert_eq!(out.status, StatusCode::NO_CONTENT);
    assert!(out.cookies()[0].contains("Max-Age=0"));
    assert_eq!(
        send(&app, with_cookie(get("/v1/me"), &session))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrator = "nirog_persistence::MIGRATOR")]
async fn callback_rejects_unbound_mismatched_or_failed_logins(pool: PgPool) {
    let app = app_with(pool, config()).await;
    let start = send(&app, get("/v1/auth/google/start")).await;
    let location = start.header("location").to_owned();
    let state = location
        .split("state=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .to_owned();
    let login_cookie = start.cookies()[0].split(';').next().unwrap().to_owned();
    let failed = format!("{WEB}/signin?error=login_failed");

    // No login cookie: a callback started in another browser (login CSRF).
    let res = send(
        &app,
        get(&format!("/v1/auth/google/callback?code=good&state={state}")),
    )
    .await;
    assert_eq!(res.header("location"), failed);
    // Cookie for a different state.
    let res = send(
        &app,
        with_cookie(
            get("/v1/auth/google/callback?code=good&state=other"),
            &login_cookie,
        ),
    )
    .await;
    assert_eq!(res.header("location"), failed);
    // Provider error or rejected code.
    let res = send(
        &app,
        with_cookie(
            get(&format!(
                "/v1/auth/google/callback?error=access_denied&state={state}"
            )),
            &login_cookie,
        ),
    )
    .await;
    assert_eq!(res.header("location"), failed);
    let res = send(
        &app,
        with_cookie(
            get(&format!("/v1/auth/google/callback?code=bad&state={state}")),
            &login_cookie,
        ),
    )
    .await;
    assert_eq!(res.header("location"), failed);
    // The state was consumed by the rejected attempt: it cannot be replayed.
    let res = send(
        &app,
        with_cookie(
            get(&format!("/v1/auth/google/callback?code=good&state={state}")),
            &login_cookie,
        ),
    )
    .await;
    assert_eq!(res.header("location"), failed);
}

#[sqlx::test(migrator = "nirog_persistence::MIGRATOR")]
async fn saved_matches_crud_with_origin_check(pool: PgPool) {
    let app = app_with(pool, config()).await;
    let session = sign_in(&app).await;
    let key = send(&app, get("/v1/search?q=dolo")).await.json["groups"][0]["key"]
        .as_str()
        .unwrap()
        .to_owned();
    let body = json!({"key": key, "offer_id": "brand:1", "label": "Dolo for Dad"});

    assert_eq!(
        send(
            &app,
            write("POST", "/v1/saved", body.clone(), &session, None)
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(
            &app,
            write(
                "POST",
                "/v1/saved",
                body.clone(),
                &session,
                Some("https://evil.test")
            )
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(
            &app,
            write("POST", "/v1/saved", body.clone(), "x=y", Some(WEB))
        )
        .await
        .status,
        StatusCode::UNAUTHORIZED
    );

    let created = send(&app, write("POST", "/v1/saved", body, &session, Some(WEB))).await;
    assert_eq!(created.status, StatusCode::CREATED);
    let id = created.json["id"].as_str().unwrap().to_owned();

    let unknown = json!({"key": key, "label": "x", "extra": 1});
    assert_eq!(
        send(
            &app,
            write("POST", "/v1/saved", unknown, &session, Some(WEB))
        )
        .await
        .status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let empty = json!({"key": key, "label": " "});
    assert_eq!(
        send(&app, write("POST", "/v1/saved", empty, &session, Some(WEB)))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );

    let list = send(&app, with_cookie(get("/v1/saved"), &session)).await;
    assert_eq!(list.json.as_array().unwrap().len(), 1);
    assert_eq!(list.json[0]["label"], "Dolo for Dad");

    let other = "/v1/saved/00000000-0000-0000-0000-000000000000";
    assert_eq!(
        send(&app, write("DELETE", other, json!({}), &session, Some(WEB)))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let del = send(
        &app,
        write(
            "DELETE",
            &format!("/v1/saved/{id}"),
            json!({}),
            &session,
            Some(WEB),
        ),
    )
    .await;
    assert_eq!(del.status, StatusCode::NO_CONTENT);

    let huge = json!({"key": key, "label": "x".repeat(20_000)});
    assert_eq!(
        send(&app, write("POST", "/v1/saved", huge, &session, Some(WEB)))
            .await
            .status,
        StatusCode::PAYLOAD_TOO_LARGE
    );
}

#[sqlx::test(migrator = "nirog_persistence::MIGRATOR")]
async fn delete_account_erases_everything(pool: PgPool) {
    let app = app_with(pool.clone(), config()).await;
    let session = sign_in(&app).await;
    let key = send(&app, get("/v1/search?q=dolo")).await.json["groups"][0]["key"]
        .as_str()
        .unwrap()
        .to_owned();
    send(
        &app,
        write(
            "POST",
            "/v1/saved",
            json!({"key": key, "label": "x"}),
            &session,
            Some(WEB),
        ),
    )
    .await;

    let res = send(
        &app,
        write("DELETE", "/v1/me", json!({}), &session, Some(WEB)),
    )
    .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    assert_eq!(
        send(&app, with_cookie(get("/v1/me"), &session))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let left: i64 = sqlx::query_scalar("SELECT (SELECT count(*) FROM users) + (SELECT count(*) FROM sessions) + (SELECT count(*) FROM saved_matches)")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[sqlx::test(migrator = "nirog_persistence::MIGRATOR")]
async fn proxy_secret_blocks_direct_access_and_rate_limit_applies(pool: PgPool) {
    let cfg = HttpConfig {
        proxy_secret: Some("s3cret".into()),
        rate_limit: Some(RateLimit {
            replenish_ms: 60_000,
            burst: 2,
        }),
        ..config()
    };
    let app = app_with(pool, cfg).await;
    let req = |key: Option<&str>, ip: &str| {
        let mut b = Request::get("/v1/search?q=dolo").header("x-nirog-client-ip", ip);
        if let Some(k) = key {
            b = b.header("x-nirog-proxy-key", k);
        }
        b.body(Body::empty()).unwrap()
    };
    assert_eq!(
        send(&app, req(None, "1.1.1.1")).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(&app, req(Some("wrong"), "1.1.1.1")).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(&app, req(Some("s3cret"), "1.1.1.1")).await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
    // Another client has its own bucket.
    assert_eq!(
        send(&app, req(Some("s3cret"), "2.2.2.2")).await.status,
        StatusCode::OK
    );
    let health = Request::get("/v1/health")
        .header("x-nirog-client-ip", "3.3.3.3")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, health).await.status, StatusCode::OK);
}

#[test]
fn openapi_json_matches_committed_file() {
    use utoipa::OpenApi;
    let generated = nirog_http::ApiDoc::openapi().to_pretty_json().unwrap() + "\n";
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../openapi.json");
    if std::env::var_os("UPDATE_OPENAPI").is_some() {
        std::fs::write(path, &generated).unwrap();
    }
    let committed = std::fs::read_to_string(path).unwrap_or_default();
    assert!(
        committed == generated,
        "openapi.json is stale: run `UPDATE_OPENAPI=1 cargo test -p nirog-http --test api openapi`"
    );
}
