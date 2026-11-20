use std::sync::{Arc, Mutex};

use time::{Duration, OffsetDateTime};

use crate::error::AppError;
use crate::model::CatalogStats;
use crate::ports::{Clock, StatsRepository};

/// The figures change only on import, so they are worked out at most this often.
const FRESH_FOR: Duration = Duration::minutes(10);

/// Headline figures for the home screen, cached so a busy page costs one query per 10 minutes.
#[derive(Clone)]
pub struct StatsService {
    repo: Arc<dyn StatsRepository>,
    clock: Arc<dyn Clock>,
    cached: Arc<Mutex<Option<(OffsetDateTime, CatalogStats)>>>,
}

impl std::fmt::Debug for StatsService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StatsService")
    }
}

impl StatsService {
    pub fn new(repo: Arc<dyn StatsRepository>, clock: Arc<dyn Clock>) -> Self {
        Self {
            repo,
            clock,
            cached: Arc::default(),
        }
    }

    pub async fn stats(&self) -> Result<CatalogStats, AppError> {
        let now = self.clock.now();
        // A poisoned lock only means a cached value may be stale; the cache is still usable.
        if let Some((at, s)) = self
            .cached
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
        {
            let age: Duration = now - at;
            if age < FRESH_FOR {
                return Ok(s);
            }
        }
        let fresh = self.repo.stats().await?;
        *self.cached.lock().unwrap_or_else(|p| p.into_inner()) = Some((now, fresh.clone()));
        Ok(fresh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{FakeClock, FakeStats};
    use std::sync::atomic::Ordering;

    #[tokio::test]
    async fn asks_the_database_at_most_once_per_ten_minutes() {
        let repo = Arc::new(FakeStats::default());
        let clock = Arc::new(FakeClock::default());
        let svc = StatsService::new(repo.clone(), clock.clone());

        let first = svc.stats().await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(first.medicines, 349_595);
        clock.advance(Duration::minutes(9));
        assert_eq!(svc.stats().await.ok(), Some(first));
        assert_eq!(repo.0.load(Ordering::SeqCst), 1);

        clock.advance(Duration::minutes(2));
        svc.stats().await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(repo.0.load(Ordering::SeqCst), 2);
    }
}
