//! Shared test fixtures.
//!
//! Exposed under the `test-support` feature so downstream crates can reuse
//! them in their own tests.
//!
//! Every host here sits under `.test`, a top-level domain reserved for
//! testing (RFC 6761). These names never resolve, so a test can't reach a
//! real Quilt stack by accident.
//!
//! A test that needs one host uses [`host`]. A test that needs two different
//! hosts uses [`one_host`] and [`another_host`], and never [`host`], so it
//! can't mean two hosts while writing the same name twice.

use crate::Host;

/// The default catalog host for tests: `quilt.test`.
#[must_use]
pub fn host() -> Host {
    domain("quilt.test")
}

/// The first of two different catalog hosts: `one.quilt.test`.
#[must_use]
pub fn one_host() -> Host {
    domain("one.quilt.test")
}

/// The second of two different catalog hosts: `another.quilt.test`.
#[must_use]
pub fn another_host() -> Host {
    domain("another.quilt.test")
}

fn domain(name: &str) -> Host {
    Host::from(url::Host::Domain(name.to_string()))
}
