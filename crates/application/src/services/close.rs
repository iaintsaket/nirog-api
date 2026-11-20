use std::sync::Arc;

use nirog_domain::{MatchKey, differences};

use crate::error::AppError;
use crate::model::CloseMatch;
use crate::ports::NearbyOffers;

/// How many candidate formulas to look at, how many to return, and how far apart they may be.
const CANDIDATES: u32 = 300;
const SHOWN: usize = 6;
const MAX_DIFFERENCES: usize = 2;

/// Nearby formulas for a medicine with no cheaper exact substitute, closest first, then cheapest.
#[derive(Clone)]
pub struct CloseMatchService {
    nearby: Arc<dyn NearbyOffers>,
}

impl std::fmt::Debug for CloseMatchService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CloseMatchService")
    }
}

impl CloseMatchService {
    pub fn new(nearby: Arc<dyn NearbyOffers>) -> Self {
        Self { nearby }
    }

    pub async fn close(&self, key: &str) -> Result<Vec<CloseMatch>, AppError> {
        let key = MatchKey::parse(key).ok_or(AppError::Invalid("key"))?;
        let mut out: Vec<CloseMatch> = self
            .nearby
            .nearby(&key, CANDIDATES)
            .await?
            .into_iter()
            .filter_map(|group| {
                let d = differences(&key, &group.key)?;
                (1..=MAX_DIFFERENCES)
                    .contains(&d.len())
                    .then_some(CloseMatch {
                        group,
                        differences: d,
                    })
            })
            .collect();
        out.sort_by_key(|m| {
            (
                m.differences.len(),
                m.group.cheapest.unit_price_millipaise(),
            )
        });
        out.truncate(SHOWN);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::RepoError;
    use crate::model::NearbyGroup;
    use async_trait::async_trait;
    use nirog_domain::{Offer, OfferId, Pack, PackUnit, Paise, SourceKind};

    struct Fixed(Vec<NearbyGroup>);
    #[async_trait]
    impl NearbyOffers for Fixed {
        async fn nearby(&self, _: &MatchKey, _: u32) -> Result<Vec<NearbyGroup>, RepoError> {
            Ok(self.0.clone())
        }
    }

    fn group(key: &str, paise: i64) -> NearbyGroup {
        let key = MatchKey::parse(key).unwrap_or_else(|| unreachable!());
        NearbyGroup {
            cheapest: Offer {
                id: OfferId(key.as_str().into()),
                source: SourceKind::Brand,
                name: "x".into(),
                manufacturer: "m".into(),
                price: Paise(paise),
                pack: Pack {
                    count: 1,
                    unit: PackUnit::G,
                },
                key: key.clone(),
                salts_label: String::new(),
            },
            key,
            offers: 1,
        }
    }

    #[tokio::test]
    async fn keeps_formulas_one_or_two_steps_away_closest_then_cheapest() {
        let ours = "topical|standard|a:1%+b:2%+c:3%|g";
        let svc = CloseMatchService::new(Arc::new(Fixed(vec![
            group("topical|standard|a:1%+b:2%+c:3%|g", 100), // identical: not a close match
            group("topical|standard|a:1%+b:2%|g", 500),      // 1 away, dearer
            group("topical|standard|a:2%+b:2%+c:3%|g", 300), // 1 away, cheaper
            group("topical|standard|a:9%+b:9%|g", 50),       // 3 away: too far
            group("topical|standard|a:1%+b:9%|g", 10),       // 2 away
        ])));
        let got: Vec<_> = svc
            .close(ours)
            .await
            .unwrap_or_default()
            .iter()
            .map(|m| m.group.key.as_str().to_owned())
            .collect();
        assert_eq!(
            got,
            [
                "topical|standard|a:2%+b:2%+c:3%|g",
                "topical|standard|a:1%+b:2%|g",
                "topical|standard|a:1%+b:9%|g",
            ]
        );
        assert!(matches!(
            svc.close("nope").await,
            Err(AppError::Invalid("key"))
        ));
    }
}
