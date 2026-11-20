//! Brands missing from every price list, mapped to their salts, so a search
//! for "Disprin" can show medicines with the same contents.
//! The list is `data/brand-aliases.csv`, reviewed by hand and compiled in.

use nirog_application::model::BrandAlias;
use nirog_application::ports::BrandAliases;
use nirog_domain::parse::{describe_salts, detect_form, detect_release, parse_brand_composition};
use nirog_domain::{MatchKey, PackUnit, Salt};
use serde::Deserialize;

/// The bundled, reviewed alias list.
#[derive(Debug, Clone, Default)]
pub struct BundledAliases(Vec<BrandAlias>);

impl BundledAliases {
    pub fn bundled() -> Self {
        Self::from_csv(include_str!("../../../data/brand-aliases.csv"))
    }

    /// Rows whose salts or strengths can't be read are left out.
    pub fn from_csv(text: &str) -> Self {
        let mut rdr = csv::Reader::from_reader(text.as_bytes());
        Self(
            rdr.deserialize::<Row>()
                .filter_map(Result::ok)
                .filter_map(Row::into_alias)
                .collect(),
        )
    }
}

#[derive(Debug, Deserialize)]
struct Row {
    brand: String,
    composition: String,
    form: String,
    unit: String,
}

impl Row {
    fn into_alias(self) -> Option<BrandAlias> {
        let salts = parse_brand_composition(&self.composition);
        let unit = PackUnit::parse(self.unit.trim()).ok()?;
        let key = MatchKey::new(
            &salts,
            detect_form(&self.form),
            detect_release(&self.form),
            unit,
        )?;
        let first = salts.first()?;
        Some(BrandAlias {
            brand: self.brand.trim().to_owned(),
            key,
            salts_label: describe_salts(&salts),
            search: describe_salts(&[Salt {
                name: first.name.clone(),
                strength: None,
            }]),
        })
    }
}

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Edit distance, for catching one-letter typos ("displin").
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let next = (row[j + 1] + 1)
                .min(row[j] + 1)
                .min(prev + usize::from(ca != *cb));
            prev = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

/// Shorter than this, a query is too vague to name a brand.
const MIN_LEN: usize = 4;

fn names_brand(query: &[String], brand: &[String]) -> bool {
    let (Some(q), Some(b)) = (query.first(), brand.first()) else {
        return false;
    };
    let allowed = if b.chars().count() <= 7 { 1 } else { 2 };
    q.chars().count() >= MIN_LEN && distance(q, b) <= allowed
}

impl BrandAliases for BundledAliases {
    fn find(&self, query: &str) -> Option<BrandAlias> {
        let q = words(query);
        self.0
            .iter()
            .find(|a| names_brand(&q, &words(&a.brand)))
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSV: &str = "brand,composition,form,unit,source
Disprin,Aspirin (350mg),Dispersible Tablet,unit,label
Gelusil MPS,Aluminium Hydroxide (250mg) + Magnesium Hydroxide (250mg) + Simethicone (50mg),Tablet,unit,label
Broken,Mystery (as directed),Tablet,unit,label
";

    #[test]
    fn finds_brands_despite_typos_case_and_extra_words() {
        let a = BundledAliases::from_csv(CSV);
        let disprin = a.find("disprin").unwrap();
        assert_eq!(disprin.brand, "Disprin");
        assert_eq!(disprin.salts_label, "Aspirin 350mg");
        assert_eq!(disprin.search, "Aspirin");
        assert_eq!(disprin.key.as_str(), "tablet|standard|aspirin:350mg|unit");
        for q in ["DISPRIN", "displin", "disprin 350", "Disprin tablet"] {
            assert_eq!(a.find(q).map(|x| x.brand), Some("Disprin".into()), "{q}");
        }
        assert_eq!(
            a.find("gelusil").map(|x| x.brand),
            Some("Gelusil MPS".into())
        );
        for q in ["dolo", "di", "aspirin", "broken"] {
            assert!(a.find(q).is_none(), "{q}");
        }
    }

    #[test]
    fn bundled_list_reads_every_row() {
        let rows = include_str!("../../../data/brand-aliases.csv")
            .lines()
            .count()
            - 1;
        assert_eq!(BundledAliases::bundled().0.len(), rows);
    }
}
