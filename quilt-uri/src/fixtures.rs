//! Shared test fixtures.
//!
//! Exposed under the `test-support` feature so downstream crates can reuse
//! them in their own tests.

use crate::Host;

/// The canonical catalog host used throughout the test suite.
#[must_use]
pub fn host() -> Host {
    Host::from(url::Host::Domain("test.quilt.dev".to_string()))
}
