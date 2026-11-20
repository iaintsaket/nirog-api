//! Google sign-in using the OAuth 2.0 authorization code flow with PKCE.
//! The code is exchanged server-side, then the profile is read from the
//! userinfo endpoint over TLS; only verified emails are accepted.

use std::time::Duration;

use async_trait::async_trait;
use nirog_application::ports::IdentityProvider;
use nirog_application::{Identity, IdentityError};
use serde::Deserialize;
use url::Url;

pub const PROVIDER: &str = "google";

/// Client secret wrapper that never prints its value.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

#[derive(Debug, Clone)]
pub struct GoogleConfig {
    pub client_id: String,
    pub client_secret: Secret,
    /// Must exactly match a redirect URI registered in Google Cloud console.
    pub redirect_uri: String,
    pub auth_endpoint: Url,
    pub token_endpoint: Url,
    pub userinfo_endpoint: Url,
}

impl GoogleConfig {
    /// Google's production endpoints.
    pub fn new(
        client_id: String,
        client_secret: Secret,
        redirect_uri: String,
    ) -> Result<Self, url::ParseError> {
        Ok(Self {
            client_id,
            client_secret,
            redirect_uri,
            auth_endpoint: Url::parse("https://accounts.google.com/o/oauth2/v2/auth")?,
            token_endpoint: Url::parse("https://oauth2.googleapis.com/token")?,
            userinfo_endpoint: Url::parse("https://openidconnect.googleapis.com/v1/userinfo")?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct GoogleIdentityProvider {
    config: GoogleConfig,
    http: reqwest::Client,
}

impl GoogleIdentityProvider {
    pub fn new(config: GoogleConfig) -> Result<Self, IdentityError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| IdentityError::Unavailable(e.to_string()))?;
        Ok(Self { config, http })
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct UserInfo {
    sub: String,
    email: Option<String>,
    #[serde(default)]
    email_verified: bool,
    name: Option<String>,
}

/// 4xx means Google rejected what we sent; anything else is an outage.
fn classify(status: reqwest::StatusCode, what: &str) -> IdentityError {
    if status.is_client_error() {
        IdentityError::Rejected(format!("{what}: {status}"))
    } else {
        IdentityError::Unavailable(format!("{what}: {status}"))
    }
}

fn unavailable(e: reqwest::Error) -> IdentityError {
    IdentityError::Unavailable(e.without_url().to_string())
}

#[async_trait]
impl IdentityProvider for GoogleIdentityProvider {
    fn authorize_url(&self, state: &str, pkce_challenge: &str) -> String {
        let mut url = self.config.auth_endpoint.clone();
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.config.client_id)
            .append_pair("redirect_uri", &self.config.redirect_uri)
            .append_pair("scope", "openid email profile")
            .append_pair("state", state)
            .append_pair("code_challenge", pkce_challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("prompt", "select_account");
        url.into()
    }

    async fn exchange(&self, code: &str, pkce_verifier: &str) -> Result<Identity, IdentityError> {
        let res = self
            .http
            .post(self.config.token_endpoint.clone())
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("code_verifier", pkce_verifier),
                ("client_id", self.config.client_id.as_str()),
                ("client_secret", self.config.client_secret.0.as_str()),
                ("redirect_uri", self.config.redirect_uri.as_str()),
            ])
            .send()
            .await
            .map_err(unavailable)?;
        if !res.status().is_success() {
            return Err(classify(res.status(), "token exchange"));
        }
        let token: TokenResponse = res
            .json()
            .await
            .map_err(|_| IdentityError::Unavailable("bad token response".into()))?;

        let res = self
            .http
            .get(self.config.userinfo_endpoint.clone())
            .bearer_auth(&token.access_token)
            .send()
            .await
            .map_err(unavailable)?;
        if !res.status().is_success() {
            return Err(classify(res.status(), "userinfo"));
        }
        let info: UserInfo = res
            .json()
            .await
            .map_err(|_| IdentityError::Unavailable("bad userinfo response".into()))?;
        let email = info
            .email
            .filter(|_| info.email_verified)
            .ok_or_else(|| IdentityError::Rejected("email not verified".into()))?;
        if info.sub.is_empty() || info.sub.len() > 255 {
            return Err(IdentityError::Rejected("bad subject".into()));
        }
        let name = info.name.unwrap_or_default().chars().take(200).collect();
        Ok(Identity {
            provider: PROVIDER,
            subject: info.sub,
            email,
            name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn provider(server: &MockServer) -> GoogleIdentityProvider {
        let base = Url::parse(&server.uri()).unwrap_or_else(|_| unreachable!());
        let at = |p: &str| base.join(p).unwrap_or_else(|_| unreachable!());
        GoogleIdentityProvider::new(GoogleConfig {
            client_id: "cid".into(),
            client_secret: Secret::new("shh"),
            redirect_uri: "https://nirog.example/v1/auth/google/callback".into(),
            auth_endpoint: at("/auth"),
            token_endpoint: at("/token"),
            userinfo_endpoint: at("/userinfo"),
        })
        .unwrap_or_else(|_| unreachable!())
    }

    async fn token_ok(server: &MockServer) {
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("code_verifier=ver"))
            .and(body_string_contains("grant_type=authorization_code"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"access_token": "at", "id_token": "x"})),
            )
            .mount(server)
            .await;
    }

    async fn userinfo(server: &MockServer, status: u16, body: serde_json::Value) {
        Mock::given(method("GET"))
            .and(path("/userinfo"))
            .and(header("authorization", "Bearer at"))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn authorize_url_carries_pkce_and_state() {
        let server = MockServer::start().await;
        let url = Url::parse(&provider(&server).authorize_url("st", "ch"))
            .unwrap_or_else(|_| unreachable!());
        let q: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(q["state"], "st");
        assert_eq!(q["code_challenge"], "ch");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["scope"], "openid email profile");
    }

    #[tokio::test]
    async fn exchange_returns_verified_identity() -> Result<(), IdentityError> {
        let server = MockServer::start().await;
        token_ok(&server).await;
        userinfo(
            &server,
            200,
            json!({"sub": "123", "email": "a@example.com", "email_verified": true, "name": "Asha"}),
        )
        .await;
        let id = provider(&server).exchange("code", "ver").await?;
        assert_eq!(
            id,
            Identity {
                provider: "google",
                subject: "123".into(),
                email: "a@example.com".into(),
                name: "Asha".into()
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn unverified_email_is_rejected() {
        let server = MockServer::start().await;
        token_ok(&server).await;
        userinfo(
            &server,
            200,
            json!({"sub": "123", "email": "a@example.com", "email_verified": false}),
        )
        .await;
        assert!(matches!(
            provider(&server).exchange("code", "ver").await,
            Err(IdentityError::Rejected(_))
        ));
    }

    #[tokio::test]
    async fn token_errors_are_classified() {
        let server = MockServer::start().await;
        Mock::given(path("/token"))
            .respond_with(ResponseTemplate::new(400))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        let p = provider(&server);
        assert!(matches!(
            p.exchange("bad", "ver").await,
            Err(IdentityError::Rejected(_))
        ));
        Mock::given(path("/token"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        assert!(matches!(
            p.exchange("c", "ver").await,
            Err(IdentityError::Unavailable(_))
        ));
    }

    #[tokio::test]
    async fn userinfo_failure_and_garbage_are_errors() {
        let server = MockServer::start().await;
        token_ok(&server).await;
        userinfo(&server, 500, json!({})).await;
        assert!(matches!(
            provider(&server).exchange("c", "ver").await,
            Err(IdentityError::Unavailable(_))
        ));

        let server = MockServer::start().await;
        token_ok(&server).await;
        userinfo(&server, 200, json!({"nope": 1})).await;
        assert!(matches!(
            provider(&server).exchange("c", "ver").await,
            Err(IdentityError::Unavailable(_))
        ));
    }

    #[test]
    fn secret_is_redacted_and_prod_config_parses() {
        assert_eq!(format!("{:?}", Secret::new("shh")), "Secret(<redacted>)");
        assert!(GoogleConfig::new("c".into(), Secret::new("s"), "r".into()).is_ok());
    }
}
