//! Recently opened medicines: the last 3 per user, newest first.

use nirog_application::ports::{RecentRepository, UserRepository};
use nirog_application::{Identity, NewSavedMatch, RepoError, UserId};
use nirog_domain::{MatchKey, OfferId};
use nirog_persistence::{PgPool, PgRecentRepository, PgUserRepository};

fn identity(subject: &str, email: &str) -> Identity {
    Identity {
        provider: "google",
        subject: subject.into(),
        email: email.into(),
        name: "Asha".into(),
        picture: None,
    }
}

fn item(n: u32) -> NewSavedMatch {
    NewSavedMatch {
        key: MatchKey::parse(&format!("tablet|standard|x:{n}mg|unit"))
            .unwrap_or_else(|| unreachable!()),
        offer_id: Some(OfferId(format!("ja:{n}"))),
        label: format!("Medicine {n}"),
    }
}

async fn labels(repo: &PgRecentRepository, user: UserId) -> Result<Vec<String>, RepoError> {
    Ok(repo
        .list(user, 3)
        .await?
        .into_iter()
        .map(|r| r.label)
        .collect())
}

#[sqlx::test(migrator = "nirog_persistence::MIGRATOR")]
async fn keeps_the_last_three_opened_newest_first_per_user(pool: PgPool) -> Result<(), RepoError> {
    let users = PgUserRepository::new(pool.clone());
    let repo = PgRecentRepository::new(pool.clone());
    let asha = users
        .upsert_identity(&identity("a", "a@example.com"))
        .await?
        .id;
    let ravi = users
        .upsert_identity(&identity("r", "r@example.com"))
        .await?
        .id;

    for n in 1..=4 {
        repo.record(asha, &item(n), 3).await?;
    }
    repo.record(ravi, &item(9), 3).await?;
    assert_eq!(
        labels(&repo, asha).await?,
        ["Medicine 4", "Medicine 3", "Medicine 2"]
    );
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM recent_medicines WHERE user_id = $1")
            .bind(asha.0)
            .fetch_one(&pool)
            .await
            .map_err(|e| RepoError(e.to_string()))?;
    assert_eq!(stored, 3, "older entries are deleted, not just hidden");

    // Opening one again moves it to the top without duplicating it.
    repo.record(asha, &item(2), 3).await?;
    assert_eq!(
        labels(&repo, asha).await?,
        ["Medicine 2", "Medicine 4", "Medicine 3"]
    );
    assert_eq!(labels(&repo, ravi).await?, ["Medicine 9"]);

    repo.clear(asha).await?;
    assert!(labels(&repo, asha).await?.is_empty());
    assert_eq!(labels(&repo, ravi).await?.len(), 1, "clear is per user");

    users.delete(ravi).await?;
    assert!(
        labels(&repo, ravi).await?.is_empty(),
        "deleted with the account"
    );
    Ok(())
}
