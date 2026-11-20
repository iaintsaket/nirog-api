use nirog_domain::{MatchKey, Offer, OfferId};
use serde::Serialize;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct UserId(pub Uuid);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct User {
    pub id: UserId,
    pub email: String,
    pub name: String,
}

/// A verified identity returned by an identity provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub provider: &'static str,
    pub subject: String,
    pub email: String,
    pub name: String,
}

/// A brand missing from every price list, and what it contains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrandAlias {
    pub brand: String,
    /// The brand's exact salts, strength and form.
    pub key: MatchKey,
    pub salts_label: String,
    /// What to search instead: the salt names ("Aspirin").
    pub search: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchGroup {
    pub key: MatchKey,
    pub salts_label: String,
    pub matched: Vec<Offer>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// Walk-in store finder (Jan Aushadhi may not be sold online).
    Store,
    /// A licensed online pharmacy's search page.
    Online,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BuyLink {
    pub label: &'static str,
    pub url: String,
    pub kind: LinkKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSavedMatch {
    pub key: MatchKey,
    pub offer_id: Option<OfferId>,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedMatch {
    pub id: Uuid,
    pub key: MatchKey,
    pub offer_id: Option<OfferId>,
    pub label: String,
    pub created_at: OffsetDateTime,
}
