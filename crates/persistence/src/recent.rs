use async_trait::async_trait;
use nirog_application::ports::RecentRepository;
use nirog_application::{NewSavedMatch, RecentMedicine, RepoError, UserId};
use nirog_domain::{MatchKey, OfferId};
use sqlx::{PgPool, Row, postgres::PgRow};

use crate::db;

#[derive(Debug, Clone)]
pub struct PgRecentRepository {
    pool: PgPool,
}

impl PgRecentRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn to_recent(row: &PgRow) -> Result<RecentMedicine, RepoError> {
    let offer_id: Option<String> = row.try_get("offer_id").map_err(db)?;
    Ok(RecentMedicine {
        key: MatchKey::parse(row.try_get("match_key").map_err(db)?)
            .ok_or_else(|| RepoError("corrupt recent match key".into()))?,
        offer_id: offer_id.map(OfferId),
        label: row.try_get("label").map_err(db)?,
        opened_at: row.try_get("opened_at").map_err(db)?,
    })
}

#[async_trait]
impl RecentRepository for PgRecentRepository {
    async fn list(&self, user: UserId, limit: u32) -> Result<Vec<RecentMedicine>, RepoError> {
        let rows = sqlx::query(
            "SELECT match_key, offer_id, label, opened_at FROM recent_medicines
             WHERE user_id = $1 ORDER BY opened_at DESC, match_key LIMIT $2",
        )
        .bind(user.0)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        rows.iter().map(to_recent).collect()
    }

    async fn record(
        &self,
        user: UserId,
        item: &NewSavedMatch,
        limit: u32,
    ) -> Result<(), RepoError> {
        // Both CTEs see the same snapshot, so the trim keeps the newest `limit - 1`
        // other medicines plus this one.
        sqlx::query(
            "WITH up AS (
                 INSERT INTO recent_medicines (user_id, match_key, offer_id, label)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (user_id, match_key) DO UPDATE
                 SET offer_id = EXCLUDED.offer_id, label = EXCLUDED.label, opened_at = clock_timestamp()
             )
             DELETE FROM recent_medicines
             WHERE user_id = $1 AND match_key <> $2 AND match_key NOT IN (
                 SELECT match_key FROM recent_medicines
                 WHERE user_id = $1 AND match_key <> $2
                 ORDER BY opened_at DESC, match_key LIMIT $5
             )",
        )
        .bind(user.0)
        .bind(item.key.as_str())
        .bind(item.offer_id.as_ref().map(|o| o.0.as_str()))
        .bind(&item.label)
        .bind(i64::from(limit.saturating_sub(1)))
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    async fn clear(&self, user: UserId) -> Result<(), RepoError> {
        sqlx::query("DELETE FROM recent_medicines WHERE user_id = $1")
            .bind(user.0)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }
}
