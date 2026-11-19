use serde::{Deserialize, Serialize};
use std::fmt;

use crate::error::DomainError;
use crate::matching::MatchKey;

/// Money in paise. Integers only: no floating-point rupees anywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Paise(pub i64);

impl Paise {
    /// Parse a rupee amount such as "9.08" or "223.42". Rejects zero, negatives and junk.
    pub fn from_rupees_str(s: &str) -> Result<Self, DomainError> {
        let s = s.trim();
        let (whole, frac) = s.split_once('.').unwrap_or((s, ""));
        if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) || !frac.bytes().all(|b| b.is_ascii_digit()) {
            return Err(DomainError::Invalid("price"));
        }
        let mut frac2: String = frac.chars().take(2).collect();
        while frac2.len() < 2 {
            frac2.push('0');
        }
        let rupees: i64 = whole.parse().map_err(|_| DomainError::Invalid("price"))?;
        let paise: i64 = frac2.parse().map_err(|_| DomainError::Invalid("price"))?;
        let total = rupees.checked_mul(100).and_then(|r| r.checked_add(paise)).ok_or(DomainError::Invalid("price"))?;
        if total <= 0 {
            return Err(DomainError::Invalid("price"));
        }
        Ok(Self(total))
    }
}

/// Where an offer comes from. New sources (AMRIT, NPPA, partner feeds) are new variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    JanAushadhi,
    Brand,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::JanAushadhi => "jan_aushadhi",
            Self::Brand => "brand",
        }
    }

    pub fn parse(s: &str) -> Result<Self, DomainError> {
        match s {
            "jan_aushadhi" => Ok(Self::JanAushadhi),
            "brand" => Ok(Self::Brand),
            _ => Err(DomainError::Invalid("source")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Form {
    Tablet,
    Capsule,
    Liquid,
    Injection,
    Topical,
    Inhaler,
    EyeEar,
    Other,
}

impl Form {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tablet => "tablet",
            Self::Capsule => "capsule",
            Self::Liquid => "liquid",
            Self::Injection => "injection",
            Self::Topical => "topical",
            Self::Inhaler => "inhaler",
            Self::EyeEar => "eye-ear",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Release {
    Standard,
    Extended,
}

impl Release {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Extended => "extended",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackUnit {
    Unit,
    Ml,
    G,
}

impl PackUnit {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unit => "unit",
            Self::Ml => "ml",
            Self::G => "g",
        }
    }

    pub fn parse(s: &str) -> Result<Self, DomainError> {
        match s {
            "unit" => Ok(Self::Unit),
            "ml" => Ok(Self::Ml),
            "g" => Ok(Self::G),
            _ => Err(DomainError::Invalid("pack unit")),
        }
    }
}

/// Strength of one ingredient. Mass is held in micrograms so it is exact.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Strength {
    Micrograms(u64),
    /// Per-volume, percentage or IU strengths, normalised text such as "30mg/5ml".
    Text(String),
}

impl fmt::Display for Strength {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Micrograms(ug) => {
                let whole = ug / 1000;
                let frac = ug % 1000;
                if frac == 0 {
                    write!(f, "{whole}mg")
                } else {
                    let frac = format!("{frac:03}");
                    write!(f, "{whole}.{}mg", frac.trim_end_matches('0'))
                }
            }
            Self::Text(t) => f.write_str(t),
        }
    }
}

/// One active ingredient. `strength` is `None` when the source did not state it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Salt {
    pub name: String,
    pub strength: Option<Strength>,
}

impl Salt {
    pub fn mg(name: &str, ug: u64) -> Self {
        Self { name: name.to_owned(), strength: Some(Strength::Micrograms(ug)) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pack {
    pub count: u32,
    pub unit: PackUnit,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OfferId(pub String);

/// One buyable product from one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub id: OfferId,
    pub source: SourceKind,
    pub name: String,
    pub manufacturer: String,
    pub price: Paise,
    pub pack: Pack,
    pub key: MatchKey,
    /// Human label such as "Amoxicillin 500mg + Clavulanic Acid 125mg".
    pub salts_label: String,
}

impl Offer {
    /// Price per unit in thousandths of a paisa, so sorting stays in integers.
    pub fn unit_price_millipaise(&self) -> i64 {
        self.price.0 * 1000 / i64::from(self.pack.count.max(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rupees_into_paise() {
        assert_eq!(Paise::from_rupees_str("9.08"), Ok(Paise(908)));
        assert_eq!(Paise::from_rupees_str("223.4"), Ok(Paise(22340)));
        assert_eq!(Paise::from_rupees_str(" 15 "), Ok(Paise(1500)));
        assert_eq!(Paise::from_rupees_str("1.999"), Ok(Paise(199)));
    }

    #[test]
    fn rejects_bad_prices() {
        for bad in ["", "abc", "0", "0.00", "-5", "1.2.3", ".5", "99999999999999999999"] {
            assert!(Paise::from_rupees_str(bad).is_err(), "{bad} should fail");
        }
    }

    #[test]
    fn enum_strings_round_trip() {
        for s in [SourceKind::JanAushadhi, SourceKind::Brand] {
            assert_eq!(SourceKind::parse(s.as_str()), Ok(s));
        }
        for u in [PackUnit::Unit, PackUnit::Ml, PackUnit::G] {
            assert_eq!(PackUnit::parse(u.as_str()), Ok(u));
        }
        assert!(SourceKind::parse("x").is_err());
        assert!(PackUnit::parse("x").is_err());
        let forms = [Form::Tablet, Form::Capsule, Form::Liquid, Form::Injection, Form::Topical, Form::Inhaler, Form::EyeEar, Form::Other];
        let names: Vec<_> = forms.iter().map(|f| f.as_str()).collect();
        assert_eq!(names, ["tablet", "capsule", "liquid", "injection", "topical", "inhaler", "eye-ear", "other"]);
        assert_eq!(Release::Extended.as_str(), "extended");
    }

    #[test]
    fn strength_displays_in_mg() {
        assert_eq!(Strength::Micrograms(500_000).to_string(), "500mg");
        assert_eq!(Strength::Micrograms(500).to_string(), "0.5mg");
        assert_eq!(Strength::Micrograms(1_250).to_string(), "1.25mg");
        assert_eq!(Strength::Text("30mg/5ml".into()).to_string(), "30mg/5ml");
    }
}
