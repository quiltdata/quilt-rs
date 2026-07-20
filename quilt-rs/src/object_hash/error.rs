//! Errors produced while computing or parsing object hashes.

use thiserror::Error;

/// Errors from object-hash construction, parsing, and multihash/multibase
/// conversions.
#[derive(Error, Debug)]
pub enum Error {
    #[error("Invalid multihash: {0}")]
    InvalidMultihash(String),

    #[error("Multihash error: {0}")]
    Multihash(#[from] multihash::Error),

    #[error("Multibase error: {0}")]
    Multibase(#[from] multibase::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
