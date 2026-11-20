use crate::error::DomainError;
use crate::model::{Offer, OfferId, Paise};

/// Every offer for one match key, cheapest per unit first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    pub offers: Vec<Offer>,
    /// Index into `offers` of the medicine the user searched for, if any.
    pub searched: Option<usize>,
    /// What the user saves per pack of the searched (or costliest) offer by
    /// buying the cheapest at its unit price.
    pub saving_per_pack: Paise,
    /// Saving in basis points (1/100 of a percent), integer to stay exact.
    pub saving_bps: i64,
}

impl Comparison {
    pub fn build(mut offers: Vec<Offer>, searched: Option<&OfferId>) -> Result<Self, DomainError> {
        let first_key = offers.first().ok_or(DomainError::NoOffers)?.key.clone();
        if offers.iter().any(|o| o.key != first_key) {
            return Err(DomainError::MixedKeys);
        }
        offers.sort_by(|a, b| {
            a.unit_price_millipaise()
                .cmp(&b.unit_price_millipaise())
                .then_with(|| a.id.cmp(&b.id))
        });
        let searched_idx = searched.and_then(|id| offers.iter().position(|o| &o.id == id));
        let cheapest = &offers[0];
        let reference = &offers[searched_idx.unwrap_or(offers.len() - 1)];
        // Exact cost of the reference pack's quantity at the cheapest offer's price,
        // rounded to the nearest paisa (no rounded unit prices in between).
        let (num, den) = (
            i128::from(cheapest.price.0) * i128::from(reference.pack.count),
            i128::from(cheapest.pack.count.max(1)),
        );
        let same_quantity_at_cheapest = (2 * num + den) / (2 * den);
        let saving = (i128::from(reference.price.0) - same_quantity_at_cheapest).max(0);
        let saving_bps = if reference.price.0 > 0 {
            saving * 10_000 / i128::from(reference.price.0)
        } else {
            0
        };
        let to_i64 = |v: i128| i64::try_from(v).unwrap_or(i64::MAX);
        Ok(Self {
            offers,
            searched: searched_idx,
            saving_per_pack: Paise(to_i64(saving)),
            saving_bps: to_i64(saving_bps),
        })
    }

    pub fn cheapest(&self) -> &Offer {
        &self.offers[0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matching::MatchKey;
    use crate::model::{Form, Pack, PackUnit, Release, Salt, SourceKind};

    fn offer(id: &str, price: i64, count: u32, ug: u64) -> Offer {
        Offer {
            id: OfferId(id.into()),
            source: SourceKind::Brand,
            name: id.into(),
            manufacturer: String::new(),
            price: Paise(price),
            pack: Pack {
                count,
                unit: PackUnit::Unit,
            },
            key: MatchKey::new(
                &[Salt::mg("atorvastatin", ug)],
                Form::Tablet,
                Release::Standard,
                PackUnit::Unit,
            )
            .unwrap(),
            salts_label: String::new(),
        }
    }

    #[test]
    fn sorts_cheapest_first_and_reports_saving_against_searched() {
        let c = Comparison::build(
            vec![
                offer("lipitor", 20_000, 10, 10_000),
                offer("atorva", 10_000, 10, 10_000),
                offer("ja", 908, 10, 10_000),
            ],
            Some(&OfferId("lipitor".into())),
        )
        .unwrap();
        let ids: Vec<_> = c.offers.iter().map(|o| o.id.0.as_str()).collect();
        assert_eq!(ids, ["ja", "atorva", "lipitor"]);
        assert_eq!(c.cheapest().id.0, "ja");
        assert_eq!(c.searched, Some(2));
        assert_eq!(c.saving_per_pack, Paise(19_092));
        assert_eq!(c.saving_bps, 9_546);
    }

    #[test]
    fn compares_against_costliest_when_nothing_searched_or_unknown_id() {
        let offers = vec![offer("a", 1000, 10, 1), offer("b", 2000, 10, 1)];
        let c = Comparison::build(offers.clone(), None).unwrap();
        assert_eq!(c.searched, None);
        assert_eq!(c.saving_bps, 5_000);
        let c = Comparison::build(offers, Some(&OfferId("zzz".into()))).unwrap();
        assert_eq!(c.searched, None);
    }

    #[test]
    fn zero_saving_when_searched_is_cheapest() {
        let c =
            Comparison::build(vec![offer("a", 1000, 10, 1)], Some(&OfferId("a".into()))).unwrap();
        assert_eq!((c.saving_per_pack, c.saving_bps), (Paise(0), 0));
    }

    #[test]
    fn saving_uses_exact_pack_prices_not_rounded_unit_prices() {
        // 30.91 for 15 vs 15.00 for 15: saving is exactly 15.91.
        let c = Comparison::build(
            vec![offer("a", 3091, 15, 650_000), offer("b", 1500, 15, 650_000)],
            Some(&OfferId("a".into())),
        )
        .unwrap();
        assert_eq!(c.saving_per_pack, Paise(1591));
        assert_eq!(c.saving_bps, 5147);
    }

    #[test]
    fn rejects_empty_and_mixed_keys() {
        assert_eq!(Comparison::build(vec![], None), Err(DomainError::NoOffers));
        let mixed = vec![offer("a", 1000, 10, 1), offer("b", 1000, 10, 2)];
        assert_eq!(Comparison::build(mixed, None), Err(DomainError::MixedKeys));
    }

    #[test]
    fn unit_price_is_integer_millipaise() {
        assert_eq!(offer("x", 908, 10, 1).unit_price_millipaise(), 90_800);
    }
}
