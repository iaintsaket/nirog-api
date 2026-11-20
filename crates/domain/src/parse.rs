//! Parsers for the text formats our sources use. All rules live here, in tested code.

use regex::Regex;
use std::sync::LazyLock;

use crate::model::{Form, Pack, PackUnit, Release, Salt, Strength};

#[allow(clippy::expect_used)] // static patterns, covered by tests
fn re(p: &str) -> Regex {
    Regex::new(p).expect("valid regex")
}

static RELEASE_WORDS: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)\([^)]*release[^)]*\)|\b(prolonged|sustained|extended|controlled|modified|delayed)[-\s]?release\b|\bgastro[-\s]?resistant\b")
});
static STRENGTH: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)(\d+(?:\.\d+)?)\s*(mcg|µg|mg|gm|g|iu|%)(?:\s*w/[wv])?(?:\s*(?:/|per)\s*(\d+(?:\.\d+)?)?\s*(ml|gm|g)\b)?")
});
static FORM_WORD: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)\b(tablets?|capsules?|syrup|suspension|injection|gel|cream|ointment|drops?|oral solution|solution|lotion|inhaler|respules|powder|sachets?)\b")
});
static SPLIT: LazyLock<Regex> = LazyLock::new(|| re(r"(?i),|\band\b|\+"));
static BRAND_PIECE: LazyLock<Regex> = LazyLock::new(|| re(r"^(.*?)\s*\((.*)\)\s*$"));
static EXTENDED: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)\b(prolonged|sustained|extended|controlled)\b|\b(sr|er|xr|pr|cr|xl)\b")
});
static JA_VOLUME: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(\d+(?:\.\d+)?)\s*(ml|gm|g)\b"));
static JA_COUNT: LazyLock<Regex> = LazyLock::new(|| re(r"(\d+)\s*['’]s\b"));
static BRAND_PACK: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(\d+(?:\.\d+)?)\s*([a-z]+)"));

/// Salt-form words dropped when they follow the molecule ("Montelukast Sodium").
const SUFFIXES: &[&str] = &[
    "hydrochloride", "dihydrochloride", "hcl", "sodium", "potassium", "calcium",
    "besylate", "besilate", "magnesium", "ip", "bp", "usp",
];

fn synonym(name: &str) -> Option<&'static str> {
    match name {
        "amoxycillin" => Some("amoxicillin"),
        "acetaminophen" => Some("paracetamol"),
        "potassium clavulanate" | "clavulanate potassium" | "diluted potassium clavulanate" => Some("clavulanic acid"),
        _ => None,
    }
}

pub fn normalize_salt(raw: &str) -> String {
    let cleaned: String = RELEASE_WORDS
        .replace_all(&raw.to_lowercase(), " ")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { ' ' })
        .collect();
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    let kept: Vec<&str> = words
        .iter()
        .enumerate()
        .filter(|(i, w)| *i == 0 || !SUFFIXES.contains(w))
        .map(|(_, w)| *w)
        .collect();
    let name = kept.join(" ");
    synonym(&name).or_else(|| synonym(&words.join(" "))).map_or(name, str::to_owned)
}

/// Converts a mass to micrograms. `None` for units that are not a mass.
pub fn to_micrograms(value: f64, unit: &str) -> Option<u64> {
    let factor = match unit.to_lowercase().as_str() {
        "mg" => 1_000.0,
        "g" | "gm" => 1_000_000.0,
        "mcg" | "µg" => 1.0,
        _ => return None,
    };
    let ug = (value * factor).round();
    // Bounded, non-negative and rounded above, so the cast is exact.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    (ug.is_finite() && (0.0..1e15).contains(&ug)).then_some(ug as u64)
}

fn num(s: &str) -> String {
    s.parse::<f64>().map_or_else(|_| s.to_owned(), |v| v.to_string())
}

fn strength_from(caps: &regex::Captures<'_>) -> Option<Strength> {
    let value = caps.get(1)?.as_str();
    let unit = caps.get(2)?.as_str().to_lowercase();
    if let Some(per_unit) = caps.get(4) {
        let per = caps.get(3).map_or("1", |m| m.as_str());
        return Some(Strength::Text(format!("{}{unit}/{}{}", num(value), num(per), per_unit.as_str().to_lowercase())));
    }
    match to_micrograms(value.parse().ok()?, &unit) {
        Some(ug) => Some(Strength::Micrograms(ug)),
        None => Some(Strength::Text(format!("{}{unit}", num(value)))),
    }
}

/// Brand data: "Amoxycillin (500mg) + Clavulanic Acid (125mg)".
pub fn parse_brand_composition(composition: &str) -> Vec<Salt> {
    composition
        .split('+')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|piece| match BRAND_PIECE.captures(piece) {
            Some(c) => Salt {
                name: normalize_salt(c.get(1).map_or("", |m| m.as_str())),
                strength: c.get(2).and_then(|m| STRENGTH.captures(m.as_str())).and_then(|s| strength_from(&s)),
            },
            None => Salt { name: normalize_salt(piece), strength: None },
        })
        .collect()
}

/// Jan Aushadhi names: "Metformin 500mg and Glimepiride 2mg Tablets IP" or "Atorvastatin Tablets IP 10mg".
pub fn parse_jan_aushadhi_name(name: &str) -> Vec<Salt> {
    let at = FORM_WORD.find(name).map_or(name.len(), |m| m.start());
    let (before, after) = name.split_at(at);
    let before = RELEASE_WORDS.replace_all(before, " ");

    let mut salts: Vec<Salt> = SPLIT
        .split(&before)
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|piece| match STRENGTH.captures(piece) {
            Some(c) => {
                let whole = c.get(0).map_or(0..0, |m| m.range());
                let salt_text = format!("{}{}", &piece[..whole.start], &piece[whole.end..]);
                Salt { name: normalize_salt(&salt_text), strength: strength_from(&c) }
            }
            None => Salt { name: normalize_salt(piece), strength: None },
        })
        .collect();

    if salts.iter().all(|s| s.strength.is_none()) {
        let tail: Vec<Strength> = STRENGTH.captures_iter(after).filter_map(|c| strength_from(&c)).collect();
        if tail.len() == salts.len() {
            for (salt, st) in salts.iter_mut().zip(tail) {
                salt.strength = Some(st);
            }
        }
    }
    salts
}

pub fn detect_form(text: &str) -> Form {
    let t = text.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| t.contains(w));
    if has(&["inhaler", "respules", "rotacap"]) {
        Form::Inhaler
    } else if has(&["injection", "vial", "ampoule", "infusion"]) {
        Form::Injection
    } else if has(&["eye drop", "ear drop", "eye/ear drop", "eye and ear drop"]) {
        Form::EyeEar
    } else if t.split(|c: char| !c.is_ascii_alphabetic()).any(|w| matches!(w, "gel" | "cream" | "ointment" | "lotion")) {
        Form::Topical
    } else if has(&["capsule"]) {
        Form::Capsule
    } else if has(&["tablet"]) {
        Form::Tablet
    } else if has(&["syrup", "suspension", "solution", "drop", "elixir", "liquid"]) {
        Form::Liquid
    } else {
        Form::Other
    }
}

pub fn detect_release(text: &str) -> Release {
    if EXTENDED.is_match(text) { Release::Extended } else { Release::Standard }
}

fn unit_of(word: &str) -> PackUnit {
    match word.to_lowercase().as_str() {
        "ml" => PackUnit::Ml,
        "g" | "gm" => PackUnit::G,
        _ => PackUnit::Unit,
    }
}

fn whole_count(s: &str) -> Option<u32> {
    let v: f64 = s.parse().ok()?;
    // Whole, positive and bounded, so the cast is exact.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    (v.fract() == 0.0 && v >= 1.0 && v <= f64::from(u32::MAX)).then_some(v as u32)
}

/// Jan Aushadhi unit sizes: "10's", "60 ML", "15 g", "200ml Bottle".
pub fn parse_jan_aushadhi_pack(text: &str) -> Option<Pack> {
    if let Some(c) = JA_VOLUME.captures(text) {
        return Some(Pack { count: whole_count(c.get(1)?.as_str())?, unit: unit_of(c.get(2)?.as_str()) });
    }
    let c = JA_COUNT.captures(text)?;
    Some(Pack { count: whole_count(c.get(1)?.as_str())?, unit: PackUnit::Unit })
}

/// Brand pack labels: "strip of 10 tablets", "bottle of 100 ml Syrup".
pub fn parse_brand_pack(text: &str) -> Option<Pack> {
    let c = BRAND_PACK.captures(text)?;
    Some(Pack { count: whole_count(c.get(1)?.as_str())?, unit: unit_of(c.get(2)?.as_str()) })
}

/// Human label for a salt list: "Amoxicillin 500mg + Clavulanic Acid 125mg".
pub fn describe_salts(salts: &[Salt]) -> String {
    salts
        .iter()
        .map(|s| {
            let name = s
                .name
                .split(' ')
                .map(|w| {
                    let mut c = w.chars();
                    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
                })
                .collect::<Vec<_>>()
                .join(" ");
            match &s.strength {
                Some(st) => format!("{name} {st}"),
                None => name,
            }
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mg(name: &str, mg: f64) -> Salt {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Salt::mg(name, (mg * 1000.0) as u64)
    }
    fn text(name: &str, t: &str) -> Salt {
        Salt { name: name.into(), strength: Some(Strength::Text(t.into())) }
    }

    #[test]
    fn normalizes_salts() {
        assert_eq!(normalize_salt("Amoxycillin"), "amoxicillin");
        assert_eq!(normalize_salt("Metformin Hydrochloride"), "metformin");
        assert_eq!(normalize_salt("Cetirizine Dihydrochloride"), "cetirizine");
        assert_eq!(normalize_salt("Montelukast Sodium"), "montelukast");
        assert_eq!(normalize_salt("Potassium Clavulanate"), "clavulanic acid");
        assert_eq!(normalize_salt("Acetaminophen"), "paracetamol");
        assert_eq!(normalize_salt("Amlodipine Besylate"), "amlodipine");
        assert_eq!(normalize_salt("Magnesium Citrate"), "magnesium citrate");
    }

    #[test]
    fn converts_mass_units() {
        assert_eq!(to_micrograms(1.0, "g"), Some(1_000_000));
        assert_eq!(to_micrograms(500.0, "mcg"), Some(500));
        assert_eq!(to_micrograms(0.5, "mg"), Some(500));
        assert_eq!(to_micrograms(5.0, "ml"), None);
        assert_eq!(to_micrograms(f64::INFINITY, "mg"), None);
    }

    #[test]
    fn detects_form() {
        assert_eq!(detect_form("Atorvastatin Tablets IP 10mg"), Form::Tablet);
        assert_eq!(detect_form("strip of 10 capsules"), Form::Capsule);
        assert_eq!(detect_form("bottle of 60 ml Syrup"), Form::Liquid);
        assert_eq!(detect_form("Diclofenac Gel IP 1.16%"), Form::Topical);
        assert_eq!(detect_form("Diclofenac Sodium Injection IP 25mg per ml"), Form::Injection);
        assert_eq!(detect_form("Budesonide Respules"), Form::Inhaler);
        assert_eq!(detect_form("Moxifloxacin Eye Drops"), Form::EyeEar);
        assert_eq!(detect_form("Something odd"), Form::Other);
        // "angel" contains "gel" but is not a gel
        assert_eq!(detect_form("Angel Tablet"), Form::Tablet);
    }

    #[test]
    fn detects_release_treating_gastro_resistant_as_standard() {
        assert_eq!(detect_release("Metformin 500mg (Prolonged release) Tablets"), Release::Extended);
        assert_eq!(detect_release("Glycomet SR 500 Tablet"), Release::Extended);
        assert_eq!(detect_release("Pantoprazole Gastro Resistant Tablets"), Release::Standard);
        assert_eq!(detect_release("Atorvastatin Tablets IP 10mg"), Release::Standard);
    }

    #[test]
    fn parses_brand_compositions() {
        assert_eq!(
            parse_brand_composition("Amoxycillin  (500mg) +  Clavulanic Acid (125mg)"),
            vec![mg("amoxicillin", 500.0), mg("clavulanic acid", 125.0)]
        );
        assert_eq!(parse_brand_composition("Ambroxol (30mg/5ml)"), vec![text("ambroxol", "30mg/5ml")]);
        assert_eq!(parse_brand_composition("Insulin (40IU)"), vec![text("insulin", "40iu")]);
        assert_eq!(parse_brand_composition("Vitamin X"), vec![Salt { name: "vitamin x".into(), strength: None }]);
        assert_eq!(parse_brand_composition(""), vec![]);
    }

    #[test]
    fn parses_jan_aushadhi_names() {
        assert_eq!(parse_jan_aushadhi_name("Atorvastatin Tablets IP 10mg"), vec![mg("atorvastatin", 10.0)]);
        assert_eq!(
            parse_jan_aushadhi_name("Paracetamol 125mg, Phenylephrine Hydrochloride 5mg and Cetirizine Dihydrochloride 2mg Suspension"),
            vec![mg("paracetamol", 125.0), mg("phenylephrine", 5.0), mg("cetirizine", 2.0)]
        );
        assert_eq!(
            parse_jan_aushadhi_name("Metformin Hydrochloride 1000mg (Prolonged Release) and Glimepiride 2mg Tablets IP"),
            vec![mg("metformin", 1000.0), mg("glimepiride", 2.0)]
        );
        assert_eq!(
            parse_jan_aushadhi_name("Potassium Citrate 1100mg and Magnesium Citrate 375mg Oral Solution"),
            vec![mg("potassium citrate", 1100.0), mg("magnesium citrate", 375.0)]
        );
        assert_eq!(parse_jan_aushadhi_name("Cetirizine Syrup IP 5 mg per 5 ml"), vec![text("cetirizine", "5mg/5ml")]);
        assert_eq!(
            parse_jan_aushadhi_name("Amoxycillin and Potassium Clavulanate Tablets IP 500mg+125mg"),
            vec![mg("amoxicillin", 500.0), mg("clavulanic acid", 125.0)]
        );
        assert_eq!(parse_jan_aushadhi_name("Glimepiride Tablets IP 0.5mg"), vec![mg("glimepiride", 0.5)]);
    }

    #[test]
    fn leaves_strength_unknown_when_counts_disagree() {
        let s = parse_jan_aushadhi_name("Alpha and Beta Tablets IP 10mg");
        assert!(s.iter().all(|x| x.strength.is_none()));
    }

    #[test]
    fn parses_packs() {
        assert_eq!(parse_jan_aushadhi_pack("10's"), Some(Pack { count: 10, unit: PackUnit::Unit }));
        assert_eq!(parse_jan_aushadhi_pack("60 ML"), Some(Pack { count: 60, unit: PackUnit::Ml }));
        assert_eq!(parse_jan_aushadhi_pack("15 g"), Some(Pack { count: 15, unit: PackUnit::G }));
        assert_eq!(parse_jan_aushadhi_pack("200ml Bottle"), Some(Pack { count: 200, unit: PackUnit::Ml }));
        assert_eq!(parse_jan_aushadhi_pack("1 kit"), None);
        assert_eq!(parse_jan_aushadhi_pack("0.5 ml"), None);
        assert_eq!(parse_brand_pack("strip of 10 tablets"), Some(Pack { count: 10, unit: PackUnit::Unit }));
        assert_eq!(parse_brand_pack("bottle of 100 ml Syrup"), Some(Pack { count: 100, unit: PackUnit::Ml }));
        assert_eq!(parse_brand_pack("tube of 30 gm Gel"), Some(Pack { count: 30, unit: PackUnit::G }));
        assert_eq!(parse_brand_pack("packet"), None);
    }

    #[test]
    fn describes_salts_for_display() {
        let salts = [Salt::mg("amoxicillin", 500_000), Salt::mg("clavulanic acid", 125_000)];
        assert_eq!(describe_salts(&salts), "Amoxicillin 500mg + Clavulanic Acid 125mg");
        assert_eq!(describe_salts(&[Salt { name: "x".into(), strength: None }]), "X");
    }
}
