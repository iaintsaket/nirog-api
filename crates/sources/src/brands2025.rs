//! Cleaner for the 2025 India medicines brand dataset (Kaggle `apkaayush/india-medicines-and-drug-info-dataset`).
//!
//! Its `Product Name` column joins every field with no separator:
//! `Dolo 650 TabletMRP₹32.13strip of 15 tabletsMicro Labs LtdParacetamol (650mg)not available`.
//! Fields end where a lowercase letter runs straight into a capital ("tablets|Micro").

use std::collections::HashSet;
use std::io::Read;

use nirog_application::ports::SourceError;
use nirog_domain::parse::{
    describe_salts, detect_form, detect_release, parse_brand_composition, parse_brand_pack,
};
use nirog_domain::{MatchKey, Offer, OfferId, PackUnit, Paise, SourceKind};
use serde::Deserialize;

use nirog_application::ports::SourceInfo;
use time::macros::date;

use crate::clean::{CleanRow, Cleaned, Cleaner, Reason, Reject};

/// The 2025 India medicines brand dataset.
#[derive(Debug, Clone, Copy, Default)]
pub struct Brands2025;

impl Cleaner for Brands2025 {
    fn info(&self) -> SourceInfo {
        SourceInfo {
            id: "brands-2025".into(),
            kind: SourceKind::Brand,
            title: "India medicines and drug info (2025)".into(),
            url: Some(
                "https://www.kaggle.com/datasets/apkaayush/india-medicines-and-drug-info-dataset"
                    .into(),
            ),
            observed_on: date!(2025 - 10 - 22),
        }
    }
    fn clean(&self, raw: &mut dyn Read) -> Result<Cleaned, SourceError> {
        clean(raw)
    }
}

/// The fields packed into one `Product Name`.
#[derive(Debug, PartialEq, Eq)]
pub struct Parts<'a> {
    pub mrp: String,
    pub rx: bool,
    pub pack: &'a str,
    pub maker: &'a str,
    pub composition: &'a str,
}

pub fn split_product_name<'a>(name: &str, product: &'a str) -> Result<Parts<'a>, Reason> {
    let rest = product.strip_prefix(name).ok_or(Reason::Layout)?;
    let rest = rest.strip_prefix("MRP₹").ok_or(Reason::NoPrice)?;
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ','))
        .unwrap_or(rest.len());
    let mrp: String = rest[..end].chars().filter(|c| *c != ',').collect();
    if mrp.is_empty() {
        return Err(Reason::NoPrice);
    }
    let rest = &rest[end..];
    let (rx, tail) = match rest.strip_prefix("Prescription Required") {
        Some(t) => (true, t),
        None => (false, rest),
    };
    // Salts start at the first bracket holding a strength ("(500mg)"); makers
    // such as "Getwell Pharma (I) Pvt Ltd" have brackets too.
    let open = tail
        .match_indices('(')
        .map(|(i, _)| i)
        .find(|&i| tail[i + 1..].starts_with(|c: char| c.is_ascii_digit()))
        .ok_or(Reason::NoSalts)?;
    let close = tail
        .rfind(')')
        .filter(|&c| c > open)
        .ok_or(Reason::NoSalts)?;
    let joins = joins(&tail[..open]);
    let (&pack_end, &salts_start) = joins.first().zip(joins.last()).ok_or(Reason::Layout)?;
    Ok(Parts {
        mrp,
        rx,
        pack: tail[..pack_end].trim(),
        maker: tail[pack_end..salts_start].trim(),
        composition: &tail[salts_start..=close],
    })
}

/// Byte offsets where one field runs into the next with no space: a lowercase
/// letter, ")" or "." followed by a capital or digit ("tablets|Micro", "tablets|4Care"),
/// or the end of a capitalised run before a word ("Biozoc INC|Domperidone").
fn joins(text: &str) -> Vec<usize> {
    let cs: Vec<(usize, char)> = text.char_indices().collect();
    (1..cs.len())
        .filter(|&i| {
            let (prev, cur) = (cs[i - 1].1, cs[i].1);
            let next = cs.get(i + 1).map(|c| c.1);
            ((prev.is_lowercase() || prev == ')' || prev == '.')
                && (cur.is_uppercase() || cur.is_ascii_digit()))
                || (prev.is_uppercase()
                    && cur.is_uppercase()
                    && next.is_some_and(char::is_lowercase))
        })
        .map(|i| cs[i].0)
        .collect()
}

/// "CROcin Advance Tablet | For Fever" → "Crocin Advance Tablet".
fn clean_name(raw: &str) -> String {
    let base = raw.split(" | ").next().unwrap_or(raw);
    base.split_whitespace()
        .map(|w| {
            let mut cs = w.chars();
            let shouty =
                w.chars().take(2).all(char::is_uppercase) && w.chars().any(char::is_lowercase);
            match (shouty, cs.next()) {
                (true, Some(first)) => first.to_string() + &cs.as_str().to_lowercase(),
                _ => w.to_owned(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// FNV-1a: a stable short id from a product's identity, with no extra dependency.
fn stable_id(identity: &str) -> String {
    let hash = identity.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("b25:{hash:016x}")
}

/// ₹10,000 per tablet or capsule.
const MAX_UNIT_PRICE_MILLIPAISE: i64 = 1_000_000 * 1000;

fn clean_row(row: &Row) -> Result<CleanRow, Reason> {
    let parts = split_product_name(&row.name, &row.product)?;
    let name = clean_name(&row.name);
    if name.is_empty() {
        return Err(Reason::Layout);
    }
    // No visible boundary before the salts (a maker such as "savoy Biotech"):
    // reject rather than store a maker's name as a salt.
    if parts.maker.is_empty() {
        return Err(Reason::Layout);
    }
    let price = Paise::from_rupees_str(&parts.mrp).map_err(|_| Reason::NoPrice)?;
    let pack = parse_brand_pack(parts.pack)
        .filter(|p| p.count > 0)
        .ok_or(Reason::NoPack)?;
    let salts = parse_brand_composition(parts.composition);
    if salts.is_empty() {
        return Err(Reason::NoSalts);
    }
    let form_text = format!("{name} {}", parts.pack);
    let key = MatchKey::new(
        &salts,
        detect_form(&form_text),
        detect_release(&form_text),
        pack.unit,
    )
    .ok_or(Reason::UnreadableStrength)?;
    let mut offer = Offer {
        id: OfferId(String::new()),
        source: SourceKind::Brand,
        name,
        manufacturer: parts.maker.split_whitespace().collect::<Vec<_>>().join(" "),
        price,
        pack,
        key,
        salts_label: describe_salts(&salts),
    };
    let oral_solid = matches!(
        offer.key.as_str().split('|').next(),
        Some("tablet" | "capsule")
    );
    if oral_solid
        && pack.unit == PackUnit::Unit
        && offer.unit_price_millipaise() > MAX_UNIT_PRICE_MILLIPAISE
    {
        return Err(Reason::ImplausiblePrice);
    }
    offer.id = OfferId(stable_id(&offer.identity()));
    let rx_required = match row.rx.trim() {
        "True" => Some(true),
        "False" => Some(false),
        _ => Some(parts.rx),
    };
    Ok(CleanRow { offer, rx_required })
}

#[derive(Debug, Deserialize)]
struct Row {
    #[serde(rename = "Product Name")]
    product: String,
    #[serde(rename = "Medicine Name")]
    name: String,
    #[serde(rename = "Is Prescription Required?")]
    rx: String,
}

pub fn clean<R: Read>(reader: R) -> Result<Cleaned, SourceError> {
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(reader);
    let headers = rdr
        .headers()
        .map_err(|e| SourceError(e.to_string()))?
        .clone();
    let mut out = Cleaned::default();
    let mut seen = HashSet::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| SourceError(e.to_string()))?;
        let line = rec.position().map_or(0, csv::Position::line);
        let row: Row = match rec.deserialize(Some(&headers)) {
            Ok(r) => r,
            Err(_) => {
                out.rejects.push(Reject {
                    line,
                    name: rec.get(4).unwrap_or("").to_owned(),
                    reason: Reason::Layout,
                });
                continue;
            }
        };
        let result = clean_row(&row).and_then(|r| {
            if seen.insert(r.offer.identity()) {
                Ok(r)
            } else {
                Err(Reason::Duplicate)
            }
        });
        match result {
            Ok(r) => out.rows.push(r),
            Err(reason) => out.rejects.push(Reject {
                line,
                name: row.name,
                reason,
            }),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_joined_product_names() {
        let p = |name, product| split_product_name(name, product);
        assert_eq!(
            p(
                "Osizin-M 5mg/10mg Tablet",
                "Osizin-M 5mg/10mg TabletMRP₹91.88Prescription Requiredstrip of 10 tabletsOsiris HealthcareLevocetirizine (5mg) + Montelukast (10mg)not available"
            ),
            Ok(Parts {
                mrp: "91.88".into(),
                rx: true,
                pack: "strip of 10 tablets",
                maker: "Osiris Healthcare",
                composition: "Levocetirizine (5mg) + Montelukast (10mg)",
            })
        );
        // Over the counter, a capital inside the maker, a form after the pack, a comma in the price.
        assert_eq!(
            p(
                "Calpol 120mg Suspension",
                "Calpol 120mg SuspensionMRP₹1,037.80bottle of 60 ml Oral SuspensionGlaxo SmithKline Pharmaceuticals LtdParacetamol (120mg/5ml)ADD"
            ),
            Ok(Parts {
                mrp: "1037.80".into(),
                rx: false,
                pack: "bottle of 60 ml Oral Suspension",
                maker: "Glaxo SmithKline Pharmaceuticals Ltd",
                composition: "Paracetamol (120mg/5ml)",
            })
        );
        // A salt with two words.
        assert_eq!(
            p("S", "SMRP₹9strip of 6 tabletsAdenscot HealthcareCefpodoxime Proxetil (200mg) + Clavulanic Acid (125mg)ADD")
                .map(|x| (x.maker, x.composition)),
            Ok(("Adenscot Healthcare", "Cefpodoxime Proxetil (200mg) + Clavulanic Acid (125mg)"))
        );
        // A bracket inside the maker's name is not the start of the salts.
        assert_eq!(
            p("G", "GMRP₹1191.56vial of 1 InjectionGetwell Pharma (I) Pvt LtdMesna (100mg) + Ifosfamide (2gm)not available")
                .map(|x| (x.pack, x.maker, x.composition)),
            Ok(("vial of 1 Injection", "Getwell Pharma (I) Pvt Ltd", "Mesna (100mg) + Ifosfamide (2gm)"))
        );
        // Makers that start with a digit or end in capitals.
        assert_eq!(
            p(
                "N",
                "NMRP₹92.81strip of 10 tablets4Care Lifescience Pvt LtdNebivolol (5mg)ADD"
            )
            .map(|x| x.maker),
            Ok("4Care Lifescience Pvt Ltd")
        );
        assert_eq!(
            p(
                "Z",
                "ZMRP₹84.38strip of 10 capsule srBiozoc INCDomperidone (30mg)ADD"
            )
            .map(|x| (x.maker, x.composition)),
            Ok(("Biozoc INC", "Domperidone (30mg)"))
        );
        assert_eq!(p("Other", "Dolo 650 TabletMRP₹3"), Err(Reason::Layout));
        // No boundary before the salts: the maker is empty and the row is rejected.
        assert_eq!(
            p(
                "N",
                "NMRP₹22strip of 10 tabletssavoy BiotechNimesulide (100mg)ADD"
            )
            .map(|x| x.maker),
            Ok("")
        );
        assert_eq!(p("Dolo", "Dolo 650 Tablet"), Err(Reason::NoPrice));
        assert_eq!(
            p("D", "DMRP₹3strip of 10 tabletsSome Labsnot available"),
            Err(Reason::NoSalts)
        );
    }

    #[test]
    fn cleans_rows_and_explains_every_reject() {
        let csv = "\u{feff}Alphabet,Page,Product Name,Image URL,Medicine Name,Price,Is Prescription Required?,Type of Medicine,Composition
O,1,Osizin-M 5mg/10mg TabletMRP₹91.88Prescription Requiredstrip of 10 tabletsOsiris HealthcareLevocetirizine (5mg) + Montelukast (10mg)not available,u,Osizin-M 5mg/10mg Tablet,₹91,True,,
O,1,Osizin-M 5mg/10mg TabletMRP₹91.88Prescription Requiredstrip of 10 tabletsOsiris HealthcareLevocetirizine (5mg) + Montelukast (10mg)ADD,u,Osizin-M 5mg/10mg Tablet,₹91,True,,
C,1,CROcin Advance Tablet | For FeverMRP₹20strip of 15 tabletsHaleon IndiaParacetamol (500mg)not available,u,CROcin Advance Tablet | For Fever,₹20,False,,
N,1,No Price Tablet,u,No Price Tablet,,False,,
P,1,P TabletMRP₹10packet of tabletsSome LabsParacetamol (500mg)not available,u,P Tablet,₹10,False,,
S,1,S TabletMRP₹10strip of 10 tabletsSome Labsnot available,u,S Tablet,₹10,False,,
Z,1,Z TabletMRP₹999999strip of 1 tabletSome LabsParacetamol (500mg)ADD,u,Z Tablet,₹9,True,,
W,1,W TabletMRP₹10strip of 10 tabletsSome LabsFoo (5 drops)ADD,u,W Tablet,₹10,True,,
V,1,V TabletMRP₹22strip of 10 tabletssavoy BiotechNimesulide (100mg)ADD,u,V Tablet,₹22,True,,
D,1,Deninta 60mg InjectionMRP₹14053Prescription Requiredprefilled syringe of 1 InjectionIntas Pharmaceuticals LtdDenosumab (60mg)not available,u,Deninta 60mg Injection,₹14053,True,,
";
        let out = clean(csv.as_bytes()).unwrap();
        let names: Vec<_> = out.rows.iter().map(|r| r.offer.name.as_str()).collect();
        // A ₹14,053 injection is a real price; only tablets and capsules are capped.
        assert_eq!(
            names,
            [
                "Osizin-M 5mg/10mg Tablet",
                "Crocin Advance Tablet",
                "Deninta 60mg Injection"
            ]
        );
        let crocin = &out.rows[1];
        assert_eq!(crocin.rx_required, Some(false));
        assert_eq!(crocin.offer.price.0, 2000);
        assert_eq!(crocin.offer.manufacturer, "Haleon India");
        assert!(crocin.offer.id.0.starts_with("b25:"));
        let reasons: Vec<_> = out.rejects.iter().map(|r| (r.line, r.reason)).collect();
        assert_eq!(
            reasons,
            [
                (3, Reason::Duplicate),
                (5, Reason::NoPrice),
                (6, Reason::NoPack),
                (7, Reason::NoSalts),
                (8, Reason::ImplausiblePrice),
                (9, Reason::UnreadableStrength),
                (10, Reason::Layout),
            ]
        );
        // Ids are stable across runs.
        assert_eq!(
            clean(csv.as_bytes()).unwrap().rows[1].offer.id,
            crocin.offer.id
        );
    }
}
