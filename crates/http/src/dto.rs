//! Wire types. Kept separate from domain types so the API contract changes
//! only on purpose; every type here appears in the OpenAPI document.

use nirog_application::buy::SourceLink;
use nirog_application::services::catalog::ComparisonView;
use nirog_application::stock::{StockStatus, VendorStatus};
use nirog_application::{
    BrandAlias, BuyLink, LinkKind, RecentMedicine, SavedMatch, SearchGroup, User,
};
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
    /// An `https` URL, or a path on this API (`/v1/go/...`) that redirects to an online pharmacy.
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
            label: l.label.clone(),
            url: l.url.clone(),
            kind,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ComparedOffer {
    pub offer: OfferDto,
    pub links: Vec<BuyLinkDto>,
    /// Stock at each online pharmacy, as last checked; empty when no pharmacy was checked.
    pub vendors: Vec<VendorStatusDto>,
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StockStatusDto {
    InStock,
    OutOfStock,
    /// Not checked recently enough to say.
    Unknown,
}

impl From<StockStatus> for StockStatusDto {
    fn from(s: StockStatus) -> Self {
        match s {
            StockStatus::InStock => Self::InStock,
            StockStatus::OutOfStock => Self::OutOfStock,
            StockStatus::Unknown => Self::Unknown,
        }
    }
}

/// One online pharmacy's listing of an offer.
#[derive(Debug, Serialize, ToSchema)]
pub struct VendorStatusDto {
    /// Stable source id, e.g. "source_a".
    pub vendor: String,
    /// The pharmacy's name.
    pub display_name: String,
    /// Path on this API (`/v1/go/...`) that redirects to the pharmacy; absent when there is
    /// nowhere to send a buyer.
    pub buy_path: Option<String>,
    pub status: StockStatusDto,
    /// The pharmacy's price per pack, in paise; absent when the listing is too old to trust.
    pub price_paise: Option<i64>,
    /// When the source was last checked.
    #[serde(with = "time::serde::rfc3339")]
    pub checked_at: OffsetDateTime,
}

impl From<(&VendorStatus, &SourceLink)> for VendorStatusDto {
    fn from((v, link): (&VendorStatus, &SourceLink)) -> Self {
        Self {
            vendor: v.vendor.as_str().to_owned(),
            display_name: link.display_name.clone(),
            buy_path: link.buy_path.clone(),
            status: v.status.into(),
            price_paise: v.price.map(|p| p.0),
            checked_at: v.checked_at,
        }
    }
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
                .enumerate()
                .map(|(i, (o, links))| ComparedOffer {
                    offer: o.into(),
                    links: links.iter().map(BuyLinkDto::from).collect(),
                    vendors: v
                        .stock
                        .get(i)
                        .zip(v.sources.get(i))
                        .map(|(s, l)| s.iter().zip(l).map(VendorStatusDto::from).collect())
                        .unwrap_or_default(),
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
    /// Google profile picture (`https://` on Google's image host), when the account has one.
    pub picture_url: Option<String>,
    /// The last medicines this user opened, newest first (at most 3).
    pub recent: Vec<RecentDto>,
}

impl From<User> for UserDto {
    fn from(u: User) -> Self {
        Self {
            id: u.id.0,
            email: u.email,
            name: u.name,
            picture_url: u.picture,
            recent: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RecentDto {
    pub key: String,
    pub offer_id: Option<String>,
    pub label: String,
    #[serde(with = "time::serde::rfc3339")]
    pub opened_at: OffsetDateTime,
}

impl From<RecentMedicine> for RecentDto {
    fn from(r: RecentMedicine) -> Self {
        Self {
            key: r.key.as_str().to_owned(),
            offer_id: r.offer_id.map(|o| o.0),
            label: r.label,
            opened_at: r.opened_at,
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

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PagesParams {
    /// The `next` value from the previous batch; omit for the first.
    pub after: Option<String>,
}

/// One medicine (exact-substitution key) as the website's pre-built page shows it.
#[derive(Debug, Serialize, ToSchema)]
pub struct MedicinePageDto {
    /// The cheapest 20 offers; the saving is against the costliest of all of them.
    pub comparison: ComparisonResponse,
    /// Distinct names of every offer with this key, cheapest first (at most 1,000).
    pub names: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PagesResponse {
    /// Medicines with at least one exact substitute, in match-key order.
    pub pages: Vec<MedicinePageDto>,
    /// Pass as `after` to get the next batch; absent when this was the last.
    pub next: Option<String>,
}

impl From<nirog_application::services::PagesBatch> for PagesResponse {
    fn from(b: nirog_application::services::PagesBatch) -> Self {
        Self {
            pages: b
                .pages
                .into_iter()
                .map(|p| MedicinePageDto {
                    comparison: p.view.into(),
                    names: p.names,
                })
                .collect(),
            next: b.next.map(|k| k.as_str().to_owned()),
        }
    }
}
