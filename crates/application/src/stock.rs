//! Stock and price at online pharmacies, as the last price check saw them.

use std::collections::HashMap;

use async_trait::async_trait;
use nirog_domain::{OfferId, Paise};
use time::{Duration, OffsetDateTime};

use crate::error::RepoError;
use crate::ports::StockSource;

use nirog_domain::DomainError;

/// After this long a listing is too old to trust, and reads as [`StockStatus::Unknown`].
pub const STALE_AFTER: Duration = Duration::days(7);

/// A price and stock source, named generically. Adding one is a variant with its arms below
/// and a migration widening the `vendor_listings` CHECK.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Vendor {
    SourceA,
    SourceB,
    SourceC,
    SourceD,
}

impl Vendor {
    /// In display order.
    pub const ALL: [Self; 4] = [Self::SourceA, Self::SourceB, Self::SourceC, Self::SourceD];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::SourceA => "source_a",
            Self::SourceB => "source_b",
            Self::SourceC => "source_c",
            Self::SourceD => "source_d",
        }
    }

    /// True when the source's stock is for one delivery area, not its whole network.
    /// Such a source's "out of stock" never hides a brand alone.
    pub fn answers_for_one_area(self) -> bool {
        self == Self::SourceA
    }

    pub fn parse(s: &str) -> Result<Self, DomainError> {
        Self::ALL
            .into_iter()
            .find(|v| v.as_str() == s)
            .ok_or(DomainError::Invalid("vendor"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StockStatus {
    InStock,
    OutOfStock,
    /// Never checked, too old, or the source could not be read. Never treated as out of stock.
    Unknown,
}

impl StockStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InStock => "in_stock",
            Self::OutOfStock => "out_of_stock",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Result<Self, DomainError> {
        match s {
            "in_stock" => Ok(Self::InStock),
            "out_of_stock" => Ok(Self::OutOfStock),
            "unknown" => Ok(Self::Unknown),
            _ => Err(DomainError::Invalid("stock status")),
        }
    }
}

/// One catalog offer as one vendor lists it. Facts only: no page copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VendorListing {
    pub offer_id: OfferId,
    pub vendor: Vendor,
    pub url: String,
    pub status: StockStatus,
    pub price: Paise,
    pub checked_at: OffsetDateTime,
}

/// What a reader is told about one vendor for one offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VendorStatus {
    pub vendor: Vendor,
    pub status: StockStatus,
    /// `None` when the listing is stale: an old price is not shown as current.
    pub price: Option<Paise>,
    pub checked_at: OffsetDateTime,
}

impl VendorStatus {
    /// The listing as of `now`.
    pub fn at(listing: &VendorListing, now: OffsetDateTime) -> Self {
        let stale = now - listing.checked_at > STALE_AFTER;
        Self {
            vendor: listing.vendor,
            status: if stale {
                StockStatus::Unknown
            } else {
                listing.status
            },
            price: (!stale).then_some(listing.price),
            checked_at: listing.checked_at,
        }
    }
}

/// True when every vendor listing the offer says it is out of stock, and at least one of them
/// answers for its whole network (see [`Vendor::answers_for_one_area`]).
/// Unknown (unchecked, stale or blocked) never counts as out of stock.
pub fn sold_out_everywhere(statuses: &[VendorStatus]) -> bool {
    statuses.iter().any(|s| !s.vendor.answers_for_one_area())
        && statuses.iter().all(|s| s.status == StockStatus::OutOfStock)
}

/// No stock data: every offer reads as not checked.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoStock;

#[async_trait]
impl StockSource for NoStock {
    async fn check(
        &self,
        _offers: &[OfferId],
        _now: OffsetDateTime,
    ) -> Result<HashMap<OfferId, Vec<VendorStatus>>, RepoError> {
        Ok(HashMap::new())
    }
}
