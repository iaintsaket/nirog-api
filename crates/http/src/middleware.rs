use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use tower_governor::GovernorError;
use tower_governor::key_extractor::KeyExtractor;

use crate::error::body;
use crate::{AppState, CLIENT_IP_HEADER, PROXY_KEY_HEADER};

/// Equal-time comparison so the proxy secret cannot be guessed byte by byte.
pub(crate) fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Writes (anything but GET/HEAD/OPTIONS) must come from the website's exact origin.
/// With SameSite=Lax cookies this blocks cross-site request forgery.
pub async fn origin_guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    let origin_ok = req
        .headers()
        .get(header::ORIGIN)
        .is_some_and(|o| o.as_bytes() == state.config.web_origin.as_bytes());
    if safe || origin_ok {
        next.run(req).await
    } else {
        body(
            StatusCode::FORBIDDEN,
            "forbidden",
            "Cross-origin request refused",
        )
    }
}

/// When a proxy secret is configured, only the web proxy may call the API.
pub async fn proxy_guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let Some(secret) = state.config.proxy_secret.as_deref() else {
        return next.run(req).await;
    };
    let presented = req
        .headers()
        .get(PROXY_KEY_HEADER)
        .is_some_and(|v| constant_eq(v.as_bytes(), secret.as_bytes()));
    if presented || req.uri().path() == "/v1/health" {
        next.run(req).await
    } else {
        body(
            StatusCode::FORBIDDEN,
            "forbidden",
            "Direct access is not allowed",
        )
    }
}

/// Rate-limit key: the client IP reported by the trusted proxy, else the TCP peer.
#[derive(Debug, Clone, Copy)]
pub struct ClientIp {
    /// Trust `x-nirog-client-ip`; only safe when the proxy secret is enforced.
    pub trust_header: bool,
}

impl KeyExtractor for ClientIp {
    type Key = IpAddr;

    fn extract<T>(&self, req: &axum::http::Request<T>) -> Result<IpAddr, GovernorError> {
        let from_header = self
            .trust_header
            .then(|| req.headers().get(CLIENT_IP_HEADER))
            .flatten()
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse().ok());
        from_header
            .or_else(|| {
                req.extensions()
                    .get::<ConnectInfo<SocketAddr>>()
                    .map(|c| c.0.ip())
            })
            .ok_or(GovernorError::UnableToExtractKey)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_eq_matches_only_identical() {
        assert!(constant_eq(b"abc", b"abc"));
        assert!(!constant_eq(b"abc", b"abd"));
        assert!(!constant_eq(b"abc", b"ab"));
    }

    #[test]
    fn client_ip_prefers_trusted_header_then_peer() {
        let req = |h: Option<&str>, peer: bool| {
            let mut r = axum::http::Request::new(());
            if let Some(h) = h {
                r.headers_mut().insert(CLIENT_IP_HEADER, h.parse().unwrap());
            }
            if peer {
                r.extensions_mut()
                    .insert(ConnectInfo(SocketAddr::from(([10, 0, 0, 1], 9))));
            }
            r
        };
        let trusted = ClientIp { trust_header: true };
        let untrusted = ClientIp {
            trust_header: false,
        };
        assert_eq!(
            trusted.extract(&req(Some("1.2.3.4"), true)).unwrap(),
            IpAddr::from([1, 2, 3, 4])
        );
        assert_eq!(
            untrusted.extract(&req(Some("1.2.3.4"), true)).unwrap(),
            IpAddr::from([10, 0, 0, 1])
        );
        assert_eq!(
            trusted.extract(&req(Some("junk"), true)).unwrap(),
            IpAddr::from([10, 0, 0, 1])
        );
        assert!(trusted.extract(&req(None, false)).is_err());
    }
}
