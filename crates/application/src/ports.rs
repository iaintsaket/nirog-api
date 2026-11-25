//! Ports: the interfaces use cases depend on. Adapters live in the outer crates.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use nirog_domain::{MatchKey, Offer, OfferId, SourceKind};
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use crate::buy::{BuyTarget, SourceProfile};
use crate::error::{IdentityError, RepoError};
use crate::model::{
    BrandAlias, BuyLink, Identity, NewSavedMatch, RecentMedicine, SavedMatch, User, UserId,
};
use crate::security::TokenHash;
use crate::stock::{Vendor, VendorListing, VendorStatus};

#[async_trait]
pub trait OfferRepository: Send + Sync {
    /// Offers whose name or salts match `query`, best matches first.
    async fn search(&self, query: &str, limit: u32) -> Result<Vec<Offer>, RepoError>;
    /// Medicine and salt names closest to a query that matched nothing, best first.
    async fn closest_names(&self, query: &str, limit: u32) -> Result<Vec<String>, RepoError>;
    async fn by_key(&self, key: &MatchKey) -> Result<Vec<Offer>, RepoError>;
    async fn key_exists(&self, key: &MatchKey) -> Result<bool, RepoError>;
    /// Atomically add `offers` as prices seen by `source` on its date, then
    /// refresh the offers served to users (the newest price per product).
    /// Products are never deleted, so the catalogue grows with every source.
    async fn record(&self, source: &SourceInfo, offers: &[Offer]) -> Result<u64, RepoError>;
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

/// The last few medicines a user opened.
#[async_trait]
pub trait RecentRepository: Send + Sync {
    /// Newest first, at most `limit`.
    async fn list(&self, user: UserId, limit: u32) -> Result<Vec<RecentMedicine>, RepoError>;
    /// Moves `item` to the top and keeps only the newest `limit`, in one statement.
    async fn record(&self, user: UserId, item: &NewSavedMatch, limit: u32)
    -> Result<(), RepoError>;
    async fn clear(&self, user: UserId) -> Result<(), RepoError>;
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

/// Brands missing from the price data, mapped to their salts.
pub trait BrandAliases: Send + Sync {
    /// The brand a query names, allowing small typos ("displin" → Disprin).
    fn find(&self, query: &str) -> Option<BrandAlias>;
}

/// Where a user can buy an offer.
pub trait SellerLinks: Send + Sync {
    fn links(&self, offer: &Offer) -> Vec<BuyLink>;
}

#[derive(Debug, thiserror::Error)]
#[error("price source failed: {0}")]
pub struct SourceError(pub String);

/// One dataset of prices, such as "brands-2022" or a Jan Aushadhi list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceInfo {
    /// Stable id, e.g. `brands-2022`; re-importing the same id updates its prices.
    pub id: String,
    pub kind: SourceKind,
    pub title: String,
    pub url: Option<String>,
    /// The date the prices were collected; the newest price per product wins.
    pub observed_on: Date,
}

/// A feed of offers from one source.
pub trait PriceSource: Send + Sync {
    fn info(&self) -> SourceInfo;
    /// Returns parsed offers and how many input rows were skipped as unusable.
    fn load(&self) -> Result<(Vec<Offer>, usize), SourceError>;
}

/// Headline figures about the price data.
#[async_trait]
pub trait StatsRepository: Send + Sync {
    async fn stats(&self) -> Result<crate::model::CatalogStats, RepoError>;
}

/// Formulas that share a salt with yours, in the same form, release type and pack unit.
#[async_trait]
pub trait NearbyOffers: Send + Sync {
    async fn nearby(
        &self,
        key: &MatchKey,
        limit: u32,
    ) -> Result<Vec<crate::model::NearbyGroup>, RepoError>;
}

/// The catalogue walked one exact-substitution key at a time, for pre-built medicine pages.
#[async_trait]
pub trait MedicinePages: Send + Sync {
    /// Offers for the first `keys` match keys after `after` (in key order) that have at least
    /// two offers, one inner list per key, each cheapest per unit first.
    async fn after(
        &self,
        after: Option<&MatchKey>,
        keys: u32,
    ) -> Result<Vec<Vec<Offer>>, RepoError>;
}

/// Where price checks keep what they saw at each source.
#[async_trait]
pub trait VendorListings: Send + Sync {
    /// Brand offers worth checking: those with at least one exact substitute.
    async fn price_check_targets(&self) -> Result<Vec<Offer>, RepoError>;
    /// Inserts or updates listings, rewriting only rows that changed. Returns rows written.
    async fn upsert(&self, listings: &[VendorListing]) -> Result<u64, RepoError>;
    /// Product page URLs at `vendor` checked at or after `since`, so a run can skip them.
    async fn fresh_urls(
        &self,
        vendor: Vendor,
        since: OffsetDateTime,
    ) -> Result<HashSet<String>, RepoError>;
}

/// Stock and price at each online pharmacy, for many offers in one read.
#[async_trait]
pub trait StockSource: Send + Sync {
    /// Statuses per offer, sorted by vendor. Offers no vendor lists are absent.
    async fn check(
        &self,
        offers: &[OfferId],
        now: OffsetDateTime,
    ) -> Result<HashMap<OfferId, Vec<VendorStatus>>, RepoError>;
}

/// Price sources' display names and search pages, kept only in the database.
#[async_trait]
pub trait SourceProfiles: Send + Sync {
    /// Every stored source, in source order. Sources without a row are absent.
    async fn profiles(&self) -> Result<Vec<SourceProfile>, RepoError>;
}

/// Where `/v1/go` sends a buyer, and how many it sent.
#[async_trait]
pub trait BuyRedirects: Send + Sync {
    /// What is stored for `offer` at `vendor`; `None` when the offer does not exist.
    async fn target(&self, offer: &OfferId, vendor: Vendor)
    -> Result<Option<BuyTarget>, RepoError>;
    /// Adds one click for `vendor` on `day`.
    async fn count_click(&self, day: Date, vendor: Vendor) -> Result<(), RepoError>;
}
