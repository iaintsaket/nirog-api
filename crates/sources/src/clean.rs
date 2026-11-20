//! The Nirog CSV: one cleaned product per row, the same columns for every
//! source. Cleaners write it (plus a rejects file); [`NirogCsv`] loads it.

use std::io::{Read, Write};
use std::path::PathBuf;

use nirog_application::ports::{PriceSource, SourceError, SourceInfo};
use nirog_domain::{MatchKey, Offer, OfferId, Pack, PackUnit, Paise, SourceKind};
use serde::Deserialize;

/// Why a raw row was left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reason {
    /// The row's fields could not be told apart.
    Layout,
    NoPrice,
    NoPack,
    NoSalts,
    /// A salt without a strength we can read; we never guess a match.
    UnreadableStrength,
    /// Over ₹10,000 per tablet or capsule: almost always a data error. Kept for review.
    ImplausiblePrice,
    /// The same product (name, maker, pack) appeared earlier in the file.
    Duplicate,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Layout => "layout",
            Self::NoPrice => "no_price",
            Self::NoPack => "no_pack",
            Self::NoSalts => "no_salts",
            Self::UnreadableStrength => "unreadable_strength",
            Self::ImplausiblePrice => "implausible_price",
            Self::Duplicate => "duplicate",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanRow {
    pub offer: Offer,
    pub rx_required: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reject {
    /// Line in the raw file (the header is line 1).
    pub line: u64,
    pub name: String,
    pub reason: Reason,
}

#[derive(Debug, Default)]
pub struct Cleaned {
    pub rows: Vec<CleanRow>,
    pub rejects: Vec<Reject>,
}

/// Turns one raw dataset into Nirog CSV rows. To add a dataset, implement this
/// in a new module and list it in [`cleaners`]; nothing else changes.
pub trait Cleaner: Send + Sync {
    fn info(&self) -> SourceInfo;
    fn clean(&self, raw: &mut dyn Read) -> Result<Cleaned, SourceError>;
}

/// Every dataset the importer can clean. The only list to extend.
pub fn cleaners() -> Vec<Box<dyn Cleaner>> {
    vec![Box::new(crate::onemg::OneMg2025)]
}

pub fn cleaner_for(id: &str) -> Option<Box<dyn Cleaner>> {
    cleaners().into_iter().find(|c| c.info().id == id)
}

/// The label (title, link, date) of a dataset the importer knows.
pub fn known_source(id: &str) -> Option<SourceInfo> {
    cleaner_for(id).map(|c| c.info())
}

const COLUMNS: [&str; 11] = [
    "source",
    "source_id",
    "name",
    "manufacturer",
    "mrp_paise",
    "pack_count",
    "pack_unit",
    "match_key",
    "salts",
    "rx_required",
    "observed_on",
];

fn csv_err(e: impl std::fmt::Display) -> SourceError {
    SourceError(e.to_string())
}

pub fn write_clean<W: Write>(
    rows: &[CleanRow],
    source: &SourceInfo,
    w: W,
) -> Result<(), SourceError> {
    let mut out = csv::Writer::from_writer(w);
    out.write_record(COLUMNS).map_err(csv_err)?;
    let observed = source.observed_on.to_string();
    for CleanRow {
        offer: o,
        rx_required,
    } in rows
    {
        let rx = rx_required.map_or("", |r| if r { "true" } else { "false" });
        out.write_record([
            source.id.as_str(),
            &o.id.0,
            &o.name,
            &o.manufacturer,
            &o.price.0.to_string(),
            &o.pack.count.to_string(),
            o.pack.unit.as_str(),
            o.key.as_str(),
            &o.salts_label,
            rx,
            &observed,
        ])
        .map_err(csv_err)?;
    }
    out.flush().map_err(csv_err)
}

pub fn write_rejects<W: Write>(rejects: &[Reject], w: W) -> Result<(), SourceError> {
    let mut out = csv::Writer::from_writer(w);
    out.write_record(["line", "name", "reason"])
        .map_err(csv_err)?;
    for r in rejects {
        out.write_record([r.line.to_string().as_str(), &r.name, r.reason.as_str()])
            .map_err(csv_err)?;
    }
    out.flush().map_err(csv_err)
}

#[derive(Debug, Deserialize)]
struct Row {
    source_id: String,
    name: String,
    manufacturer: String,
    mrp_paise: i64,
    pack_count: u32,
    pack_unit: String,
    match_key: String,
    salts: String,
}

/// Loads a Nirog CSV written by [`write_clean`].
#[derive(Debug, Clone)]
pub struct NirogCsv {
    path: PathBuf,
    info: SourceInfo,
}

impl NirogCsv {
    pub fn new(path: impl Into<PathBuf>, info: SourceInfo) -> Self {
        Self {
            path: path.into(),
            info,
        }
    }

    pub fn parse<R: Read>(reader: R, kind: SourceKind) -> Result<(Vec<Offer>, usize), SourceError> {
        crate::read_all(reader, |r: Row| {
            Some(Offer {
                id: OfferId(r.source_id),
                source: kind,
                name: r.name,
                manufacturer: r.manufacturer,
                price: Some(Paise(r.mrp_paise)).filter(|p| p.0 > 0)?,
                pack: Pack {
                    count: Some(r.pack_count).filter(|&c| c > 0)?,
                    unit: PackUnit::parse(&r.pack_unit).ok()?,
                },
                key: MatchKey::parse(&r.match_key)?,
                salts_label: r.salts,
            })
        })
    }
}

impl PriceSource for NirogCsv {
    fn info(&self) -> SourceInfo {
        self.info.clone()
    }
    fn load(&self) -> Result<(Vec<Offer>, usize), SourceError> {
        Self::parse(crate::open(&self.path)?, self.info.kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::date;

    const RAW: &str = "Alphabet,Page,Product Name,Image URL,Medicine Name,Price,Is Prescription Required?,Type of Medicine,Composition
O,1,Osizin-M 5mg/10mg TabletMRP₹91.88Prescription Requiredstrip of 10 tabletsOsiris HealthcareLevocetirizine (5mg) + Montelukast (10mg)not available,u,Osizin-M 5mg/10mg Tablet,₹91,True,,
C,1,Calpol 120mg SuspensionMRP₹37.8bottle of 60 ml Oral SuspensionGlaxo SmithKline Pharmaceuticals LtdParacetamol (120mg/5ml)ADD,u,Calpol 120mg Suspension,₹37,False,,
";

    #[test]
    fn nirog_csv_round_trips_and_explains_rejects() {
        let src = known_source("onemg-2025").unwrap();
        assert_eq!(src.kind, SourceKind::Brand);
        assert_eq!(src.observed_on, date!(2025 - 10 - 22));
        assert!(known_source("nope").is_none());
        let ids: Vec<_> = cleaners().iter().map(|c| c.info().id).collect();
        assert_eq!(ids, ["onemg-2025"]);

        let cleaned = cleaner_for("onemg-2025")
            .unwrap()
            .clean(&mut RAW.as_bytes())
            .unwrap();
        let mut out = Vec::new();
        write_clean(&cleaned.rows, &src, &mut out).unwrap();
        let text = String::from_utf8(out.clone()).unwrap();
        assert_eq!(
            text.lines().next(),
            Some(
                "source,source_id,name,manufacturer,mrp_paise,pack_count,pack_unit,match_key,salts,rx_required,observed_on"
            )
        );
        assert!(
            text.contains(
                ",Calpol 120mg Suspension,Glaxo SmithKline Pharmaceuticals Ltd,3780,60,ml,"
            )
        );
        let (offers, skipped) = NirogCsv::parse(out.as_slice(), SourceKind::Brand).unwrap();
        assert_eq!(skipped, 0);
        assert_eq!(
            offers,
            cleaned
                .rows
                .iter()
                .map(|r| r.offer.clone())
                .collect::<Vec<_>>()
        );

        let mut rej = Vec::new();
        let reject = Reject {
            line: 7,
            name: "Z Tablet".into(),
            reason: Reason::ImplausiblePrice,
        };
        write_rejects(&[reject], &mut rej).unwrap();
        assert_eq!(
            String::from_utf8(rej).unwrap(),
            "line,name,reason\n7,Z Tablet,implausible_price\n"
        );
    }
}
