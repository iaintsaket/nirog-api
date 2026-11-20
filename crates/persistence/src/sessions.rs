use async_trait::async_trait;
use nirog_application::ports::{LoginStateRepository, SessionRepository};
use nirog_application::security::TokenHash;
use nirog_application::{RepoError, UserId};
use sqlx::PgPool;
use time::OffsetDateTime;

use crate::db;

#[derive(Debug, Clone)]
pub struct PgSessionRepository {
    pool: PgPool,
}

impl PgSessionRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Housekeeping: removes expired sessions and login states. Returns rows removed.
    pub async fn purge_expired(&self, now: OffsetDateTime) -> Result<u64, RepoError> {
        let a = sqlx::query("DELETE FROM sessions WHERE expires_at <= $1")
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        let b = sqlx::query("DELETE FROM login_states WHERE expires_at <= $1")
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(a.rows_affected() + b.rows_affected())
    }
}

#[async_trait]
impl SessionRepository for PgSessionRepository {
    async fn create(
        &self,
        hash: &TokenHash,
        user: UserId,
        expires_at: OffsetDateTime,
    ) -> Result<(), RepoError> {
        sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ($1, $2, $3)")
            .bind(&hash.0[..])
            .bind(user.0)
            .bind(expires_at)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }

    async fn user_for(
        &self,
        hash: &TokenHash,
        now: OffsetDateTime,
    ) -> Result<Option<UserId>, RepoError> {
        let id: Option<uuid::Uuid> = sqlx::query_scalar(
            "SELECT user_id FROM sessions WHERE token_hash = $1 AND expires_at > $2",
        )
        .bind(&hash.0[..])
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(id.map(UserId))
    }

    async fn delete(&self, hash: &TokenHash) -> Result<(), RepoError> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(&hash.0[..])
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct PgLoginStateRepository {
    pool: PgPool,
}

impl PgLoginStateRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl LoginStateRepository for PgLoginStateRepository {
    async fn put(
        &self,
        state: &TokenHash,
        pkce_verifier: &str,
        expires_at: OffsetDateTime,
    ) -> Result<(), RepoError> {
        sqlx::query(
            "INSERT INTO login_states (state_hash, pkce_verifier, expires_at) VALUES ($1, $2, $3)",
        )
        .bind(&state.0[..])
        .bind(pkce_verifier)
        .bind(expires_at)
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(())
    }

    /// Single use: the row is deleted whether or not it has expired.
    async fn take(
        &self,
        state: &TokenHash,
        now: OffsetDateTime,
    ) -> Result<Option<String>, RepoError> {
        let row: Option<(String, OffsetDateTime)> = sqlx::query_as(
            "DELETE FROM login_states WHERE state_hash = $1 RETURNING pkce_verifier, expires_at",
        )
        .bind(&state.0[..])
        .fetch_optional(&self.pool)
        .await
        .map_err(db)?;
        Ok(row.and_then(|(verifier, expires)| (expires > now).then_some(verifier)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PgUserRepository;
    use crate::users::tests::identity;
    use nirog_application::ports::UserRepository;
    use time::Duration;

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn sessions_expire_and_cascade_with_user(pool: PgPool) -> Result<(), RepoError> {
        let users = PgUserRepository::new(pool.clone());
        let repo = PgSessionRepository::new(pool);
        let user = users
            .upsert_identity(&identity("s", "s@example.com"))
            .await?;
        let now = OffsetDateTime::now_utc();
        let (live, old) = (TokenHash::of("live"), TokenHash::of("old"));
        repo.create(&live, user.id, now + Duration::hours(1))
            .await?;
        repo.create(&old, user.id, now - Duration::seconds(1))
            .await?;

        assert_eq!(repo.user_for(&live, now).await?, Some(user.id));
        assert_eq!(repo.user_for(&old, now).await?, None);
        assert_eq!(repo.user_for(&TokenHash::of("nope"), now).await?, None);
        assert_eq!(repo.purge_expired(now).await?, 1);

        repo.delete(&live).await?;
        assert_eq!(repo.user_for(&live, now).await?, None);

        repo.create(&live, user.id, now + Duration::hours(1))
            .await?;
        users.delete(user.id).await?;
        assert_eq!(repo.user_for(&live, now).await?, None);
        Ok(())
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn login_state_is_single_use_and_expires(pool: PgPool) -> Result<(), RepoError> {
        let repo = PgLoginStateRepository::new(pool);
        let now = OffsetDateTime::now_utc();
        let (s1, s2) = (TokenHash::of("s1"), TokenHash::of("s2"));
        repo.put(&s1, "verifier", now + Duration::minutes(10))
            .await?;
        repo.put(&s2, "stale", now - Duration::seconds(1)).await?;

        assert_eq!(repo.take(&s1, now).await?.as_deref(), Some("verifier"));
        assert_eq!(repo.take(&s1, now).await?, None);
        assert_eq!(repo.take(&s2, now).await?, None);
        assert!(repo.put(&s1, "again", now).await.is_ok());
        Ok(())
    }
}
