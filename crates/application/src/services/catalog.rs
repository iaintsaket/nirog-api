use std::sync::Arc;

use nirog_domain::{Comparison, MatchKey, OfferId};

use crate::error::AppError;
use crate::model::{BuyLink, SearchGroup};
use crate::ports::{OfferRepository, SellerLinks};

pub const MIN_QUERY: usize = 2;
pub const MAX_QUERY: usize = 100;
const SEARCH_ROWS: u32 = 200;
const MAX_GROUPS: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComparisonView {
    pub comparison: Comparison,
    /// Buy links, index-aligned with `comparison.offers`.
    pub links: Vec<Vec<BuyLink>>,
}

/// Search and compare medicines across every source.
#[derive(Clone)]
pub struct CatalogService {
    offers: Arc<dyn OfferRepository>,
    sellers: Arc<dyn SellerLinks>,
}

impl std::fmt::Debug for CatalogService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CatalogService")
    }
}

impl CatalogService {
    pub fn new(offers: Arc<dyn OfferRepository>, sellers: Arc<dyn SellerLinks>) -> Self {
        Self { offers, sellers }
    }

    /// Matches grouped by exact-substitution key, in the repository's relevance order.
    pub async fn search(&self, query: &str) -> Result<Vec<SearchGroup>, AppError> {
        let q = query.trim();
        let len = q.chars().count();
        if !(MIN_QUERY..=MAX_QUERY).contains(&len) {
            return Err(AppError::Invalid("query must be 2 to 100 characters"));
        }
        let mut groups: Vec<SearchGroup> = Vec::new();
        for offer in self.offers.search(q, SEARCH_ROWS).await? {
            match groups.iter().position(|g| g.key == offer.key) {
                Some(i) => groups[i].matched.push(offer),
                None if groups.len() < MAX_GROUPS => groups.push(SearchGroup {
                    key: offer.key.clone(),
                    salts_label: offer.salts_label.clone(),
                    matched: vec![offer],
                }),
                None => {}
            }
        }
        Ok(groups)
    }

    pub async fn compare(
        &self,
        key: &str,
        searched: Option<&str>,
    ) -> Result<ComparisonView, AppError> {
        let key = MatchKey::parse(key).ok_or(AppError::Invalid("malformed match key"))?;
        let offers = self.offers.by_key(&key).await?;
        if offers.is_empty() {
            return Err(AppError::NotFound);
        }
        let searched = searched.map(|s| OfferId(s.to_owned()));
        let comparison =
            Comparison::build(offers, searched.as_ref()).map_err(|_| AppError::NotFound)?;
        let links = comparison
            .offers
            .iter()
            .map(|o| self.sellers.links(o))
            .collect();
        Ok(ComparisonView { comparison, links })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{FakeOffers, offer};
    use crate::sellers::DefaultSellerLinks;
    use nirog_domain::SourceKind;

    fn service(offers: FakeOffers) -> CatalogService {
        CatalogService::new(Arc::new(offers), Arc::new(DefaultSellerLinks))
    }

    fn sample() -> FakeOffers {
        FakeOffers::with(vec![
            offer("b1", SourceKind::Brand, "Lipitor 10 Tablet", 20_000, "k1"),
            offer("b2", SourceKind::Brand, "Atorva 10 Tablet", 10_000, "k1"),
            offer(
                "j1",
                SourceKind::JanAushadhi,
                "Atorvastatin Tablets IP 10mg",
                908,
                "k1",
            ),
            offer("b3", SourceKind::Brand, "Dolo 650 Tablet", 3_000, "k2"),
        ])
    }

    #[tokio::test]
    async fn search_groups_by_key() {
        let groups = service(sample()).search("tablet").await.unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].matched.len(), 3);
    }

    #[tokio::test]
    async fn search_rejects_too_short_or_long_queries() {
        let s = service(sample());
        assert!(matches!(s.search(" a ").await, Err(AppError::Invalid(_))));
        assert!(matches!(
            s.search(&"x".repeat(101)).await,
            Err(AppError::Invalid(_))
        ));
    }

    #[tokio::test]
    async fn search_caps_groups() {
        let many = (0..30)
            .map(|i| {
                offer(
                    &format!("o{i}"),
                    SourceKind::Brand,
                    "Same Name",
                    100,
                    &format!("key{i}"),
                )
            })
            .collect();
        let groups = service(FakeOffers::with(many))
            .search("same")
            .await
            .unwrap();
        assert_eq!(groups.len(), MAX_GROUPS);
    }

    #[tokio::test]
    async fn compare_returns_cheapest_first_with_links() {
        let key = crate::fakes::key("k1");
        let view = service(sample())
            .compare(key.as_str(), Some("b1"))
            .await
            .unwrap();
        assert_eq!(view.comparison.cheapest().id.0, "j1");
        assert_eq!(view.links.len(), 3);
        assert_eq!(view.links[0][0].label, "Find a Jan Aushadhi Kendra");
    }

    #[tokio::test]
    async fn compare_errors() {
        let s = service(sample());
        assert!(matches!(
            s.compare("bad", None).await,
            Err(AppError::Invalid(_))
        ));
        assert!(matches!(
            s.compare(crate::fakes::key("none").as_str(), None).await,
            Err(AppError::NotFound)
        ));
    }
}
