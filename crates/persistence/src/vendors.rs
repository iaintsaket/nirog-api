use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use nirog_application::RepoError;
use nirog_application::ports::{StockSource, VendorListings};
use nirog_application::stock::{StockStatus, Vendor, VendorListing, VendorStatus};
use nirog_domain::{Offer, OfferId, Paise};
use sqlx::{PgPool, Row};
use time::OffsetDateTime;

use crate::db;
use crate::offers::to_offer;

/// Stock and prices that price checks saw. Serves both the price checker and the API.
#[derive(Debug, Clone)]
pub struct PgVendorListings {
    pool: PgPool,
}

impl PgVendorListings {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// An unchanged listing's check time is rewritten at most this often, so a
/// repeated run writes nothing and a daily run touches each row once.
const TOUCH_AFTER: &str = "20 hours";

#[async_trait]
impl VendorListings for PgVendorListings {
    async fn price_check_targets(&self) -> Result<Vec<Offer>, RepoError> {
        let rows = sqlx::query(
            "SELECT id, source, name, manufacturer, price_paise, pack_count, pack_unit, match_key,
                    salts_label
             FROM offers o
             WHERE source = 'brand'
               AND EXISTS (SELECT 1 FROM offers s WHERE s.match_key = o.match_key AND s.id <> o.id)
             ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        rows.iter().map(to_offer).collect()
    }

    async fn upsert(&self, listings: &[VendorListing]) -> Result<u64, RepoError> {
        let mut written = 0;
        // Chunked UNNEST: one round trip per 1,000 rows.
        for chunk in listings.chunks(1000) {
            let mut text: [Vec<String>; 4] = Default::default();
            let (mut price, mut at) = (Vec::new(), Vec::new());
            for l in chunk {
                text[0].push(l.offer_id.0.clone());
                text[1].push(l.vendor.as_str().to_owned());
                text[2].push(l.url.clone());
                text[3].push(l.status.as_str().to_owned());
                price.push(l.price.0);
                at.push(l.checked_at);
            }
            let [ids, vendors, urls, statuses] = text;
            written += sqlx::query(
                "INSERT INTO vendor_listings (offer_id, vendor, url, status, price_paise, checked_at)
                 SELECT * FROM UNNEST($1::text[], $2::text[], $3::text[], $4::text[],
                                      $5::bigint[], $6::timestamptz[])
                 ON CONFLICT (offer_id, vendor) DO UPDATE SET
                     url = EXCLUDED.url, status = EXCLUDED.status,
                     price_paise = EXCLUDED.price_paise, checked_at = EXCLUDED.checked_at
                 WHERE EXCLUDED.checked_at >= vendor_listings.checked_at
                   AND ((vendor_listings.url, vendor_listings.status, vendor_listings.price_paise)
                        IS DISTINCT FROM (EXCLUDED.url, EXCLUDED.status, EXCLUDED.price_paise)
                        OR EXCLUDED.checked_at - vendor_listings.checked_at >= $7::interval)",
            )
            .bind(ids)
            .bind(vendors)
            .bind(urls)
            .bind(statuses)
            .bind(price)
            .bind(at)
            .bind(TOUCH_AFTER)
            .execute(&self.pool)
            .await
            .map_err(db)?
            .rows_affected();
        }
        Ok(written)
    }

    async fn fresh_urls(
        &self,
        vendor: Vendor,
        since: OffsetDateTime,
    ) -> Result<HashSet<String>, RepoError> {
        let urls: Vec<String> = sqlx::query_scalar(
            "SELECT url FROM vendor_listings WHERE vendor = $1 AND checked_at >= $2",
        )
        .bind(vendor.as_str())
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        Ok(urls.into_iter().collect())
    }
}

#[async_trait]
impl StockSource for PgVendorListings {
    async fn check(
        &self,
        offers: &[OfferId],
        now: OffsetDateTime,
    ) -> Result<HashMap<OfferId, Vec<VendorStatus>>, RepoError> {
        if offers.is_empty() {
            return Ok(HashMap::new());
        }
        let ids: Vec<&str> = offers.iter().map(|o| o.0.as_str()).collect();
        let rows = sqlx::query(
            "SELECT offer_id, vendor, url, status, price_paise, checked_at FROM vendor_listings
             WHERE offer_id = ANY($1)",
        )
        .bind(&ids)
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        let bad = |what: &str| RepoError(format!("corrupt vendor listing: {what}"));
        let mut out: HashMap<OfferId, Vec<VendorStatus>> = HashMap::new();
        for row in &rows {
            let listing = VendorListing {
                offer_id: OfferId(row.try_get("offer_id").map_err(db)?),
                vendor: Vendor::parse(row.try_get("vendor").map_err(db)?)
                    .map_err(|_| bad("vendor"))?,
                url: row.try_get("url").map_err(db)?,
                status: StockStatus::parse(row.try_get("status").map_err(db)?)
                    .map_err(|_| bad("status"))?,
                price: Paise(row.try_get("price_paise").map_err(db)?),
                checked_at: row.try_get("checked_at").map_err(db)?,
            };
            out.entry(listing.offer_id.clone())
                .or_default()
                .push(VendorStatus::at(&listing, now));
        }
        for statuses in out.values_mut() {
            statuses.sort_by_key(|s| s.vendor);
        }
        Ok(out)
    }
}
