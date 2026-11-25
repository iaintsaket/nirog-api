use async_trait::async_trait;
use nirog_application::RepoError;
use nirog_application::ports::MedicinePages;
use nirog_domain::{MatchKey, Offer};
use sqlx::PgPool;

use crate::db;
use crate::offers::to_offer;

/// Walks `offers` by match key for the website's pre-built medicine pages.
#[derive(Debug, Clone)]
pub struct PgMedicinePages {
    pool: PgPool,
}

impl PgMedicinePages {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Keyset walk over the `(match_key, unit_price_millipaise)` index: the next `$2` keys after
/// `$1` that have a substitute, then all their offers in key order, cheapest first.
const PAGES: &str = r#"
WITH keys AS (
    SELECT match_key FROM offers
    WHERE $1::text IS NULL OR match_key > $1
    GROUP BY match_key HAVING count(*) >= 2
    ORDER BY match_key
    LIMIT $2
)
SELECT o.id, o.source, o.name, o.manufacturer, o.price_paise, o.pack_count, o.pack_unit,
       o.match_key, o.salts_label
FROM offers o JOIN keys USING (match_key)
ORDER BY o.match_key, o.unit_price_millipaise, o.id
"#;

#[async_trait]
impl MedicinePages for PgMedicinePages {
    async fn after(
        &self,
        after: Option<&MatchKey>,
        keys: u32,
    ) -> Result<Vec<Vec<Offer>>, RepoError> {
        let rows = sqlx::query(PAGES)
            .bind(after.map(MatchKey::as_str))
            .bind(i64::from(keys))
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        let mut groups: Vec<Vec<Offer>> = Vec::new();
        for row in &rows {
            let offer = to_offer(row)?;
            match groups.last_mut() {
                Some(g) if g[0].key == offer.key => g.push(offer),
                _ => groups.push(vec![offer]),
            }
        }
        Ok(groups)
    }
}
