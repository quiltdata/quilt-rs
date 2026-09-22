//! The build this is, resolved once so every gate can be handed one.
//!
//! A construction gate is only half enforced where Settings draws it — the route
//! has to refuse a stale value too — so the build is a fact several places ask
//! about. Asking it once, here, is what leaves each of those places testable.

/// Which build this is, as a value a test can supply.
///
/// The one `cfg!(debug_assertions)` in the UI crate lives in `resolve`. Everything
/// that gates on the build takes this instead, because tests compile with
/// `debug_assertions` on and so can never reach the release branch through `cfg!`.
/// `src-tauri`'s `Sinks::resolve` is the same shape for the same reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildProfile {
    /// A `trunk watch` build — the developer's.
    Development,
    /// A `trunk build --release` build — the reader's.
    Release,
}

impl BuildProfile {
    pub fn resolve() -> Self {
        // `trunk watch` serves the dev profile and `trunk build --release` the
        // release one — see `beforeDevCommand`/`beforeBuildCommand` in
        // `src-tauri/tauri.conf.json`, which is what makes this track the
        // distinction the gates mean.
        if cfg!(debug_assertions) {
            Self::Development
        } else {
            Self::Release
        }
    }

    /// Whether a control that exists only to build unfinished work is offered
    /// and obeyed.
    pub fn allows_construction_gates(self) -> bool {
        matches!(self, Self::Development)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests build with `debug_assertions` on, so this pins the branch a
    /// developer actually runs — the same claim, for the same reason, as
    /// `telemetry.rs`'s `a_local_build_resolves_to_development`.
    #[test]
    fn a_local_build_resolves_to_development() {
        assert_eq!(BuildProfile::resolve(), BuildProfile::Development);
    }

    /// The release branch is unreachable through `resolve` from a test, which
    /// is the whole reason the profile is a value: every gate below takes one.
    #[test]
    fn a_release_build_offers_no_construction_gate() {
        assert!(!BuildProfile::Release.allows_construction_gates());
        assert!(BuildProfile::Development.allows_construction_gates());
    }
}
