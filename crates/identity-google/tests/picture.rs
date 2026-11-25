#![allow(clippy::unwrap_used)]

use nirog_application::ports::IdentityProvider;
use nirog_identity_google::{GoogleConfig, GoogleIdentityProvider, Secret};
use serde_json::json;
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn picture_for(picture: serde_json::Value) -> Option<String> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": "at"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/userinfo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "sub": "123", "email": "a@example.com", "email_verified": true, "name": "Asha",
            "picture": picture,
        })))
        .mount(&server)
        .await;
    let base = Url::parse(&server.uri()).unwrap();
    let google = GoogleIdentityProvider::new(GoogleConfig {
        client_id: "cid".into(),
        client_secret: Secret::new("shh"),
        redirect_uri: "https://nirog.example/v1/auth/google/callback".into(),
        auth_endpoint: base.join("/auth").unwrap(),
        token_endpoint: base.join("/token").unwrap(),
        userinfo_endpoint: base.join("/userinfo").unwrap(),
    })
    .unwrap();
    google.exchange("code", "ver").await.unwrap().picture
}

/// The header shows the picture, so only Google's own https image host is trusted:
/// anything else could point the site at a tracker or a script URL.
#[tokio::test]
async fn keeps_only_https_pictures_on_googles_image_host() {
    let ok = "https://lh3.googleusercontent.com/a/ACg8oc=s96-c";
    assert_eq!(picture_for(json!(ok)).await.as_deref(), Some(ok));
    for bad in [
        json!("http://lh3.googleusercontent.com/a/x"),
        json!("https://evil.example/a.png"),
        json!("https://lh3.googleusercontent.com.evil.example/a"),
        json!("javascript:alert(1)"),
        json!(format!(
            "https://lh3.googleusercontent.com/{}",
            "a".repeat(600)
        )),
        json!(null),
    ] {
        assert_eq!(picture_for(bad.clone()).await, None, "{bad}");
    }
}
