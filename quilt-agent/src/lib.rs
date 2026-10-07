//! Unattended instrument-to-cloud mover (lab-to-cloud increment 1).
//!
//! Watches instrument run folders, decides when a run is complete, lands its
//! files in S3, writes a sentinel recording what was observed, and publishes
//! the run as a fresh Quilt package revision. Outbound HTTPS only; reads the
//! source, never writes to it.
// Same call as quilt-rs: a binary's internal surface where every fn returns
// `Result`, so a per-fn "# Errors" section adds noise, not information.
#![allow(clippy::missing_errors_doc)]

pub mod agent;
pub mod boundary;
pub mod packager;
pub mod profile;
pub mod sentinel;
pub mod spool;
pub mod watch;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid profile: {0}")]
    Profile(String),
    #[error("invalid sentinel: {0}")]
    Sentinel(String),
    /// Not transient: the run is parked with this reason.
    #[error("{0}")]
    Refused(String),
    #[error(transparent)]
    Quilt(#[from] quilt_rs::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Glob(#[from] globset::Error),
}
