//! Wire types. Kept separate from domain types so the API contract changes
//! only on purpose; every type here appears in the OpenAPI document.

use nirog_application::services::catalog::ComparisonView;
use nirog_application::{BrandAlias, BuyLink, LinkKind, SavedMatch, SearchGroup, User};
use nirog_domain::{Offer, PackUnit, SourceKind};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    /// Stable machine-readable code: `not_found`, `unauthorized`, `invalid`, `limit`, `forbidden`, ...
    pub error: String,
    pub message: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct Health {
    pub status: String,
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    JanAushadhi,
    Brand,
}

impl From<SourceKind> for Source {
    fn from(s: SourceKind) -> Self {
        match s {
            SourceKind::JanAushadhi => Self::JanAushadhi,
            SourceKind::Brand => Self::Brand,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Unit,
    Ml,
    G,
}

impl From<PackUnit> for Unit {
    fn from(u: PackUnit) -> Self {
        match u {
            PackUnit::Unit => Self::Unit,
            PackUnit::Ml => Self::Ml,
            PackUnit::G => Self::G,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OfferDto {
    pub id: String,
    pub source: Source,
    pub name: String,
    pub manufacturer: String,
    /// Pack price (MRP) in paise.
    pub price_paise: i64,
    pub pack_count: u32,
    pub pack_unit: Unit,
    /// Price of one unit (tablet, ml or g) in thousandths of a paisa.
    pub unit_price_millipaise: i64,
    pub salts_label: String,
}

impl From<&Offer> for OfferDto {
    fn from(o: &Offer) -> Self {
        Self {
            id: o.id.0.clone(),
            source: o.source.into(),
            name: o.name.clone(),
            manufacturer: o.manufacturer.clone(),
            price_paise: o.price.0,
            pack_count: o.pack.count,
            pack_unit: o.pack.unit.into(),
            unit_price_millipaise: o.unit_price_millipaise(),
            salts_label: o.salts_label.clone(),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SearchGroupDto {
    /// Exact-substitution key; pass it to `/v1/compare`.
    pub key: String,
    pub salts_label: String,
    pub matched: Vec<OfferDto>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SearchResponse {
    pub groups: Vec<SearchGroupDto>,
    /// When nothing matched: the closest medicine or salt names, best first.
    pub did_you_mean: Vec<String>,
    /// Set when the query names a brand we have no price for (e.g. Disprin):
    /// `groups` then lists medicines with the same salt, exact strength first.
    pub same_salt_as: Option<BrandAliasDto>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BrandAliasDto {
    pub brand: String,
    /// The brand's salts and strengths, e.g. "Aspirin 350mg".
    pub salts_label: String,
    /// Exact-substitution key of the brand itself; a group with this key is an exact match.
    pub key: String,
}

impl From<&BrandAlias> for BrandAliasDto {
    fn from(a: &BrandAlias) -> Self {
        Self {
            brand: a.brand.clone(),
            salts_label: a.salts_label.clone(),
            key: a.key.as_str().to_owned(),
        }
    }
}

impl From<Vec<SearchGroup>> for SearchResponse {
    fn from(groups: Vec<SearchGroup>) -> Self {
        Self {
            groups: groups
                .iter()
                .map(|g| SearchGroupDto {
                    key: g.key.as_str().to_owned(),
                    salts_label: g.salts_label.clone(),
                    matched: g.matched.iter().map(OfferDto::from).collect(),
                })
                .collect(),
            did_you_mean: Vec::new(),
            same_salt_as: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LinkKindDto {
    /// Walk-in store finder (Jan Aushadhi medicines may not be sold online).
    Store,
    /// A licensed online pharmacy's search page.
    Online,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BuyLinkDto {
    pub label: String,
    pub url: String,
    pub kind: LinkKindDto,
}

impl From<&BuyLink> for BuyLinkDto {
    fn from(l: &BuyLink) -> Self {
        let kind = match l.kind {
            LinkKind::Store => LinkKindDto::Store,
            LinkKind::Online => LinkKindDto::Online,
        };
        Self {
            label: l.label.to_owned(),
            url: l.url.clone(),
            kind,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ComparedOffer {
    pub offer: OfferDto,
    pub links: Vec<BuyLinkDto>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ComparisonResponse {
    pub key: String,
    /// Cheapest per unit first: the 50 cheapest, plus the searched offer if it ranks lower.
    pub offers: Vec<ComparedOffer>,
    /// Number of exact substitutes in the catalogue.
    pub total_offers: usize,
    /// Index into `offers` of the medicine the user searched for, if known.
    pub searched_index: Option<usize>,
    /// Saving per pack of the searched (or costliest) offer when buying the cheapest, in paise.
    pub saving_per_pack_paise: i64,
    /// Saving in basis points (1/100 of a percent).
    pub saving_bps: i64,
}

impl From<ComparisonView> for ComparisonResponse {
    fn from(v: ComparisonView) -> Self {
        let c = v.comparison;
        Self {
            key: c.cheapest().key.as_str().to_owned(),
            offers: c
                .offers
                .iter()
                .zip(&v.links)
                .map(|(o, links)| ComparedOffer {
                    offer: o.into(),
                    links: links.iter().map(BuyLinkDto::from).collect(),
                })
                .collect(),
            total_offers: v.total,
            searched_index: c.searched,
            saving_per_pack_paise: c.saving_per_pack.0,
            saving_bps: c.saving_bps,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserDto {
    pub id: Uuid,
    pub email: String,
    pub name: String,
}

impl From<User> for UserDto {
    fn from(u: User) -> Self {
        Self {
            id: u.id.0,
            email: u.email,
            name: u.name,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SavedMatchDto {
    pub id: Uuid,
    pub key: String,
    pub offer_id: Option<String>,
    pub label: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

impl From<SavedMatch> for SavedMatchDto {
    fn from(s: SavedMatch) -> Self {
        Self {
            id: s.id,
            key: s.key.as_str().to_owned(),
            offer_id: s.offer_id.map(|o| o.0),
            label: s.label,
            created_at: s.created_at,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveMatchRequest {
    pub key: String,
    /// The offer the user was looking at when saving, if any.
    pub offer_id: Option<String>,
    /// 1 to 120 characters.
    pub label: String,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SearchParams {
    /// Medicine or salt name, 2 to 100 characters.
    pub q: String,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CompareParams {
    /// Match key from a search group.
    pub key: String,
    /// Offer id the user searched for, to compute the saving against it.
    pub searched: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CallbackParams {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

/// Headline figures about the price data, for the home screen.
#[derive(Debug, Serialize, ToSchema)]
pub struct StatsResponse {
    /// Every medicine we hold a price for.
    pub medicines: u64,
    pub jan_aushadhi: u64,
    /// Medicines with an exact substitute that costs less per unit.
    pub with_cheaper_substitute: u64,
    /// Median saving per unit for those medicines, in basis points (5147 = 51.47%).
    pub median_saving_bps: u32,
    /// Date of the newest price list, YYYY-MM-DD.
    pub prices_updated_on: Option<String>,
}

impl From<nirog_application::model::CatalogStats> for StatsResponse {
    fn from(s: nirog_application::model::CatalogStats) -> Self {
        Self {
            medicines: s.medicines,
            jan_aushadhi: s.jan_aushadhi,
            with_cheaper_substitute: s.with_cheaper_substitute,
            median_saving_bps: s.median_saving_bps,
            prices_updated_on: s.prices_updated_on.map(|d| d.to_string()),
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CloseParams {
    /// Match key of your medicine (from search or compare).
    pub key: String,
}

/// Nearby formulas when there is no cheaper exact substitute. These are NOT substitutes.
#[derive(Debug, Serialize, ToSchema)]
pub struct CloseResponse {
    pub matches: Vec<CloseMatchDto>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CloseMatchDto {
    pub key: String,
    pub salts_label: String,
    /// How many offers have this formula.
    pub offers: u64,
    pub cheapest: OfferDto,
    /// What differs from your medicine, in plain words, e.g. "Capsaicin 0.025% instead of 0.01%".
    pub differences: Vec<String>,
}

impl From<Vec<nirog_application::model::CloseMatch>> for CloseResponse {
    fn from(v: Vec<nirog_application::model::CloseMatch>) -> Self {
        Self {
            matches: v
                .into_iter()
                .map(|m| CloseMatchDto {
                    key: m.group.key.as_str().to_owned(),
                    salts_label: m.group.cheapest.salts_label.clone(),
                    offers: m.group.offers,
                    cheapest: OfferDto::from(&m.group.cheapest),
                    differences: m.differences.iter().map(ToString::to_string).collect(),
                })
                .collect(),
        }
    }
}
