//! Price sources. Each one turns a raw feed into domain offers and counts the
//! rows it had to skip (no price, no pack size, or a strength it cannot read).

use std::io::Read;
use std::path::PathBuf;

use nirog_application::ports::{PriceSource, SourceError, SourceInfo};
use nirog_domain::parse::{
    describe_salts, detect_form, detect_release, parse_brand_composition, parse_brand_pack,
    parse_jan_aushadhi_name, parse_jan_aushadhi_pack,
};
use nirog_domain::{MatchKey, Offer, OfferId, Pack, Paise, Salt, SourceKind};
use serde::Deserialize;
use time::macros::date;

/// One source row after field extraction, before validation.
struct Raw<'a> {
    id: String,
    source: SourceKind,
    name: &'a str,
    manufacturer: &'a str,
    price: &'a str,
    pack: Option<Pack>,
    salts: Vec<Salt>,
    /// Text used to detect dosage form and release type.
    form_text: &'a str,
}

fn build(raw: Raw<'_>) -> Option<Offer> {
    let Raw {
        id,
        source,
        name,
        manufacturer,
        price,
        pack,
        salts,
        form_text,
    } = raw;
    let salts = &salts[..];
    let price = Paise::from_rupees_str(price.trim()).ok()?;
    let pack = pack.filter(|p| p.count > 0)?;
    let key = MatchKey::new(
        salts,
        detect_form(form_text),
        detect_release(form_text),
        pack.unit,
    )?;
    Some(Offer {
        id: OfferId(id),
        source,
        name: name.split_whitespace().collect::<Vec<_>>().join(" "),
        manufacturer: manufacturer
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
        price,
        pack,
        key,
        salts_label: describe_salts(salts),
    })
}

/// Reads every row of a CSV; returns offers and how many rows were skipped.
fn read_all<R: Read, T: for<'de> Deserialize<'de>>(
    reader: R,
    to_offer: impl Fn(T) -> Option<Offer>,
) -> Result<(Vec<Offer>, usize), SourceError> {
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(reader);
    let (mut offers, mut skipped) = (Vec::new(), 0);
    for row in rdr.deserialize::<T>() {
        match row {
            Ok(r) => match to_offer(r) {
                Some(o) => offers.push(o),
                None => skipped += 1,
            },
            Err(e) if e.is_io_error() => return Err(SourceError(e.to_string())),
            Err(_) => skipped += 1,
        }
    }
    Ok((offers, skipped))
}

fn open(path: &PathBuf) -> Result<std::fs::File, SourceError> {
    std::fs::File::open(path).map_err(|e| SourceError(format!("{}: {e}", path.display())))
}

#[derive(Debug, Deserialize)]
struct JanAushadhiRow {
    drug_code: String,
    generic_name: String,
    unit_size: String,
    mrp: String,
}

/// The PMBJP product list (`data/janaushadhi-products.csv`).
#[derive(Debug, Clone)]
pub struct JanAushadhiCsv {
    path: PathBuf,
}

impl JanAushadhiCsv {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn parse<R: Read>(reader: R) -> Result<(Vec<Offer>, usize), SourceError> {
        read_all(reader, |r: JanAushadhiRow| {
            build(Raw {
                id: format!("ja:{}", r.drug_code.trim()),
                source: SourceKind::JanAushadhi,
                name: &r.generic_name,
                manufacturer: "PMBI (Jan Aushadhi)",
                price: &r.mrp,
                pack: parse_jan_aushadhi_pack(&r.unit_size),
                salts: parse_jan_aushadhi_name(&r.generic_name),
                form_text: &r.generic_name,
            })
        })
    }
}

impl PriceSource for JanAushadhiCsv {
    fn info(&self) -> SourceInfo {
        SourceInfo {
            id: "jan-aushadhi".into(),
            kind: SourceKind::JanAushadhi,
            title: "Jan Aushadhi product list".into(),
            url: Some("https://janaushadhi.gov.in/".into()),
            // When the bundled list was taken; update it with the file.
            observed_on: date!(2026 - 11 - 20),
        }
    }
    fn load(&self) -> Result<(Vec<Offer>, usize), SourceError> {
        Self::parse(open(&self.path)?)
    }
}

#[derive(Debug, Deserialize)]
struct BrandRow {
    id: String,
    name: String,
    #[serde(rename = "price(₹)")]
    price: String,
    #[serde(rename = "Is_discontinued")]
    discontinued: String,
    manufacturer_name: String,
    #[serde(rename = "type")]
    kind: String,
    pack_size_label: String,
    short_composition1: String,
    #[serde(default)]
    short_composition2: String,
}

/// The Indian brand-medicine dataset (CSV with `short_composition1/2`).
/// Discontinued and non-allopathic rows are skipped.
#[derive(Debug, Clone)]
pub struct BrandDatasetCsv {
    path: PathBuf,
}

impl BrandDatasetCsv {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn parse<R: Read>(reader: R) -> Result<(Vec<Offer>, usize), SourceError> {
        read_all(reader, |r: BrandRow| {
            if r.discontinued.trim().eq_ignore_ascii_case("true")
                || !r.kind.trim().eq_ignore_ascii_case("allopathy")
            {
                return None;
            }
            let composition = [r.short_composition1.trim(), r.short_composition2.trim()]
                .into_iter()
                .filter(|c| !c.is_empty())
                .collect::<Vec<_>>()
                .join(" + ");
            let form_text = format!("{} {}", r.name, r.pack_size_label);
            build(Raw {
                id: format!("brand:{}", r.id.trim()),
                source: SourceKind::Brand,
                name: &r.name,
                manufacturer: &r.manufacturer_name,
                price: &r.price,
                pack: parse_brand_pack(&r.pack_size_label),
                salts: parse_brand_composition(&composition),
                form_text: &form_text,
            })
        })
    }
}

impl PriceSource for BrandDatasetCsv {
    fn info(&self) -> SourceInfo {
        SourceInfo {
            id: "brands-2022".into(),
            kind: SourceKind::Brand,
            title: "Indian Medicine Dataset (Nov 2022)".into(),
            url: Some("https://github.com/junioralive/Indian-Medicine-Dataset".into()),
            observed_on: date!(2022 - 11 - 17),
        }
    }
    fn load(&self) -> Result<(Vec<Offer>, usize), SourceError> {
        Self::parse(open(&self.path)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JA: &str = "drug_code,generic_name,unit_size,mrp,group
1,Aceclofenac 100mg and Paracetamol 325mg Tablets,10's,10.32,Analgesic
2,Paracetamol Tablets IP 650 mg,15's,15.00,Analgesic
3,Some Kit,1 kit,50.00,Other
4,Paracetamol Tablets IP 650 mg,15's,abc,Analgesic
";

    const BRANDS: &str = "id,name,price(₹),Is_discontinued,manufacturer_name,type,pack_size_label,short_composition1,short_composition2
1,Augmentin 625 Duo Tablet,223.42,FALSE,Glaxo SmithKline Pharmaceuticals Ltd,allopathy,strip of 10 tablets,Amoxycillin  (500mg) ,  Clavulanic Acid (125mg)
2,Dolo 650 Tablet,30.91,FALSE,Micro Labs Ltd,allopathy,strip of 15 tablets,Paracetamol (650mg),
3,Old Tablet,10,TRUE,X,allopathy,strip of 10 tablets,Paracetamol (650mg),
4,Herbal Syrup,10,FALSE,X,ayurveda,bottle of 100 ml Syrup,Tulsi (10mg),
5,Mystery Tablet,10,FALSE,X,allopathy,strip of 10 tablets,Paracetamol,
";

    #[test]
    fn jan_aushadhi_rows_become_offers_and_bad_rows_are_counted() -> Result<(), SourceError> {
        let (offers, skipped) = JanAushadhiCsv::parse(JA.as_bytes())?;
        assert_eq!(skipped, 2);
        assert_eq!(offers.len(), 2);
        let para = &offers[1];
        assert_eq!(para.id, OfferId("ja:2".into()));
        assert_eq!(para.price, Paise(1500));
        assert_eq!(para.pack.count, 15);
        assert_eq!(para.salts_label, "Paracetamol 650mg");
        Ok(())
    }

    #[test]
    fn brand_rows_skip_discontinued_non_allopathic_and_unknown_strength() -> Result<(), SourceError>
    {
        let (offers, skipped) = BrandDatasetCsv::parse(BRANDS.as_bytes())?;
        assert_eq!(skipped, 3);
        assert_eq!(
            offers.iter().map(|o| o.id.0.as_str()).collect::<Vec<_>>(),
            ["brand:1", "brand:2"]
        );
        assert_eq!(
            offers[0].manufacturer,
            "Glaxo SmithKline Pharmaceuticals Ltd"
        );
        Ok(())
    }

    #[test]
    fn jan_aushadhi_and_brand_paracetamol_share_a_match_key() -> Result<(), SourceError> {
        let (ja, _) = JanAushadhiCsv::parse(JA.as_bytes())?;
        let (brands, _) = BrandDatasetCsv::parse(BRANDS.as_bytes())?;
        assert_eq!(ja[1].key, brands[1].key);
        assert!(ja[1].unit_price_millipaise() < brands[1].unit_price_millipaise());
        Ok(())
    }

    #[test]
    fn missing_file_is_an_error() {
        assert!(JanAushadhiCsv::new("/no/such/file.csv").load().is_err());
        assert_eq!(BrandDatasetCsv::new("x").info().kind, SourceKind::Brand);
        assert_eq!(
            JanAushadhiCsv::new("x").info().kind,
            SourceKind::JanAushadhi
        );
    }
}
