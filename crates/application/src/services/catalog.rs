use std::collections::HashMap;
use std::sync::Arc;

use nirog_domain::{Comparison, MatchKey, Offer, OfferId, SourceKind};

use crate::buy::{NoSources, SourceLink, for_offer};
use crate::error::AppError;
use crate::model::{BrandAlias, BuyLink, SearchGroup};
use crate::ports::{
    BrandAliases, Clock, OfferRepository, SellerLinks, SourceProfiles, StockSource, SystemClock,
};
use crate::stock::{NoStock, VendorStatus, sold_out_everywhere};

pub const MIN_QUERY: usize = 2;
pub const MAX_QUERY: usize = 100;
const SEARCH_ROWS: u32 = 200;
const MAX_GROUPS: usize = 20;
pub const MAX_COMPARED: usize = 50;
const MAX_SUGGESTIONS: u32 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComparisonView {
    /// At most [`MAX_COMPARED`] offers (plus the searched one).
    pub comparison: Comparison,
    /// How many exact substitutes exist in total.
    pub total: usize,
    /// Buy links, index-aligned with `comparison.offers`.
    pub links: Vec<Vec<BuyLink>>,
    /// Stock at each online pharmacy, index-aligned with `comparison.offers`; empty when unchecked.
    pub stock: Vec<Vec<VendorStatus>>,
    /// Display name and buy redirect of each `stock` entry, index-aligned with `stock`.
    pub sources: Vec<Vec<SourceLink>>,
}

/// Search and compare medicines across every source.
#[derive(Clone)]
pub struct CatalogService {
    offers: Arc<dyn OfferRepository>,
    sellers: Arc<dyn SellerLinks>,
    aliases: Arc<dyn BrandAliases>,
    stock: Arc<dyn StockSource>,
    clock: Arc<dyn Clock>,
    sources: Arc<dyn SourceProfiles>,
}

impl std::fmt::Debug for CatalogService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CatalogService")
    }
}

impl CatalogService {
    pub fn new(
        offers: Arc<dyn OfferRepository>,
        sellers: Arc<dyn SellerLinks>,
        aliases: Arc<dyn BrandAliases>,
    ) -> Self {
        Self {
            offers,
            sellers,
            aliases,
            stock: Arc::new(NoStock),
            clock: Arc::new(SystemClock),
            sources: Arc::new(NoSources),
        }
    }

    /// Names sources and adds buy redirects from `sources`: each listing gets one, and a brand
    /// gets a search link at every source with a search page but no listing of it.
    pub fn with_sources(mut self, sources: Arc<dyn SourceProfiles>) -> Self {
        self.sources = sources;
        self
    }

    /// Reads online stock from `stock`: compare shows it, and compare and search hide brand
    /// medicines that every pharmacy lists as out of stock.
    pub fn with_stock(mut self, stock: Arc<dyn StockSource>, clock: Arc<dyn Clock>) -> Self {
        self.stock = stock;
        self.clock = clock;
        self
    }

    /// For a brand we have no price for (e.g. "Disprin"): the brand, and the
    /// medicines with the same salt, the exact strength and form first.
    pub async fn same_salt_as_brand(
        &self,
        query: &str,
    ) -> Result<Option<(BrandAlias, Vec<SearchGroup>)>, AppError> {
        let Some(alias) = self.aliases.find(query.trim()) else {
            return Ok(None);
        };
        let mut groups = self.search(&alias.search).await?;
        if let Some(i) = groups.iter().position(|g| g.key == alias.key) {
            let exact = groups.remove(i);
            groups.insert(0, exact);
        }
        Ok(Some((alias, groups)))
    }

    /// Matches grouped by exact-substitution key, in the repository's relevance order.
    pub async fn search(&self, query: &str) -> Result<Vec<SearchGroup>, AppError> {
        let q = query.trim();
        let len = q.chars().count();
        if !(MIN_QUERY..=MAX_QUERY).contains(&len) {
            return Err(AppError::Invalid("query must be 2 to 100 characters"));
        }
        let found = self.offers.search(q, SEARCH_ROWS).await?;
        let stock = self.stock_of(&found).await?;
        let mut groups: Vec<SearchGroup> = Vec::new();
        for offer in found {
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
        for g in &mut groups {
            g.matched = hide_sold_out(std::mem::take(&mut g.matched), &stock, None);
        }
        Ok(groups)
    }

    /// Up to [`MAX_SUGGESTIONS`] names close to a query that matched nothing.
    pub async fn closest(&self, query: &str) -> Result<Vec<String>, AppError> {
        Ok(self
            .offers
            .closest_names(query.trim(), MAX_SUGGESTIONS)
            .await?)
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
        let stock = self.stock_of(&offers).await?;
        let offers = hide_sold_out(offers, &stock, searched.as_ref());
        let total = offers.len();
        let comparison = Comparison::build(offers, searched.as_ref())
            .map_err(|_| AppError::NotFound)?
            .top(MAX_COMPARED);
        let profiles = self.sources.profiles().await?;
        let n = comparison.offers.len();
        let (mut links, mut stocks, mut sources) = (
            Vec::with_capacity(n),
            Vec::with_capacity(n),
            Vec::with_capacity(n),
        );
        for o in &comparison.offers {
            let statuses = stock.get(&o.id).cloned().unwrap_or_default();
            let (entries, search) = for_offer(o, &statuses, &profiles);
            let mut offer_links = self.sellers.links(o);
            offer_links.extend(search);
            links.push(offer_links);
            stocks.push(statuses);
            sources.push(entries);
        }
        Ok(ComparisonView {
            comparison,
            total,
            links,
            stock: stocks,
            sources,
        })
    }
}

impl CatalogService {
    /// Online stock for `offers`, in one read.
    async fn stock_of(
        &self,
        offers: &[Offer],
    ) -> Result<HashMap<OfferId, Vec<VendorStatus>>, AppError> {
        let ids: Vec<OfferId> = offers.iter().map(|o| o.id.clone()).collect();
        Ok(self.stock.check(&ids, self.clock.now()).await?)
    }
}

/// Drops brand offers that every pharmacy lists as out of stock, except `keep` (the one the
/// user searched for). If that would leave nothing, returns `offers` unchanged.
fn hide_sold_out(
    offers: Vec<Offer>,
    stock: &HashMap<OfferId, Vec<VendorStatus>>,
    keep: Option<&OfferId>,
) -> Vec<Offer> {
    let hidden = |o: &Offer| {
        o.source == SourceKind::Brand
            && Some(&o.id) != keep
            && stock.get(&o.id).is_some_and(|s| sold_out_everywhere(s))
    };
    if offers.iter().all(hidden) {
        return offers;
    }
    offers.into_iter().filter(|o| !hidden(o)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{FakeAliases, FakeOffers, key, offer};
    use crate::model::BrandAlias;
    use crate::sellers::DefaultSellerLinks;
    use nirog_domain::SourceKind;

    fn service(offers: FakeOffers) -> CatalogService {
        CatalogService::new(
            Arc::new(offers),
            Arc::new(DefaultSellerLinks),
            Arc::new(FakeAliases::default()),
        )
    }

    #[tokio::test]
    async fn missing_brand_shows_same_salt_with_exact_strength_first() {
        let alias = |k: &str| BrandAlias {
            brand: "Disprin".into(),
            key: key(k),
            salts_label: "Aspirin 350mg".into(),
            search: "aspirin".into(),
        };
        let svc = |a| {
            CatalogService::new(
                Arc::new(FakeOffers::with(vec![])),
                Arc::new(DefaultSellerLinks),
                Arc::new(FakeAliases(vec![a])),
            )
        };
        let with = |a| {
            CatalogService::new(
                Arc::new(FakeOffers::with(sample_aspirin())),
                Arc::new(DefaultSellerLinks),
                Arc::new(FakeAliases(vec![a])),
            )
        };

        let (found, groups) = with(alias("aspirin350"))
            .same_salt_as_brand("Disprin 350")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.brand, "Disprin");
        let keys: Vec<_> = groups.iter().map(|g| g.key.clone()).collect();
        assert_eq!(keys, [key("aspirin350"), key("aspirin")]);
        // Not a known brand, or a known brand with no priced medicine yet.
        assert!(
            with(alias("aspirin350"))
                .same_salt_as_brand("dolo")
                .await
                .unwrap()
                .is_none()
        );
        let (_, none) = svc(alias("aspirin350"))
            .same_salt_as_brand("disprin")
            .await
            .unwrap()
            .unwrap();
        assert!(none.is_empty());
    }

    fn sample_aspirin() -> Vec<nirog_domain::Offer> {
        vec![
            offer("b1", SourceKind::Brand, "Ecosprin 75", 100, "aspirin"),
            offer("b2", SourceKind::Brand, "Aspent 350", 200, "aspirin350"),
            offer("b3", SourceKind::Brand, "Dolo 650", 300, "paracetamol"),
        ]
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
