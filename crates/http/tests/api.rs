//! End-to-end API tests: real router, real Postgres (sqlx::test), fake identity provider.
#![allow(clippy::unwrap_used, clippy::panic)] // test helpers: a failed unwrap is a failed test

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use nirog_application::ports::{
    IdentityProvider, OfferRepository, SourceInfo, SystemClock, VendorListings,
};
use nirog_application::sellers::DefaultSellerLinks;
use nirog_application::services::{
    AuthService, BuyService, CatalogService, CloseMatchService, PagesService, RecentService,
    SavedMatchService, StatsService,
};
use nirog_application::stock::{StockStatus, Vendor, VendorListing};
use nirog_application::{Identity, IdentityError};
use nirog_domain::parse::{describe_salts, detect_form, detect_release};
use nirog_domain::{MatchKey, Offer, OfferId, Pack, PackUnit, Paise, Salt, SourceKind};
use nirog_http::{AppState, HttpConfig, RateLimit, router};
use nirog_persistence::{
    PgBuyLinks, PgLoginStateRepository, PgMedicinePages, PgOfferRepository, PgRecentRepository,
    PgSavedMatchRepository, PgSessionRepository, PgStatsRepository, PgUserRepository,
    PgVendorListings,
};
use nirog_sources::aliases::BundledAliases;
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
                picture: Some("https://lh3.googleusercontent.com/a/asha=s96-c".into()),
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
        pages_token: None,
        rate_limit: None,
        request_timeout: Duration::from_secs(5),
    }
}

async fn app_with(pool: PgPool, config: HttpConfig) -> Router {
    let offers = Arc::new(PgOfferRepository::new(pool.clone()));
    offers
        .record(
            &SourceInfo {
                id: "jan-aushadhi".into(),
                kind: SourceKind::JanAushadhi,
                title: "jan-aushadhi".into(),
                url: None,
                observed_on: time::macros::date!(2026 - 11 - 20),
            },
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
        .record(
            &SourceInfo {
                id: "brands-2022".into(),
                kind: SourceKind::Brand,
                title: "brands-2022".into(),
                url: None,
                observed_on: time::macros::date!(2022 - 11 - 17),
            },
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
        catalog: CatalogService::new(
            offers.clone(),
            Arc::new(DefaultSellerLinks),
            Arc::new(BundledAliases::bundled()),
        )
        .with_stock(
            Arc::new(PgVendorListings::new(pool.clone())),
            Arc::new(SystemClock),
        )
        .with_sources(Arc::new(PgBuyLinks::new(pool.clone()))),
        buy: BuyService::new(
            Arc::new(PgBuyLinks::new(pool.clone())),
            Arc::new(SystemClock),
        ),
        auth: AuthService::new(
            Arc::new(PgUserRepository::new(pool.clone())),
            Arc::new(PgSessionRepository::new(pool.clone())),
            Arc::new(PgLoginStateRepository::new(pool.clone())),
            Arc::new(FakeGoogle),
            Arc::new(SystemClock),
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
        stats: StatsService::new(
            Arc::new(PgStatsRepository::new(pool)),
            Arc::new(SystemClock),
        ),
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
    let app = app_with(pool.clone(), config()).await;
    let res = send(&app, get("/v1/search?q=dolo")).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.header("cache-control"), "public, max-age=300");
    let key = res.json["groups"][0]["key"].as_str().unwrap().to_owned();
    assert_eq!(res.json["groups"][0]["matched"][0]["id"], "brand:1");

    assert_eq!(
        send(&app, get("/v1/search?q=a")).await.status,
        StatusCode::BAD_REQUEST
    );
    // A brand missing from every price list leads to its salt instead.
    let brand = send(&app, get("/v1/search?q=displin")).await;
    assert_eq!(
        brand.json["same_salt_as"]["brand"], "Disprin",
        "{}",
        brand.json
    );
    assert_eq!(brand.json["same_salt_as"]["salts_label"], "Aspirin 350mg");
    assert_eq!(brand.json["did_you_mean"].as_array().map(Vec::len), Some(0));

    let miss = send(&app, get("/v1/search?q=dollo")).await;
    assert!(miss.json["same_salt_as"].is_null(), "{}", miss.json);
    assert_eq!(
        miss.json["groups"].as_array().unwrap().len(),
        0,
        "{}",
        miss.json
    );
    assert!(
        miss.json["did_you_mean"][0]
            .as_str()
            .is_some_and(|n| n.starts_with("Dolo")),
        "{}",
        miss.json
    );

    let enc = key
        .replace('|', "%7C")
        .replace(':', "%3A")
        .replace('+', "%2B");
    let checked = time::OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .unwrap();
    PgVendorListings::new(pool)
        .upsert(&[VendorListing {
            offer_id: OfferId("brand:1".into()),
            vendor: Vendor::SourceB,
            url: "https://source-b.example/dolo-650-15".into(),
            status: StockStatus::InStock,
            price: Paise(2900),
            checked_at: checked,
        }])
        .await
        .unwrap();
    let cmp = send(
        &app,
        get(&format!("/v1/compare?key={enc}&searched=brand:1")),
    )
    .await;
    assert_eq!(cmp.status, StatusCode::OK);
    assert_eq!(cmp.json["offers"][0]["offer"]["source"], "jan_aushadhi");
    assert_eq!(cmp.json["offers"][0]["links"][0]["kind"], "store");
    // Brands carry no links to outside pharmacy pages.
    assert_eq!(cmp.json["offers"][1]["links"], serde_json::json!([]));
    assert_eq!(cmp.json["searched_index"], 1);
    assert_eq!(cmp.json["total_offers"], 2);
    assert_eq!(cmp.json["saving_per_pack_paise"], 1591);
    // Online stock rides along with each offer; offers nobody lists carry none.
    let pe = &cmp.json["offers"][1]["vendors"][0];
    assert_eq!(pe["vendor"], "source_b", "{}", cmp.json);
    assert_eq!(pe["status"], "in_stock");
    assert_eq!(pe["price_paise"], 2900);
    assert_eq!(
        pe["checked_at"],
        checked
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap()
    );
    assert!(pe.get("url").is_none(), "{pe}");
    assert_eq!(cmp.json["offers"][0]["vendors"], serde_json::json!([]));

    assert_eq!(
        send(
            &app,
            get("/v1/compare?key=tablet%7Cstandard%7Cx%3A2mg%7Cunit")
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
async fn buy_links_go_through_our_redirect_to_stored_https_pages_only(pool: PgPool) {
    let app = app_with(pool.clone(), config()).await;
    // Display names and search pages are stored privately, never in code.
    sqlx::query(
        "INSERT INTO price_sources (id, display_name, search_url_template) VALUES
         ('source_a', 'Pharmacy One', 'https://shop.example/search?q={query}'),
         ('source_b', 'Pharmacy Two', NULL)",
    )
    .execute(&pool)
    .await
    .unwrap();
    for bad in [
        "INSERT INTO price_sources VALUES ('source_x', 'Pharmacy X', NULL)",
        "INSERT INTO price_sources VALUES ('source_c', 'Pharmacy Three', 'http://shop.example/?q={query}')",
    ] {
        assert!(sqlx::query(bad).execute(&pool).await.is_err(), "{bad}");
    }
    let listing = |vendor, url: &str| VendorListing {
        offer_id: OfferId("brand:1".into()),
        vendor,
        url: url.into(),
        status: StockStatus::InStock,
        price: Paise(2900),
        checked_at: time::OffsetDateTime::now_utc(),
    };
    PgVendorListings::new(pool.clone())
        .upsert(&[
            listing(Vendor::SourceB, "https://shop.example/p/dolo-650"),
            listing(Vendor::SourceD, "http://shop.example/p/dolo-650"),
        ])
        .await
        .unwrap();

    let found = send(&app, get("/v1/search?q=dolo")).await;
    let key = urlencoding(found.json["groups"][0]["key"].as_str().unwrap());
    let cmp = send(&app, get(&format!("/v1/compare?key={key}"))).await;
    let brand = &cmp.json["offers"][1];
    assert_eq!(brand["offer"]["id"], "brand:1", "{}", cmp.json);
    assert_eq!(
        brand["vendors"][0]["display_name"], "Pharmacy Two",
        "{brand}"
    );
    assert_eq!(brand["vendors"][0]["buy_path"], "/v1/go/brand%3A1/source_b");
    // No stored name and no https page: a placeholder name and no link.
    assert_eq!(brand["vendors"][1]["display_name"], "Pharmacy D");
    assert!(brand["vendors"][1]["buy_path"].is_null(), "{brand}");
    // A source with a search page but no listing: a search link, through our redirect.
    assert_eq!(
        brand["links"],
        serde_json::json!([{"label": "Pharmacy One",
            "url": "/v1/go/brand%3A1/source_a", "kind": "online"}])
    );
    assert_eq!(cmp.json["offers"][0]["links"][0]["kind"], "store");
    assert_eq!(
        cmp.json["offers"][0]["links"].as_array().map(Vec::len),
        Some(2)
    );

    // Query parameters never choose the destination.
    let go = send(
        &app,
        get("/v1/go/brand%3A1/source_b?url=https://evil.example/&next=https://evil.example/"),
    )
    .await;
    assert_eq!(go.status, StatusCode::FOUND);
    assert_eq!(go.header("location"), "https://shop.example/p/dolo-650");
    assert_eq!(go.header("cache-control"), "no-store");
    assert_eq!(go.header("referrer-policy"), "no-referrer");
    let search = send(&app, get("/v1/go/brand:1/source_a")).await;
    assert_eq!(search.status, StatusCode::FOUND);
    assert_eq!(
        search.header("location"),
        "https://shop.example/search?q=Dolo%20650%20Tablet"
    );
    // A stored non-https page is refused, as is a source with nothing stored.
    for (uri, status) in [
        ("/v1/go/brand:1/source_d", StatusCode::NOT_FOUND),
        ("/v1/go/brand:1/source_c", StatusCode::NOT_FOUND),
        ("/v1/go/nope/source_a", StatusCode::NOT_FOUND),
        ("/v1/go/brand:1/source_x", StatusCode::BAD_REQUEST),
    ] {
        let res = send(&app, get(uri)).await;
        assert_eq!(res.status, status, "{uri}");
        assert!(res.header("location").is_empty(), "{uri}");
        assert!(res.json["error"].is_string(), "{uri}");
    }
    let clicks: Vec<(String, i64)> =
        sqlx::query_as("SELECT source_id, clicks FROM buy_clicks ORDER BY source_id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(
        clicks,
        [("source_a".to_owned(), 1), ("source_b".to_owned(), 1)]
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
    // The header shows the Google profile picture.
    assert_eq!(
        me.json["picture_url"],
        "https://lh3.googleusercontent.com/a/asha=s96-c"
    );

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
async fn me_lists_recently_opened_medicines(pool: PgPool) {
    let app = app_with(pool, config()).await;
    let session = sign_in(&app).await;
    let me = send(&app, with_cookie(get("/v1/me"), &session)).await;
    assert_eq!(me.json["recent"], json!([]));
    let key = send(&app, get("/v1/search?q=dolo")).await.json["groups"][0]["key"]
        .as_str()
        .unwrap()
        .to_owned();
    let body = json!({"key": key, "offer_id": "brand:1", "label": "Dolo 650"});

    for (cookie, origin, want) in [
        ("x=y", Some(WEB), StatusCode::UNAUTHORIZED),
        (
            session.as_str(),
            Some("https://evil.test"),
            StatusCode::FORBIDDEN,
        ),
    ] {
        let res = send(
            &app,
            write("POST", "/v1/me/recent", body.clone(), cookie, origin),
        )
        .await;
        assert_eq!(res.status, want);
    }
    for (bad, want) in [
        (
            json!({"key": "nope", "label": "x"}),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({"key": "tablet|standard|zz:2mg|unit", "label": "x"}),
            StatusCode::NOT_FOUND,
        ),
    ] {
        let res = send(
            &app,
            write("POST", "/v1/me/recent", bad, &session, Some(WEB)),
        )
        .await;
        assert_eq!(res.status, want);
    }

    for _ in 0..2 {
        let res = send(
            &app,
            write("POST", "/v1/me/recent", body.clone(), &session, Some(WEB)),
        )
        .await;
        assert_eq!(res.status, StatusCode::NO_CONTENT);
    }
    let me = send(&app, with_cookie(get("/v1/me"), &session)).await;
    let recent = me.json["recent"].as_array().unwrap();
    assert_eq!(recent.len(), 1, "opening twice keeps one entry");
    assert_eq!(recent[0]["key"], key.as_str());
    assert_eq!(recent[0]["offer_id"], "brand:1");
    assert_eq!(recent[0]["label"], "Dolo 650");

    let res = send(
        &app,
        write("DELETE", "/v1/me/recent", json!({}), &session, Some(WEB)),
    )
    .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let me = send(&app, with_cookie(get("/v1/me"), &session)).await;
    assert_eq!(me.json["recent"], json!([]));
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

    send(
        &app,
        write(
            "POST",
            "/v1/me/recent",
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
    let left: i64 = sqlx::query_scalar("SELECT (SELECT count(*) FROM users) + (SELECT count(*) FROM sessions) + (SELECT count(*) FROM saved_matches) + (SELECT count(*) FROM recent_medicines)")
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

#[sqlx::test(migrator = "nirog_persistence::MIGRATOR")]
async fn stats_sum_up_the_price_data(pool: PgPool) {
    let app = app_with(pool, config()).await;
    let res = send(&app, get("/v1/stats")).await;
    assert_eq!(res.status, StatusCode::OK);
    // Counts change only on import: browsers keep 5 min, the edge cache 1 hour.
    assert_eq!(
        res.header("cache-control"),
        "public, max-age=300, s-maxage=3600"
    );
    // Dolo at 206.07 paise a tablet has a 100-paise Jan Aushadhi substitute: 51.47% less.
    assert_eq!(
        res.json,
        json!({
            "medicines": 2,
            "jan_aushadhi": 1,
            "with_cheaper_substitute": 1,
            "median_saving_bps": 5147,
            "prices_updated_on": "2026-11-20"
        })
    );
}

#[sqlx::test(migrator = "nirog_persistence::MIGRATOR")]
async fn close_matches_say_what_differs(pool: PgPool) {
    let app = app_with(pool.clone(), config()).await;
    // A 500mg paracetamol: one strength away from the seeded 650mg medicines.
    let salts = [Salt::mg("paracetamol", 500_000)];
    let mut o = offer(
        "brand:500",
        SourceKind::Brand,
        "Calpol 500 Tablet",
        1500,
        15,
    );
    o.key = MatchKey::new(
        &salts,
        detect_form("tablet"),
        detect_release("tablet"),
        PackUnit::Unit,
    )
    .unwrap();
    o.salts_label = describe_salts(&salts);
    PgOfferRepository::new(pool)
        .record(
            &SourceInfo {
                id: "extra".into(),
                kind: SourceKind::Brand,
                title: "extra".into(),
                url: None,
                observed_on: time::macros::date!(2026 - 11 - 20),
            },
            &[o],
        )
        .await
        .unwrap();
    let key = offer("x", SourceKind::Brand, "x", 1, 1).key;
    let url = format!("/v1/close?key={}", urlencoding(key.as_str()));
    let res = send(&app, get(&url)).await;
    assert_eq!(res.status, StatusCode::OK);
    let m = &res.json["matches"];
    assert_eq!(m.as_array().map(Vec::len), Some(1));
    assert_eq!(m[0]["cheapest"]["name"], "Calpol 500 Tablet");
    assert_eq!(
        m[0]["differences"],
        json!(["Paracetamol 500mg instead of 650mg"])
    );
    assert_eq!(
        send(&app, get("/v1/close?key=bad")).await.status,
        StatusCode::BAD_REQUEST
    );
}

fn urlencoding(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[sqlx::test(migrator = "nirog_persistence::MIGRATOR")]
async fn pages_walk_every_medicine_with_a_substitute(pool: PgPool) {
    let app = app_with(pool, config()).await;
    let res = send(&app, get("/v1/pages")).await;
    assert_eq!(res.status, StatusCode::OK);
    // The whole catalogue is never kept by a shared cache, where anyone could read it.
    assert_eq!(res.header("cache-control"), "no-store");
    let pages = res.json["pages"].as_array().unwrap();
    assert_eq!(pages.len(), 1);
    let c = &pages[0]["comparison"];
    assert_eq!(c["total_offers"], 2);
    assert_eq!(c["offers"][0]["offer"]["id"], "ja:1", "cheapest first");
    assert_eq!(pages[0]["names"].as_array().unwrap().len(), 2);
    assert!(
        res.json["next"].is_null(),
        "one short batch is the whole walk"
    );

    let after = c["key"]
        .as_str()
        .unwrap()
        .replace('|', "%7C")
        .replace(':', "%3A")
        .replace('+', "%2B");
    let rest = send(&app, get(&format!("/v1/pages?after={after}"))).await;
    assert_eq!(rest.json["pages"].as_array().unwrap().len(), 0);
    assert_eq!(
        send(&app, get("/v1/pages?after=bad")).await.status,
        StatusCode::BAD_REQUEST
    );
}

#[sqlx::test(migrator = "nirog_persistence::MIGRATOR")]
async fn pages_answer_only_the_deploy_when_a_token_is_set(pool: PgPool) {
    let token = "t".repeat(40);
    let cfg = HttpConfig {
        pages_token: Some(token.clone()),
        ..config()
    };
    let app = app_with(pool, cfg).await;
    let pages = |auth: Option<String>| {
        let mut b = Request::get("/v1/pages");
        if let Some(a) = auth {
            b = b.header(header::AUTHORIZATION, a);
        }
        b.body(Body::empty()).unwrap()
    };
    for bad in [None, Some("Bearer wrong".into()), Some(token.clone())] {
        let res = send(&app, pages(bad)).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN);
        assert_eq!(res.json["error"], "forbidden");
    }
    let ok = send(&app, pages(Some(format!("Bearer {token}")))).await;
    assert_eq!(ok.status, StatusCode::OK);
    assert_eq!(ok.header("cache-control"), "no-store");
    // Everything else stays public.
    assert_eq!(send(&app, get("/v1/stats")).await.status, StatusCode::OK);
}
