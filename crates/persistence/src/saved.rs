use async_trait::async_trait;
use nirog_application::ports::SavedMatchRepository;
use nirog_application::{NewSavedMatch, RepoError, SavedMatch, UserId};
use nirog_domain::{MatchKey, OfferId};
use sqlx::{PgPool, Row, postgres::PgRow};
use uuid::Uuid;

use crate::db;

#[derive(Debug, Clone)]
pub struct PgSavedMatchRepository {
    pool: PgPool,
}

impl PgSavedMatchRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn to_saved(row: &PgRow) -> Result<SavedMatch, RepoError> {
    let offer_id: Option<String> = row.try_get("offer_id").map_err(db)?;
    Ok(SavedMatch {
        id: row.try_get("id").map_err(db)?,
        key: MatchKey::parse(row.try_get("match_key").map_err(db)?)
            .ok_or_else(|| RepoError("corrupt saved match key".into()))?,
        offer_id: offer_id.map(OfferId),
        label: row.try_get("label").map_err(db)?,
        created_at: row.try_get("created_at").map_err(db)?,
    })
}

#[async_trait]
impl SavedMatchRepository for PgSavedMatchRepository {
    async fn list(&self, user: UserId) -> Result<Vec<SavedMatch>, RepoError> {
        let rows = sqlx::query(
            "SELECT id, match_key, offer_id, label, created_at FROM saved_matches
             WHERE user_id = $1 ORDER BY created_at DESC, id",
        )
        .bind(user.0)
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;
        rows.iter().map(to_saved).collect()
    }

    async fn count(&self, user: UserId) -> Result<u64, RepoError> {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM saved_matches WHERE user_id = $1")
            .bind(user.0)
            .fetch_one(&self.pool)
            .await
            .map_err(db)?;
        Ok(n.unsigned_abs())
    }

    async fn save(&self, user: UserId, item: &NewSavedMatch) -> Result<SavedMatch, RepoError> {
        let row = sqlx::query(
            "INSERT INTO saved_matches (user_id, match_key, offer_id, label) VALUES ($1, $2, $3, $4)
             ON CONFLICT (user_id, match_key) DO UPDATE SET offer_id = EXCLUDED.offer_id, label = EXCLUDED.label
             RETURNING id, match_key, offer_id, label, created_at",
        )
        .bind(user.0)
        .bind(item.key.as_str())
        .bind(item.offer_id.as_ref().map(|o| o.0.as_str()))
        .bind(&item.label)
        .fetch_one(&self.pool)
        .await
        .map_err(db)?;
        to_saved(&row)
    }

    async fn delete(&self, user: UserId, id: Uuid) -> Result<bool, RepoError> {
        let res = sqlx::query("DELETE FROM saved_matches WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user.0)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(res.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PgUserRepository;
    use crate::users::tests::identity;
    use nirog_application::ports::UserRepository;

    fn item(key: &str, label: &str) -> NewSavedMatch {
        NewSavedMatch {
            key: MatchKey::parse(key).unwrap_or_else(|| unreachable!()),
            offer_id: Some(OfferId("ja:1".into())),
            label: label.into(),
        }
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn saved_matches_are_per_user_upserted_and_cascade(
        pool: PgPool,
    ) -> Result<(), RepoError> {
        let users = PgUserRepository::new(pool.clone());
        let repo = PgSavedMatchRepository::new(pool);
        let asha = users
            .upsert_identity(&identity("a", "a@example.com"))
            .await?
            .id;
        let ravi = users
            .upsert_identity(&identity("r", "r@example.com"))
            .await?
            .id;

        let first = repo
            .save(asha, &item("tablet|standard|x:2mg|unit", "Dolo"))
            .await?;
        let updated = repo
            .save(asha, &item("tablet|standard|x:2mg|unit", "Dolo 650"))
            .await?;
        assert_eq!(first.id, updated.id);
        assert_eq!(updated.label, "Dolo 650");
        repo.save(asha, &item("tablet|standard|y:2mg|unit", "Other"))
            .await?;
        assert_eq!(repo.count(asha).await?, 2);
        assert_eq!(repo.list(ravi).await?, vec![]);

        // Another user cannot delete Asha's entry.
        assert!(!repo.delete(ravi, first.id).await?);
        assert!(repo.delete(asha, first.id).await?);
        assert_eq!(repo.count(asha).await?, 1);

        users.delete(asha).await?;
        assert_eq!(repo.count(asha).await?, 0);
        Ok(())
    }
}
