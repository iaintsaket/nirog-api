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
    /// Profile picture URL from the identity provider, if it gave a trusted one.
    pub picture: Option<String>,
}

/// A verified identity returned by an identity provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub provider: &'static str,
    pub subject: String,
    pub email: String,
    pub name: String,
    /// Profile picture URL, `https://` on the provider's own image host only.
    pub picture: Option<String>,
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
    pub label: String,
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

/// A medicine a signed-in user opened recently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentMedicine {
    pub key: MatchKey,
    pub offer_id: Option<OfferId>,
    pub label: String,
    pub opened_at: OffsetDateTime,
}

/// Headline figures about the price data, for the home screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogStats {
    /// Every product we hold a price for.
    pub medicines: u64,
    pub jan_aushadhi: u64,
    /// Products with an exact substitute that costs less per unit.
    pub with_cheaper_substitute: u64,
    /// Median saving per unit for those products, in basis points.
    pub median_saving_bps: u32,
    /// Date of the newest price list.
    pub prices_updated_on: Option<time::Date>,
}

/// One formula near yours: its key, cheapest offer and how many offers it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearbyGroup {
    pub key: nirog_domain::MatchKey,
    pub cheapest: nirog_domain::Offer,
    pub offers: u64,
}

/// A nearby formula and exactly how it differs from yours. Never an exact substitute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseMatch {
    pub group: NearbyGroup,
    pub differences: Vec<nirog_domain::Difference>,
}
