//! Use cases for Nirog and the ports (traits) they need.
//!
//! Adapters (Postgres, Google, CSV sources, HTTP) implement these traits in
//! their own crates; the server app wires them together.

pub mod buy;
pub mod error;
pub mod model;
pub mod ports;
pub mod security;
pub mod sellers;
pub mod services;
pub mod stock;

#[cfg(test)]
mod fakes;

pub use error::{AppError, IdentityError, RepoError};
pub use model::{
    BrandAlias, BuyLink, Identity, LinkKind, NewSavedMatch, RecentMedicine, SavedMatch,
    SearchGroup, User, UserId,
};
