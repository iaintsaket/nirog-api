use std::sync::Arc;

use nirog_domain::{MatchKey, OfferId};
use uuid::Uuid;

use crate::error::AppError;
use crate::model::{NewSavedMatch, SavedMatch, UserId};
use crate::ports::{OfferRepository, SavedMatchRepository};

pub const MAX_SAVED_PER_USER: u64 = 200;
const MAX_LABEL: usize = 120;

/// A signed-in user's saved medicine matches.
#[derive(Clone)]
pub struct SavedMatchService {
    saved: Arc<dyn SavedMatchRepository>,
    offers: Arc<dyn OfferRepository>,
}

impl std::fmt::Debug for SavedMatchService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SavedMatchService")
    }
}

impl SavedMatchService {
    pub fn new(saved: Arc<dyn SavedMatchRepository>, offers: Arc<dyn OfferRepository>) -> Self {
        Self { saved, offers }
    }

    pub async fn list(&self, user: UserId) -> Result<Vec<SavedMatch>, AppError> {
        Ok(self.saved.list(user).await?)
    }

    pub async fn save(&self, user: UserId, key: &str, offer_id: Option<&str>, label: &str) -> Result<SavedMatch, AppError> {
        let key = MatchKey::parse(key).ok_or(AppError::Invalid("malformed match key"))?;
        let label = label.trim();
        if label.is_empty() || label.chars().count() > MAX_LABEL || label.chars().any(char::is_control) {
            return Err(AppError::Invalid("label must be 1 to 120 characters"));
        }
        if offer_id.is_some_and(|id| id.is_empty() || id.len() > 64) {
            return Err(AppError::Invalid("bad offer id"));
        }
        if !self.offers.key_exists(&key).await? {
            return Err(AppError::NotFound);
        }
        if self.saved.count(user).await? >= MAX_SAVED_PER_USER {
            return Err(AppError::Limit("too many saved matches"));
        }
        let item = NewSavedMatch { key, offer_id: offer_id.map(|s| OfferId(s.to_owned())), label: label.to_owned() };
        Ok(self.saved.save(user, &item).await?)
    }

    pub async fn delete(&self, user: UserId, id: Uuid) -> Result<(), AppError> {
        if self.saved.delete(user, id).await? { Ok(()) } else { Err(AppError::NotFound) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{FakeOffers, FakeSaved, key, offer};
    use nirog_domain::SourceKind;

    fn service() -> (SavedMatchService, UserId) {
        let offers = FakeOffers::with(vec![offer("b1", SourceKind::Brand, "Lipitor", 100, "k1")]);
        (SavedMatchService::new(Arc::new(FakeSaved::default()), Arc::new(offers)), UserId(Uuid::new_v4()))
    }

    #[tokio::test]
    async fn save_list_delete() {
        let (s, user) = service();
        let k = key("k1");
        let saved = s.save(user, k.as_str(), Some("b1"), " Mum's BP tablet ").await.unwrap();
        assert_eq!(saved.label, "Mum's BP tablet");
        assert_eq!(s.list(user).await.unwrap().len(), 1);
        // other users cannot see or delete it
        let other = UserId(Uuid::new_v4());
        assert!(s.list(other).await.unwrap().is_empty());
        assert!(matches!(s.delete(other, saved.id).await, Err(AppError::NotFound)));
        s.delete(user, saved.id).await.unwrap();
        assert!(s.list(user).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn validates_input() {
        let (s, user) = service();
        let k = key("k1");
        assert!(matches!(s.save(user, "bad", None, "x").await, Err(AppError::Invalid(_))));
        assert!(matches!(s.save(user, k.as_str(), None, "  ").await, Err(AppError::Invalid(_))));
        assert!(matches!(s.save(user, k.as_str(), None, &"x".repeat(121)).await, Err(AppError::Invalid(_))));
        assert!(matches!(s.save(user, k.as_str(), None, "a\u{0}b").await, Err(AppError::Invalid(_))));
        assert!(matches!(s.save(user, k.as_str(), Some(""), "x").await, Err(AppError::Invalid(_))));
        assert!(matches!(s.save(user, key("missing").as_str(), None, "x").await, Err(AppError::NotFound)));
    }

    #[tokio::test]
    async fn enforces_per_user_limit() {
        let (s, user) = service();
        let saved = FakeSaved::default();
        saved.fill(user, MAX_SAVED_PER_USER);
        let offers = FakeOffers::with(vec![offer("b1", SourceKind::Brand, "Lipitor", 100, "k1")]);
        let s2 = SavedMatchService::new(Arc::new(saved), Arc::new(offers));
        assert!(matches!(s2.save(user, key("k1").as_str(), None, "x").await, Err(AppError::Limit(_))));
        drop(s);
    }
}
