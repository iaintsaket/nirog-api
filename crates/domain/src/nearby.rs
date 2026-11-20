//! How far apart two formulas are, for "close matches" when no exact substitute exists.
//! Close matches are never substitutes: callers must show what differs.

use std::collections::BTreeMap;
use std::fmt;

use crate::matching::MatchKey;

/// One way a nearby formula differs from yours.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Difference {
    /// Yours has this salt; theirs does not.
    Missing { salt: String },
    /// Theirs has a salt yours does not.
    Extra { salt: String, strength: String },
    /// Same salt, different strength.
    Strength {
        salt: String,
        ours: String,
        theirs: String,
    },
}

fn title(name: &str) -> String {
    name.split(' ')
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect())
                .unwrap_or_default()
        })
        .collect::<Vec<String>>()
        .join(" ")
}

impl fmt::Display for Difference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { salt } => write!(f, "No {}", title(salt)),
            Self::Extra { salt, strength } => write!(f, "Also has {} {strength}", title(salt)),
            Self::Strength { salt, ours, theirs } => {
                write!(f, "{} {theirs} instead of {ours}", title(salt))
            }
        }
    }
}

/// form|release, salts by name, unit.
fn split(key: &MatchKey) -> Option<(&str, BTreeMap<&str, &str>, &str)> {
    let mut parts = key.as_str().split('|');
    let (form, release, salts, unit) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    let salts = salts
        .split('+')
        .map(|t| t.split_once(':'))
        .collect::<Option<BTreeMap<_, _>>>()?;
    Some((&key.as_str()[..form.len() + 1 + release.len()], salts, unit))
}

/// What differs between your formula and theirs, or `None` when they are not comparable at all
/// (different form, release or pack unit, or no salt in common). An empty list means identical.
pub fn differences(ours: &MatchKey, theirs: &MatchKey) -> Option<Vec<Difference>> {
    let (o_kind, o, o_unit) = split(ours)?;
    let (t_kind, t, t_unit) = split(theirs)?;
    if o_kind != t_kind || o_unit != t_unit || !o.keys().any(|k| t.contains_key(k)) {
        return None;
    }
    let mut out = Vec::new();
    for (salt, strength) in &o {
        match t.get(salt) {
            None => out.push(Difference::Missing {
                salt: (*salt).to_owned(),
            }),
            Some(s) if s != strength => out.push(Difference::Strength {
                salt: (*salt).to_owned(),
                ours: (*strength).to_owned(),
                theirs: (*s).to_owned(),
            }),
            Some(_) => {}
        }
    }
    for (salt, strength) in &t {
        if !o.contains_key(salt) {
            out.push(Difference::Extra {
                salt: (*salt).to_owned(),
                strength: (*strength).to_owned(),
            });
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(s: &str) -> MatchKey {
        MatchKey::parse(s).unwrap_or_else(|| unreachable!())
    }
    const DOLOTEZ: &str = "topical|standard|capsaicin:0.01%+diclofenac:1.16%+linseed oil:3%+menthol:5%+methyl salicylate:10%|g";

    #[test]
    fn names_each_difference_in_plain_words() {
        let strength = k(
            "topical|standard|capsaicin:0.025%+diclofenac:1.16%+linseed oil:3%+menthol:5%+methyl salicylate:10%|g",
        );
        let d = differences(&k(DOLOTEZ), &strength).unwrap_or_default();
        assert_eq!(
            d.iter().map(ToString::to_string).collect::<Vec<_>>(),
            ["Capsaicin 0.025% instead of 0.01%"]
        );

        let missing = k(
            "topical|standard|diclofenac:1.16%+linseed oil:3%+menthol:5%+methyl salicylate:10%|g",
        );
        let d = differences(&k(DOLOTEZ), &missing).unwrap_or_default();
        assert_eq!(
            d.iter().map(ToString::to_string).collect::<Vec<_>>(),
            ["No Capsaicin"]
        );

        let extra = k(
            "topical|standard|capsaicin:0.01%+diclofenac:1.16%+linseed oil:3%+menthol:5%+methyl salicylate:10%+thiocolchicoside:0.125%|g",
        );
        let d = differences(&k(DOLOTEZ), &extra).unwrap_or_default();
        assert_eq!(
            d.iter().map(ToString::to_string).collect::<Vec<_>>(),
            ["Also has Thiocolchicoside 0.125%"]
        );

        assert_eq!(differences(&k(DOLOTEZ), &k(DOLOTEZ)), Some(vec![]));
    }

    #[test]
    fn other_forms_units_or_unrelated_salts_are_not_comparable() {
        assert_eq!(
            differences(&k(DOLOTEZ), &k("tablet|standard|capsaicin:0.01%|unit")),
            None
        );
        assert_eq!(
            differences(&k(DOLOTEZ), &k("topical|standard|capsaicin:0.01%|ml")),
            None
        );
        assert_eq!(
            differences(&k(DOLOTEZ), &k("topical|standard|ketoprofen:2.5%|g")),
            None
        );
    }
}
