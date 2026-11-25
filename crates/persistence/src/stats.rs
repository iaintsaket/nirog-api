use async_trait::async_trait;
use nirog_application::RepoError;
use nirog_application::model::CatalogStats;
use nirog_application::ports::StatsRepository;
use sqlx::{PgPool, Row};

use crate::db;

/// Headline figures worked out from `offers` (newest price per product) and `sources`.
#[derive(Debug, Clone)]
pub struct PgStatsRepository {
    pool: PgPool,
}

impl PgStatsRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

const STATS: &str = r#"
WITH best AS (
    SELECT unit_price_millipaise AS unit, source,
           min(unit_price_millipaise) OVER (PARTITION BY match_key) AS cheapest
    FROM offers
)
SELECT count(*) AS medicines,
       count(*) FILTER (WHERE source = 'jan_aushadhi') AS jan_aushadhi,
       count(*) FILTER (WHERE unit > cheapest) AS with_cheaper,
       coalesce(percentile_disc(0.5) WITHIN GROUP (ORDER BY (unit - cheapest) * 10000 / unit)
                FILTER (WHERE unit > cheapest), 0) AS median_bps,
       (SELECT max(observed_on) FROM sources) AS updated_on
FROM best
"#;

#[async_trait]
impl StatsRepository for PgStatsRepository {
    async fn stats(&self) -> Result<CatalogStats, RepoError> {
        let row = sqlx::query(STATS).fetch_one(&self.pool).await.map_err(db)?;
        let n = |c: &str| -> Result<u64, RepoError> {
            let v: i64 = row.try_get(c).map_err(db)?;
            u64::try_from(v).map_err(|_| RepoError(format!("negative {c}")))
        };
        let bps: i64 = row.try_get("median_bps").map_err(db)?;
        Ok(CatalogStats {
            medicines: n("medicines")?,
            jan_aushadhi: n("jan_aushadhi")?,
            with_cheaper_substitute: n("with_cheaper")?,
            median_saving_bps: u32::try_from(bps)
                .map_err(|_| RepoError("saving out of range".into()))?,
            prices_updated_on: row.try_get("updated_on").map_err(db)?,
        })
    }
}
