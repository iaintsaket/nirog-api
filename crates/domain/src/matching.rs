use serde::{Deserialize, Serialize};

use crate::model::{Form, PackUnit, Release, Salt};

/// Two offers with the same key have the same ingredients, strengths, dosage
/// form, release type and pack unit, so they are exact substitutes and their
/// unit prices can be compared.
/// Marks the salts part of an unread key. Salts parts are `name:strength` tokens, so a
/// token with no `:` can never equal or overlap a real composition.
const UNREAD: &str = "?";

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MatchKey(String);

impl MatchKey {
    /// Returns `None` when any strength is unknown: we never guess a match.
    pub fn new(salts: &[Salt], form: Form, release: Release, unit: PackUnit) -> Option<Self> {
        if salts.is_empty() {
            return None;
        }
        let mut tokens = salts
            .iter()
            .map(|s| s.strength.as_ref().map(|st| format!("{}:{st}", s.name)))
            .collect::<Option<Vec<_>>>()?;
        tokens.sort();
        Some(Self(format!(
            "{}|{}|{}|{}",
            form.as_str(),
            release.as_str(),
            tokens.join("+"),
            unit.as_str()
        )))
    }

    /// A key for a product whose ingredients we could not read: it matches only that
    /// product (same identity), never a substitute or a near match.
    pub fn unread(form: Form, unit: PackUnit, identity: &str) -> Self {
        let hash = identity.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
        Self(format!(
            "{}|{}|{UNREAD}{hash:016x}|{}",
            form.as_str(),
            Release::Standard.as_str(),
            unit.as_str()
        ))
    }

    /// True for a key made by [`MatchKey::unread`].
    pub fn is_unread(&self) -> bool {
        self.0
            .split('|')
            .nth(2)
            .is_some_and(|s| s.starts_with(UNREAD))
    }

    /// Rebuild a key that was produced by [`MatchKey::new`] (from storage or a URL).
    /// Only the shape is checked; an unknown key simply matches nothing.
    pub fn parse(raw: &str) -> Option<Self> {
        let ok =
            raw.len() <= 512 && raw.split('|').count() == 4 && !raw.chars().any(char::is_control);
        ok.then(|| Self(raw.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Strength;

    fn key(salts: &[Salt], form: Form, release: Release) -> Option<MatchKey> {
        MatchKey::new(salts, form, release, PackUnit::Unit)
    }

    #[test]
    fn ignores_salt_order() {
        let a = key(
            &[Salt::mg("b", 3000), Salt::mg("a", 2000)],
            Form::Tablet,
            Release::Standard,
        );
        let b = key(
            &[Salt::mg("a", 2000), Salt::mg("b", 3000)],
            Form::Tablet,
            Release::Standard,
        );
        assert_eq!(a, b);
        assert_eq!(a.unwrap().as_str(), "tablet|standard|a:2mg+b:3mg|unit");
    }

    #[test]
    fn form_release_and_unit_change_the_key() {
        let s = [Salt::mg("a", 2000)];
        let base = key(&s, Form::Tablet, Release::Standard);
        assert_ne!(base, key(&s, Form::Tablet, Release::Extended));
        assert_ne!(base, key(&s, Form::Capsule, Release::Standard));
        assert_ne!(
            base,
            MatchKey::new(&s, Form::Tablet, Release::Standard, PackUnit::Ml)
        );
    }

    #[test]
    fn no_key_when_any_strength_is_unknown() {
        assert_eq!(
            key(
                &[Salt {
                    name: "a".into(),
                    strength: None
                }],
                Form::Tablet,
                Release::Standard
            ),
            None
        );
        assert_eq!(key(&[], Form::Tablet, Release::Standard), None);
        let text = Salt {
            name: "a".into(),
            strength: Some(Strength::Text("5mg/5ml".into())),
        };
        assert!(key(&[text], Form::Liquid, Release::Standard).is_some());
    }

    #[test]
    fn unread_key_matches_only_its_own_product() {
        let a = MatchKey::unread(Form::Liquid, PackUnit::Ml, "brand|savlon|itc|100ml");
        let b = MatchKey::unread(Form::Liquid, PackUnit::Ml, "brand|lifebuoy|hul|100ml");
        assert_eq!(
            a,
            MatchKey::unread(Form::Liquid, PackUnit::Ml, "brand|savlon|itc|100ml")
        );
        assert_ne!(a, b);
        assert!(
            a.is_unread()
                && !key(&[Salt::mg("a", 1)], Form::Tablet, Release::Standard)
                    .unwrap()
                    .is_unread()
        );
        assert_eq!(MatchKey::parse(a.as_str()), Some(a.clone()));
        assert_eq!(crate::nearby::differences(&a, &b), None);
    }

    #[test]
    fn parse_checks_shape_only() {
        assert!(MatchKey::parse("tablet|standard|a:2mg|unit").is_some());
        assert!(MatchKey::parse("nope").is_none());
        assert!(MatchKey::parse("a|b|c|d\n").is_none());
        assert!(MatchKey::parse(&"x|".repeat(300)).is_none());
    }
}
