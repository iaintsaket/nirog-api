use std::collections::HashSet;

use crate::error::AppError;
use crate::ports::{OfferRepository, PriceSource};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportReport {
    pub imported: u64,
    /// Input rows the source could not use (no price, pack or readable strength).
    pub skipped: usize,
    /// Repeat listings of a product already in this feed; the first one is kept.
    pub duplicates: usize,
}

/// Load one source and add its prices. Nothing is deleted, so a feed that
/// drops a product leaves its last known price in place.
/// Refuses an empty feed, which usually means the file format changed.
pub async fn import_source(
    source: &dyn PriceSource,
    repo: &dyn OfferRepository,
) -> Result<ImportReport, AppError> {
    let (offers, skipped) = source
        .load()
        .map_err(|e| AppError::Repo(crate::error::RepoError(e.to_string())))?;
    if offers.is_empty() {
        return Err(AppError::Invalid("source returned no usable offers"));
    }
    let total = offers.len();
    let mut seen = HashSet::new();
    let unique: Vec<_> = offers
        .into_iter()
        .filter(|o| seen.insert(o.identity()))
        .collect();
    let imported = repo.record(&source.info(), &unique).await?;
    Ok(ImportReport {
        imported,
        skipped,
        duplicates: total - unique.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{FakeOffers, FakeSource, offer};
    use crate::ports::OfferRepository;
    use nirog_domain::SourceKind;

    #[tokio::test]
    async fn imports_once_per_product_and_reports() {
        let repo = FakeOffers::default();
        let a = offer("j1", SourceKind::JanAushadhi, "A", 1, "k");
        let mut dup = offer("j2", SourceKind::JanAushadhi, "a", 9, "k");
        dup.manufacturer = a.manufacturer.clone();
        let b = offer("j3", SourceKind::JanAushadhi, "B", 2, "k");
        let src = FakeSource(Ok(vec![a.clone(), dup, b.clone()]));
        assert_eq!(
            import_source(&src, &repo).await.unwrap(),
            ImportReport {
                imported: 2,
                skipped: 2,
                duplicates: 1
            }
        );
        // The first listing of a product wins within one source.
        let key = a.key.clone();
        assert_eq!(repo.by_key(&key).await.unwrap(), vec![a, b]);
    }

    #[tokio::test]
    async fn refuses_empty_feed_and_surfaces_errors() {
        let repo = FakeOffers::default();
        assert!(matches!(
            import_source(&FakeSource(Ok(vec![])), &repo).await,
            Err(AppError::Invalid(_))
        ));
        assert!(matches!(
            import_source(&FakeSource(Err("boom".into())), &repo).await,
            Err(AppError::Repo(_))
        ));
    }
}
