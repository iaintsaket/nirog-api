use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("no offers to compare")]
    NoOffers,
    #[error("offers do not share one match key")]
    MixedKeys,
    #[error("invalid value: {0}")]
    Invalid(&'static str),
}
