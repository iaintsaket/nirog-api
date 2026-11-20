//! Ports: the interfaces use cases depend on. Each has one production adapter
//! in its own crate and an in-memory fake for tests.

use async_trait::async_trait;
use nirog_domain::{MatchKey, Offer, SourceKind};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::error::{IdentityError, RepoError};
use crate::model::{BuyLink, Identity, NewSavedMatch, SavedMatch, User, UserId};
use crate::security::TokenHash;

#[async_trait]
pub trait OfferRepository: Send + Sync {
    /// Offers whose name or salts match `query`, best matches first.
    async fn search(&self, query: &str, limit: u32) -> Result<Vec<Offer>, RepoError>;
    /// Medicine and salt names closest to a query that matched nothing, best first.
    async fn closest_names(&self, query: &str, limit: u32) -> Result<Vec<String>, RepoError>;
    async fn by_key(&self, key: &MatchKey) -> Result<Vec<Offer>, RepoError>;
    async fn key_exists(&self, key: &MatchKey) -> Result<bool, RepoError>;
    /// Atomically replace every offer from `source` with `offers`.
    async fn replace_source(&self, source: SourceKind, offers: &[Offer]) -> Result<u64, RepoError>;
}

#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn upsert_identity(&self, identity: &Identity) -> Result<User, RepoError>;
    async fn get(&self, id: UserId) -> Result<Option<User>, RepoError>;
    /// Deletes the user and, by cascade, their sessions and saved matches.
    async fn delete(&self, id: UserId) -> Result<bool, RepoError>;
}

#[async_trait]
pub trait SessionRepository: Send + Sync {
    async fn create(
        &self,
        hash: &TokenHash,
        user: UserId,
        expires_at: OffsetDateTime,
    ) -> Result<(), RepoError>;
    async fn user_for(
        &self,
        hash: &TokenHash,
        now: OffsetDateTime,
    ) -> Result<Option<UserId>, RepoError>;
    async fn delete(&self, hash: &TokenHash) -> Result<(), RepoError>;
}

/// Short-lived OAuth `state` → PKCE verifier store. `take` is single-use.
#[async_trait]
pub trait LoginStateRepository: Send + Sync {
    async fn put(
        &self,
        state: &TokenHash,
        pkce_verifier: &str,
        expires_at: OffsetDateTime,
    ) -> Result<(), RepoError>;
    async fn take(
        &self,
        state: &TokenHash,
        now: OffsetDateTime,
    ) -> Result<Option<String>, RepoError>;
}

#[async_trait]
pub trait SavedMatchRepository: Send + Sync {
    async fn list(&self, user: UserId) -> Result<Vec<SavedMatch>, RepoError>;
    async fn count(&self, user: UserId) -> Result<u64, RepoError>;
    /// Saving the same key twice updates the existing entry.
    async fn save(&self, user: UserId, item: &NewSavedMatch) -> Result<SavedMatch, RepoError>;
    async fn delete(&self, user: UserId, id: Uuid) -> Result<bool, RepoError>;
}

#[async_trait]
pub trait IdentityProvider: Send + Sync {
    fn authorize_url(&self, state: &str, pkce_challenge: &str) -> String;
    async fn exchange(&self, code: &str, pkce_verifier: &str) -> Result<Identity, IdentityError>;
}

pub trait Clock: Send + Sync {
    fn now(&self) -> OffsetDateTime;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}

/// Where a user can buy an offer.
pub trait SellerLinks: Send + Sync {
    fn links(&self, offer: &Offer) -> Vec<BuyLink>;
}

#[derive(Debug, thiserror::Error)]
#[error("price source failed: {0}")]
pub struct SourceError(pub String);

/// A feed of offers from one source (a CSV today; AMRIT, NPPA or a partner API later).
pub trait PriceSource: Send + Sync {
    fn kind(&self) -> SourceKind;
    /// Returns parsed offers and how many input rows were skipped as unusable.
    fn load(&self) -> Result<(Vec<Offer>, usize), SourceError>;
}
