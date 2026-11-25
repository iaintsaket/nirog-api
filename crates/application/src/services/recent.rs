use std::sync::Arc;

use crate::error::AppError;
use crate::model::{RecentMedicine, UserId};
use crate::ports::{OfferRepository, RecentRepository};
use crate::services::saved::parse_item;

/// How many recently opened medicines are kept per user.
pub const RECENT_LIMIT: u32 = 3;

/// The last few medicines a signed-in user opened.
#[derive(Clone)]
pub struct RecentService {
    recent: Arc<dyn RecentRepository>,
    offers: Arc<dyn OfferRepository>,
}

impl std::fmt::Debug for RecentService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RecentService")
    }
}

impl RecentService {
    pub fn new(recent: Arc<dyn RecentRepository>, offers: Arc<dyn OfferRepository>) -> Self {
        Self { recent, offers }
    }

    pub async fn list(&self, user: UserId) -> Result<Vec<RecentMedicine>, AppError> {
        Ok(self.recent.list(user, RECENT_LIMIT).await?)
    }

    /// Moves the medicine to the top of the user's recent list.
    pub async fn record(
        &self,
        user: UserId,
        key: &str,
        offer_id: Option<&str>,
        label: &str,
    ) -> Result<(), AppError> {
        let item = parse_item(key, offer_id, label)?;
        if !self.offers.key_exists(&item.key).await? {
            return Err(AppError::NotFound);
        }
        Ok(self.recent.record(user, &item, RECENT_LIMIT).await?)
    }

    pub async fn clear(&self, user: UserId) -> Result<(), AppError> {
        Ok(self.recent.clear(user).await?)
    }
}
