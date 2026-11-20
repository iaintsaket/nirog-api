use async_trait::async_trait;
use nirog_application::ports::UserRepository;
use nirog_application::{Identity, RepoError, User, UserId};
use sqlx::{PgPool, Row, postgres::PgRow};

use crate::db;

#[derive(Debug, Clone)]
pub struct PgUserRepository {
    pool: PgPool,
}

impl PgUserRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn to_user(row: &PgRow) -> Result<User, RepoError> {
    Ok(User {
        id: UserId(row.try_get("id").map_err(db)?),
        email: row.try_get("email").map_err(db)?,
        name: row.try_get("name").map_err(db)?,
    })
}

#[async_trait]
impl UserRepository for PgUserRepository {
    async fn upsert_identity(&self, identity: &Identity) -> Result<User, RepoError> {
        let row = sqlx::query(
            "INSERT INTO users (provider, subject, email, name) VALUES ($1, $2, $3, $4)
             ON CONFLICT (provider, subject)
             DO UPDATE SET email = EXCLUDED.email, name = EXCLUDED.name, updated_at = now()
             RETURNING id, email, name",
        )
        .bind(identity.provider)
        .bind(&identity.subject)
        .bind(&identity.email)
        .bind(&identity.name)
        .fetch_one(&self.pool)
        .await
        .map_err(db)?;
        to_user(&row)
    }

    async fn get(&self, id: UserId) -> Result<Option<User>, RepoError> {
        let row = sqlx::query("SELECT id, email, name FROM users WHERE id = $1")
            .bind(id.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;
        row.as_ref().map(to_user).transpose()
    }

    async fn delete(&self, id: UserId) -> Result<bool, RepoError> {
        let res = sqlx::query("DELETE FROM users WHERE id = $1").bind(id.0).execute(&self.pool).await.map_err(db)?;
        Ok(res.rows_affected() > 0)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn identity(subject: &str, email: &str) -> Identity {
        Identity { provider: "google", subject: subject.into(), email: email.into(), name: "Asha".into() }
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn upsert_is_stable_per_subject_and_delete_works(pool: PgPool) -> Result<(), RepoError> {
        let repo = PgUserRepository::new(pool);
        let first = repo.upsert_identity(&identity("sub-1", "a@example.com")).await?;
        let again = repo.upsert_identity(&identity("sub-1", "new@example.com")).await?;
        assert_eq!(first.id, again.id);
        assert_eq!(again.email, "new@example.com");
        let other = repo.upsert_identity(&identity("sub-2", "b@example.com")).await?;
        assert_ne!(first.id, other.id);

        assert_eq!(repo.get(first.id).await?, Some(again));
        assert!(repo.delete(first.id).await?);
        assert!(!repo.delete(first.id).await?);
        assert_eq!(repo.get(first.id).await?, None);
        Ok(())
    }
}
