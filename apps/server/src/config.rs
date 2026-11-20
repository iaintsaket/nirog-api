use std::time::Duration;

use anyhow::{Context, Result, bail};
use nirog_http::{HttpConfig, RateLimit};

/// Everything the server needs, read once at start-up. Secrets never reach logs:
/// the Debug output below redacts them.
pub struct Config {
    pub port: u16,
    pub database_url: String,
    pub db_max_connections: u32,
    pub google_client_id: String,
    pub google_client_secret: String,
    pub google_redirect_uri: String,
    pub http: HttpConfig,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("port", &self.port)
            .field("db_max_connections", &self.db_max_connections)
            .field("google_redirect_uri", &self.google_redirect_uri)
            .field("web_origin", &self.http.web_origin)
            .field("secure_cookies", &self.http.secure_cookies)
            .field("proxy_secret_set", &self.http.proxy_secret.is_some())
            .finish_non_exhaustive()
    }
}

impl Config {
    /// `get` looks up a variable; injected so tests need not touch the process env.
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let req = |k: &str| {
            get(k)
                .filter(|v| !v.trim().is_empty())
                .with_context(|| format!("{k} is not set"))
        };
        let num = |k: &str, default: u64| -> Result<u64> {
            get(k).map_or(Ok(default), |v| {
                v.parse().with_context(|| format!("{k} must be a number"))
            })
        };

        let web_origin = req("WEB_ORIGIN")?.trim_end_matches('/').to_owned();
        let secure_cookies = get("SECURE_COOKIES").is_none_or(|v| v != "false");
        if secure_cookies && !web_origin.starts_with("https://") {
            bail!("WEB_ORIGIN must be https:// unless SECURE_COOKIES=false");
        }
        let proxy_secret = get("PROXY_SECRET").filter(|s| !s.is_empty());
        if proxy_secret.as_ref().is_some_and(|s| s.len() < 32) {
            bail!("PROXY_SECRET must be at least 32 characters");
        }
        let google_redirect_uri = get("GOOGLE_REDIRECT_URI")
            .unwrap_or_else(|| format!("{web_origin}/v1/auth/google/callback"));

        Ok(Self {
            port: u16::try_from(num("PORT", 8080)?).context("PORT out of range")?,
            database_url: req("DATABASE_URL")?,
            db_max_connections: u32::try_from(num("DB_MAX_CONNECTIONS", 5)?)
                .context("DB_MAX_CONNECTIONS out of range")?,
            google_client_id: req("GOOGLE_CLIENT_ID")?,
            google_client_secret: req("GOOGLE_CLIENT_SECRET")?,
            google_redirect_uri,
            http: HttpConfig {
                web_origin,
                secure_cookies,
                proxy_secret,
                // 5 requests/second sustained per client, bursts of 30.
                rate_limit: Some(RateLimit {
                    replenish_ms: 200,
                    burst: 30,
                }),
                request_timeout: Duration::from_secs(15),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect();
        move |k| map.get(k).cloned()
    }

    const BASE: &[(&str, &str)] = &[
        ("WEB_ORIGIN", "https://nirog.pages.dev/"),
        ("DATABASE_URL", "postgres://x"),
        ("GOOGLE_CLIENT_ID", "id"),
        ("GOOGLE_CLIENT_SECRET", "very-secret"),
    ];

    #[test]
    fn defaults_and_derived_values() -> Result<()> {
        let c = Config::from_env(env(BASE))?;
        assert_eq!(c.port, 8080);
        assert_eq!(c.http.web_origin, "https://nirog.pages.dev");
        assert_eq!(
            c.google_redirect_uri,
            "https://nirog.pages.dev/v1/auth/google/callback"
        );
        assert!(c.http.secure_cookies);
        assert!(!format!("{c:?}").contains("very-secret"));
        Ok(())
    }

    #[test]
    fn rejects_missing_insecure_or_weak_settings() {
        assert!(Config::from_env(env(&BASE[1..])).is_err());
        let mut http = BASE.to_vec();
        http[0] = ("WEB_ORIGIN", "http://localhost:3000");
        assert!(Config::from_env(env(&http)).is_err());
        http.push(("SECURE_COOKIES", "false"));
        assert!(Config::from_env(env(&http)).is_ok());
        let mut weak = BASE.to_vec();
        weak.push(("PROXY_SECRET", "short"));
        assert!(Config::from_env(env(&weak)).is_err());
        let mut port = BASE.to_vec();
        port.push(("PORT", "99999"));
        assert!(Config::from_env(env(&port)).is_err());
    }
}
