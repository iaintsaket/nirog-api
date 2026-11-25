use async_trait::async_trait;
use nirog_application::RepoError;
use nirog_application::buy::{BuyTarget, SourceProfile};
use nirog_application::ports::{BuyRedirects, SourceProfiles};
use nirog_application::stock::Vendor;
use nirog_domain::OfferId;
use sqlx::{PgPool, Row};
use time::Date;

use crate::db;

/// Price sources' names and search pages, buy redirect targets and click counts.
#[derive(Debug, Clone)]
pub struct PgBuyLinks {
    pool: PgPool,
}

impl PgBuyLinks {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SourceProfiles for PgBuyLinks {
    async fn profiles(&self) -> Result<Vec<SourceProfile>, RepoError> {
        let rows = sqlx::query(
            "SELECT id, display_name, search_url_template FROM price_sources ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        rows.iter()
            .map(|row| {
                Ok(SourceProfile {
                    vendor: Vendor::parse(row.try_get("id").map_err(db)?)
                        .map_err(|_| RepoError("corrupt price source id".into()))?,
                    display_name: row.try_get("display_name").map_err(db)?,
                    search_url_template: row.try_get("search_url_template").map_err(db)?,
                })
            })
            .collect()
    }
}

#[async_trait]
impl BuyRedirects for PgBuyLinks {
    async fn target(
        &self,
        offer: &OfferId,
        vendor: Vendor,
    ) -> Result<Option<BuyTarget>, RepoError> {
        let row = sqlx::query(
            "SELECT o.name, l.url, p.search_url_template
             FROM offers o
             LEFT JOIN vendor_listings l ON l.offer_id = o.id AND l.vendor = $2
             LEFT JOIN price_sources p ON p.id = $2
             WHERE o.id = $1",
        )
        .bind(&offer.0)
        .bind(vendor.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        row.map(|row| {
            Ok(BuyTarget {
                offer_name: row.try_get("name").map_err(db)?,
                listing_url: row.try_get("url").map_err(db)?,
                search_url_template: row.try_get("search_url_template").map_err(db)?,
            })
        })
        .transpose()
    }

    async fn count_click(&self, day: Date, vendor: Vendor) -> Result<(), RepoError> {
        sqlx::query(
            "INSERT INTO buy_clicks (day, source_id, clicks) VALUES ($1, $2, 1)
             ON CONFLICT (day, source_id) DO UPDATE SET clicks = buy_clicks.clicks + 1",
        )
        .bind(day)
        .bind(vendor.as_str())
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }
}
