//! Nirog domain: medicines, offers and the exact-substitution rules.
//!
//! This crate is pure: no I/O, no async, no framework types. Every other
//! crate depends on it; it depends on nothing internal.

pub mod compare;
pub mod error;
pub mod matching;
pub mod model;
pub mod parse;

pub use compare::Comparison;
pub use error::DomainError;
pub use matching::MatchKey;
pub use model::{Form, Offer, OfferId, Pack, PackUnit, Paise, Release, Salt, SourceKind, Strength};
