use std::sync::Arc;

use nirog_domain::OfferId;

use crate::buy::destination;
use crate::error::AppError;
use crate::ports::{BuyRedirects, Clock};
use crate::stock::Vendor;

/// Sends buyers on to online pharmacies through `/v1/go`, and counts how many went.
#[derive(Clone)]
pub struct BuyService {
    repo: Arc<dyn BuyRedirects>,
    clock: Arc<dyn Clock>,
}

impl std::fmt::Debug for BuyService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BuyService")
    }
}

impl BuyService {
    pub fn new(repo: Arc<dyn BuyRedirects>, clock: Arc<dyn Clock>) -> Self {
        Self { repo, clock }
    }

    /// The URL to send a buyer of `offer` at `source` to. Counts the click; a failed count
    /// never blocks the buyer.
    pub async fn go(&self, offer: &str, source: &str) -> Result<String, AppError> {
        let vendor = Vendor::parse(source).map_err(|_| AppError::Invalid("unknown source"))?;
        let target = self
            .repo
            .target(&OfferId(offer.to_owned()), vendor)
            .await?
            .ok_or(AppError::NotFound)?;
        let url = destination(&target).ok_or(AppError::NotFound)?;
        if let Err(e) = self.repo.count_click(self.clock.now().date(), vendor).await {
            tracing::warn!(error = %e, "buy click not counted");
        }
        Ok(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buy::BuyTarget;
    use crate::fakes::{FakeBuy, FakeClock};

    fn service(target: Option<BuyTarget>, fail_count: bool) -> (BuyService, Arc<FakeBuy>) {
        let repo = Arc::new(FakeBuy::new(target, fail_count));
        (
            BuyService::new(repo.clone(), Arc::new(FakeClock::default())),
            repo,
        )
    }

    fn listed(url: &str) -> Option<BuyTarget> {
        Some(BuyTarget {
            offer_name: "Dolo 650 Tablet".into(),
            listing_url: Some(url.into()),
            search_url_template: None,
        })
    }

    #[tokio::test]
    async fn go_redirects_to_stored_destinations_and_counts_each_click() {
        let page = "https://shop.example/p/dolo-650";
        let (svc, repo) = service(listed(page), false);
        assert_eq!(svc.go("brand:1", "source_b").await.unwrap(), page);
        let today = FakeClock::default().now().date();
        assert_eq!(repo.clicks(), [(today, Vendor::SourceB)]);

        assert!(matches!(
            svc.go("brand:1", "source_x").await,
            Err(AppError::Invalid(_))
        ));
        let (missing, _) = service(None, false);
        assert!(matches!(
            missing.go("nope", "source_b").await,
            Err(AppError::NotFound)
        ));
        // Nowhere safe to go: not found, and not counted.
        let (plain, plain_repo) = service(listed("http://shop.example/p/1"), false);
        assert!(matches!(
            plain.go("brand:1", "source_b").await,
            Err(AppError::NotFound)
        ));
        assert!(plain_repo.clicks().is_empty());
    }

    #[tokio::test]
    async fn a_failed_count_never_blocks_the_buyer() {
        let page = "https://shop.example/p/dolo-650";
        let (svc, _) = service(listed(page), true);
        assert_eq!(svc.go("brand:1", "source_b").await.unwrap(), page);
    }
}
