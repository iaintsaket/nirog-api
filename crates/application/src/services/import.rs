use crate::error::AppError;
use crate::ports::{OfferRepository, PriceSource};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportReport {
    pub imported: u64,
    pub skipped: usize,
}

/// Load one source and atomically replace its offers.
/// Refuses to wipe a source when the feed comes back empty.
pub async fn import_source(source: &dyn PriceSource, repo: &dyn OfferRepository) -> Result<ImportReport, AppError> {
    let (offers, skipped) = source.load().map_err(|e| AppError::Repo(crate::error::RepoError(e.to_string())))?;
    if offers.is_empty() {
        return Err(AppError::Invalid("source returned no usable offers"));
    }
    let imported = repo.replace_source(source.kind(), &offers).await?;
    Ok(ImportReport { imported, skipped })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{FakeOffers, FakeSource, offer};
    use nirog_domain::SourceKind;

    #[tokio::test]
    async fn imports_and_reports() {
        let repo = FakeOffers::default();
        let src = FakeSource(Ok(vec![offer("j1", SourceKind::JanAushadhi, "A", 1, "k")]));
        assert_eq!(import_source(&src, &repo).await.unwrap(), ImportReport { imported: 1, skipped: 2 });
    }

    #[tokio::test]
    async fn refuses_empty_feed_and_surfaces_errors() {
        let repo = FakeOffers::default();
        assert!(matches!(import_source(&FakeSource(Ok(vec![])), &repo).await, Err(AppError::Invalid(_))));
        assert!(matches!(import_source(&FakeSource(Err("boom".into())), &repo).await, Err(AppError::Repo(_))));
    }
}
