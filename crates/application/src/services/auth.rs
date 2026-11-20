use std::sync::Arc;

use time::Duration;

use crate::error::AppError;
use crate::model::{User, UserId};
use crate::ports::{
    Clock, IdentityProvider, LoginStateRepository, SessionRepository, UserRepository,
};
use crate::security::{TokenHash, pkce_challenge, random_token};

const LOGIN_STATE_TTL_MINUTES: i64 = 10;
const MAX_TOKEN_LEN: usize = 128;

/// Raw session token. Goes into an HttpOnly cookie; never stored or logged.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionToken(pub String);

impl std::fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionToken(<redacted>)")
    }
}

/// Where to send the browser, and the `state` the HTTP layer must bind to that
/// browser (a short-lived cookie) so a callback started elsewhere is refused.
#[derive(Clone, PartialEq, Eq)]
pub struct LoginStart {
    pub redirect_url: String,
    pub state: String,
}

impl std::fmt::Debug for LoginStart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LoginStart(<redacted>)")
    }
}

/// Sign-in with an external identity provider, server-side sessions, account deletion.
#[derive(Clone)]
pub struct AuthService {
    users: Arc<dyn UserRepository>,
    sessions: Arc<dyn SessionRepository>,
    states: Arc<dyn LoginStateRepository>,
    idp: Arc<dyn IdentityProvider>,
    clock: Arc<dyn Clock>,
    session_ttl: Duration,
}

impl std::fmt::Debug for AuthService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AuthService")
    }
}

fn internal(e: impl std::fmt::Display) -> AppError {
    AppError::Repo(crate::error::RepoError(e.to_string()))
}

impl AuthService {
    pub fn new(
        users: Arc<dyn UserRepository>,
        sessions: Arc<dyn SessionRepository>,
        states: Arc<dyn LoginStateRepository>,
        idp: Arc<dyn IdentityProvider>,
        clock: Arc<dyn Clock>,
        session_ttl: Duration,
    ) -> Self {
        Self {
            users,
            sessions,
            states,
            idp,
            clock,
            session_ttl,
        }
    }

    pub fn session_ttl(&self) -> Duration {
        self.session_ttl
    }

    /// Creates a single-use `state` bound to a PKCE verifier and returns the provider URL.
    pub async fn begin_login(&self) -> Result<LoginStart, AppError> {
        let state = random_token().map_err(internal)?;
        let verifier = random_token().map_err(internal)?;
        let expires = self.clock.now() + Duration::minutes(LOGIN_STATE_TTL_MINUTES);
        self.states
            .put(&TokenHash::of(&state), &verifier, expires)
            .await?;
        Ok(LoginStart {
            redirect_url: self.idp.authorize_url(&state, &pkce_challenge(&verifier)),
            state,
        })
    }

    /// Verifies `state`, exchanges the code, upserts the user and opens a session.
    pub async fn complete_login(&self, code: &str, state: &str) -> Result<SessionToken, AppError> {
        if code.is_empty() || code.len() > 2048 || state.is_empty() || state.len() > MAX_TOKEN_LEN {
            return Err(AppError::Invalid("bad callback parameters"));
        }
        let now = self.clock.now();
        let verifier = self
            .states
            .take(&TokenHash::of(state), now)
            .await?
            .ok_or(AppError::Unauthorized)?;
        let identity = self.idp.exchange(code, &verifier).await?;
        let user = self.users.upsert_identity(&identity).await?;
        let token = random_token().map_err(internal)?;
        self.sessions
            .create(&TokenHash::of(&token), user.id, now + self.session_ttl)
            .await?;
        Ok(SessionToken(token))
    }

    pub async fn current_user(&self, token: &str) -> Result<User, AppError> {
        if token.is_empty() || token.len() > MAX_TOKEN_LEN {
            return Err(AppError::Unauthorized);
        }
        let id = self
            .sessions
            .user_for(&TokenHash::of(token), self.clock.now())
            .await?
            .ok_or(AppError::Unauthorized)?;
        self.users.get(id).await?.ok_or(AppError::Unauthorized)
    }

    pub async fn logout(&self, token: &str) -> Result<(), AppError> {
        if token.len() <= MAX_TOKEN_LEN {
            self.sessions.delete(&TokenHash::of(token)).await?;
        }
        Ok(())
    }

    /// Deletes the account and everything linked to it (DPDP right to erasure).
    pub async fn delete_account(&self, user: UserId) -> Result<(), AppError> {
        if self.users.delete(user).await? {
            Ok(())
        } else {
            Err(AppError::NotFound)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::IdentityError;
    use crate::fakes::{FakeClock, FakeIdp, InMemoryAuth};
    use url::Url;

    fn service(idp: FakeIdp, store: &Arc<InMemoryAuth>, clock: &Arc<FakeClock>) -> AuthService {
        AuthService::new(
            store.clone(),
            store.clone(),
            store.clone(),
            Arc::new(idp),
            clock.clone(),
            Duration::days(30),
        )
    }

    fn state_of(url: &str) -> String {
        Url::parse(url)
            .unwrap()
            .query_pairs()
            .find(|(k, _)| k == "state")
            .unwrap()
            .1
            .into_owned()
    }

    #[tokio::test]
    async fn full_login_logout_cycle() {
        let (store, clock) = (
            Arc::new(InMemoryAuth::default()),
            Arc::new(FakeClock::default()),
        );
        let auth = service(FakeIdp::ok(), &store, &clock);
        let start = auth.begin_login().await.unwrap();
        let state = state_of(&start.redirect_url);
        assert!(start.redirect_url.contains("code_challenge="));
        assert_eq!(start.state, state);
        assert!(!format!("{start:?}").contains(&state));

        let token = auth.complete_login("good-code", &state).await.unwrap();
        assert_eq!(format!("{token:?}"), "SessionToken(<redacted>)");
        let user = auth.current_user(&token.0).await.unwrap();
        assert_eq!(user.email, "asha@example.com");

        // state is single-use
        assert!(matches!(
            auth.complete_login("good-code", &state).await,
            Err(AppError::Unauthorized)
        ));

        auth.logout(&token.0).await.unwrap();
        assert!(matches!(
            auth.current_user(&token.0).await,
            Err(AppError::Unauthorized)
        ));
    }

    #[tokio::test]
    async fn expired_state_and_session_are_rejected() {
        let (store, clock) = (
            Arc::new(InMemoryAuth::default()),
            Arc::new(FakeClock::default()),
        );
        let auth = service(FakeIdp::ok(), &store, &clock);
        let state = state_of(&auth.begin_login().await.unwrap().redirect_url);
        clock.advance(Duration::minutes(11));
        assert!(matches!(
            auth.complete_login("c", &state).await,
            Err(AppError::Unauthorized)
        ));

        let state = state_of(&auth.begin_login().await.unwrap().redirect_url);
        let token = auth.complete_login("c", &state).await.unwrap();
        clock.advance(Duration::days(31));
        assert!(matches!(
            auth.current_user(&token.0).await,
            Err(AppError::Unauthorized)
        ));
    }

    #[tokio::test]
    async fn rejects_bad_input_and_provider_failures() {
        let (store, clock) = (
            Arc::new(InMemoryAuth::default()),
            Arc::new(FakeClock::default()),
        );
        let auth = service(FakeIdp::failing(), &store, &clock);
        assert!(matches!(
            auth.complete_login("", "s").await,
            Err(AppError::Invalid(_))
        ));
        assert!(matches!(
            auth.complete_login("c", &"s".repeat(200)).await,
            Err(AppError::Invalid(_))
        ));
        assert!(matches!(
            auth.current_user("").await,
            Err(AppError::Unauthorized)
        ));
        assert!(matches!(
            auth.current_user(&"t".repeat(200)).await,
            Err(AppError::Unauthorized)
        ));
        auth.logout(&"t".repeat(200)).await.unwrap();

        let state = state_of(&auth.begin_login().await.unwrap().redirect_url);
        assert!(matches!(
            auth.complete_login("c", &state).await,
            Err(AppError::Identity(IdentityError::Rejected(_)))
        ));
    }

    #[tokio::test]
    async fn delete_account_removes_user_and_sessions() {
        let (store, clock) = (
            Arc::new(InMemoryAuth::default()),
            Arc::new(FakeClock::default()),
        );
        let auth = service(FakeIdp::ok(), &store, &clock);
        let state = state_of(&auth.begin_login().await.unwrap().redirect_url);
        let token = auth.complete_login("c", &state).await.unwrap();
        let user = auth.current_user(&token.0).await.unwrap();
        auth.delete_account(user.id).await.unwrap();
        assert!(matches!(
            auth.current_user(&token.0).await,
            Err(AppError::Unauthorized)
        ));
        assert!(matches!(
            auth.delete_account(user.id).await,
            Err(AppError::NotFound)
        ));
        assert_eq!(auth.session_ttl(), Duration::days(30));
    }
}
