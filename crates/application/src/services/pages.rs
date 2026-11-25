use std::sync::Arc;

use nirog_domain::{Comparison, MatchKey};

use crate::error::AppError;
use crate::ports::{MedicinePages, SellerLinks};
use crate::services::catalog::ComparisonView;

/// Match keys per batch: a full walk of the catalogue is about 20 requests.
pub const KEYS_PER_BATCH: u32 = 500;
/// Offers kept per page; the page links to the live comparison for the rest.
pub const OFFERS_PER_PAGE: usize = 20;
/// Distinct medicine names listed per page, so it can be found by any of them.
pub const NAMES_PER_PAGE: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MedicinePage {
    /// Cheapest [`OFFERS_PER_PAGE`] offers; the saving is against the costliest of all.
    pub view: ComparisonView,
    /// Distinct names of every offer with this key, cheapest first, at most [`NAMES_PER_PAGE`].
    pub names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagesBatch {
    pub pages: Vec<MedicinePage>,
    /// Pass back as `after` for the next batch; `None` when the walk is done.
    pub next: Option<MatchKey>,
}

/// Every medicine worth a page of its own (at least one substitute), in batches, for the
/// website's build. Read-only, and a full walk costs a handful of indexed queries.
#[derive(Clone)]
pub struct PagesService {
    repo: Arc<dyn MedicinePages>,
    sellers: Arc<dyn SellerLinks>,
}

impl std::fmt::Debug for PagesService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PagesService")
    }
}

impl PagesService {
    pub fn new(repo: Arc<dyn MedicinePages>, sellers: Arc<dyn SellerLinks>) -> Self {
        Self { repo, sellers }
    }

    pub async fn batch(&self, after: Option<&str>) -> Result<PagesBatch, AppError> {
        let after = match after {
            Some(raw) => {
                Some(MatchKey::parse(raw).ok_or(AppError::Invalid("malformed match key"))?)
            }
            None => None,
        };
        let groups = self.repo.after(after.as_ref(), KEYS_PER_BATCH).await?;
        let full = groups.len() == KEYS_PER_BATCH as usize;
        let mut pages = Vec::with_capacity(groups.len());
        for offers in groups {
            let total = offers.len();
            let mut names: Vec<String> = Vec::new();
            for o in &offers {
                if names.len() < NAMES_PER_PAGE && !names.contains(&o.name) {
                    names.push(o.name.clone());
                }
            }
            let Ok(comparison) = Comparison::build(offers, None) else {
                continue;
            };
            let comparison = comparison.top(OFFERS_PER_PAGE);
            let links = comparison
                .offers
                .iter()
                .map(|o| self.sellers.links(o))
                .collect();
            // Pre-built pages are cached for days: they never carry stock.
            let stock = vec![Vec::new(); comparison.offers.len()];
            pages.push(MedicinePage {
                view: ComparisonView {
                    comparison,
                    total,
                    links,
                    stock,
                },
                names,
            });
        }
        let next = if full {
            pages
                .last()
                .map(|p| p.view.comparison.cheapest().key.clone())
        } else {
            None
        };
        Ok(PagesBatch { pages, next })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{FakeOffers, offer};
    use crate::sellers::DefaultSellerLinks;
    use nirog_domain::SourceKind;

    fn service(offers: Vec<nirog_domain::Offer>) -> PagesService {
        PagesService::new(
            Arc::new(FakeOffers::with(offers)),
            Arc::new(DefaultSellerLinks),
        )
    }

    #[tokio::test]
    async fn pages_cover_keys_with_a_substitute_cheapest_first_with_every_name() {
        let s = service(vec![
            offer("b1", SourceKind::Brand, "Dolo 650", 300, "paracetamol"),
            offer(
                "j1",
                SourceKind::JanAushadhi,
                "Paracetamol 650",
                100,
                "paracetamol",
            ),
            offer("b2", SourceKind::Brand, "Calpol 650", 250, "paracetamol"),
            offer("b3", SourceKind::Brand, "Lonely", 100, "solo"),
        ]);
        let batch = s.batch(None).await.unwrap();
        assert_eq!(
            batch.pages.len(),
            1,
            "a key with one offer has nothing to compare"
        );
        let page = &batch.pages[0];
        assert_eq!(page.view.total, 3);
        assert_eq!(page.view.comparison.cheapest().name, "Paracetamol 650");
        assert_eq!(page.view.links.len(), 3);
        assert_eq!(page.names, ["Paracetamol 650", "Calpol 650", "Dolo 650"]);
        assert!(page.view.comparison.saving_per_pack.0 > 0);
        assert_eq!(batch.next, None, "a short batch is the last one");
    }

    #[tokio::test]
    async fn a_full_batch_points_at_the_next_and_bad_cursors_are_rejected() {
        let mut offers = Vec::new();
        for i in 0..=KEYS_PER_BATCH {
            let k = format!("salt{i:04}");
            offers.push(offer(&format!("a{i}"), SourceKind::Brand, "A", 100, &k));
            offers.push(offer(&format!("b{i}"), SourceKind::Brand, "B", 200, &k));
        }
        let s = service(offers);
        let first = s.batch(None).await.unwrap();
        assert_eq!(first.pages.len(), KEYS_PER_BATCH as usize);
        let next = first.next.expect("more to come");
        let rest = s.batch(Some(next.as_str())).await.unwrap();
        assert_eq!((rest.pages.len(), rest.next), (1, None));
        assert!(matches!(
            s.batch(Some("nope")).await,
            Err(AppError::Invalid(_))
        ));
    }
}
