//! Shared contract types for Feuer's immutable-object range tiers.
//!
//! The top-level `feuer` crate re-exports these types. Keeping their single
//! definitions below that public boundary lets the private memory and disk
//! components exchange exact object identities and byte ranges without a
//! dependency cycle.

mod download;
mod range;

pub use download::{Download, DownloadError};
pub use range::{ByteRange, InvalidRange};

/// The complete UTF-8 identity of one immutable object.
pub type ObjectKey = String;
