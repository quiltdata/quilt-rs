//! Decides when a run folder is complete (`spec/boundary-methods.md`).
//!
//! Pure over an observation: no I/O, no clock reads, so every rule is a unit
//! test. One method per instrument; only `size_stable` returns a guess.

use std::fmt::Write as _;
use std::time::SystemTime;

use crate::profile::Boundary;
use crate::profile::Method;
use crate::watch::Member;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Pending,
    Complete {
        evidence: String,
        /// The newest marker's (or member's) mtime as the share reports it.
        instrument_mtime: Option<SystemTime>,
    },
    /// Something contradicts completion; the run never lands as it stands.
    Suspect(String),
}

/// What the observer knows about a run folder at `now`.
#[derive(Debug)]
pub struct Observation<'a> {
    pub members: &'a [Member],
    /// When the folder's listing last changed (a member added, resized or touched).
    pub last_change: SystemTime,
    pub now: SystemTime,
}

#[must_use]
pub fn decide(boundary: &Boundary, quiet_window_s: u64, obs: &Observation) -> Verdict {
    match boundary.method {
        Method::MarkerFile => marker_file(boundary, quiet_window_s, obs),
        Method::SizeStable => size_stable(boundary, quiet_window_s, obs),
        // ponytail: parsers wait on the site inventory (UNK-16); refusing loudly
        // beats guessing a vendor layout.
        Method::VendorManifest => Verdict::Suspect(format!(
            "manifest layout unknown: no parser for {}",
            boundary.manifest_kind.as_deref().unwrap_or("<unset>")
        )),
        // ponytail: needs the loopback endpoint; add with the first site that asks.
        Method::Explicit => Verdict::Suspect("explicit boundary not supported yet".to_string()),
    }
}

fn quiet_for(obs: &Observation) -> u64 {
    obs.now
        .duration_since(obs.last_change)
        .map_or(0, |d| d.as_secs())
}

fn marker_file(boundary: &Boundary, window_s: u64, obs: &Observation) -> Verdict {
    let globs = match crate::watch::glob_set(&boundary.markers) {
        Ok(g) => g,
        Err(e) => return Verdict::Suspect(format!("bad marker glob: {e}")),
    };
    let (markers, data): (Vec<&Member>, Vec<&Member>) =
        obs.members.iter().partition(|m| globs.is_match(&m.path));
    if markers.len() < boundary.markers.len() || markers.is_empty() {
        return Verdict::Pending;
    }
    let newest_marker = markers.iter().map(|m| m.mtime).max().expect("non-empty");

    // UNK-32: a member written after the marker means the instrument was not
    // done when it wrote the marker (the MinKNOW case). Re-arm instead of
    // parking: wait until the folder has been still for the whole window after
    // that last write, then complete and say which member came late. Parking
    // forever (SP-2's result) strands the run until an operator intervenes.
    let late: Vec<&Member> = if boundary.marker_must_be_newest {
        data.iter()
            .copied()
            .filter(|m| m.mtime > newest_marker)
            .collect()
    } else {
        Vec::new()
    };

    let quiet = quiet_for(obs);
    if quiet < window_s {
        return Verdict::Pending;
    }

    let names: Vec<String> = markers
        .iter()
        .map(|m| m.path.display().to_string())
        .collect();
    let mut evidence = format!(
        "{} present; folder stable {quiet}s (window {window_s}s); {} members",
        names.join(", "),
        data.len()
    );
    if let Some(last) = late.iter().max_by_key(|m| m.mtime) {
        let _ = write!(
            evidence,
            "; {} member(s) newer than the marker, last {}; confirm window re-armed",
            late.len(),
            last.path.display()
        );
    }
    Verdict::Complete {
        evidence: truncate(evidence),
        instrument_mtime: Some(newest_marker),
    }
}

fn size_stable(boundary: &Boundary, window_s: u64, obs: &Observation) -> Verdict {
    if obs.members.len() < boundary.min_members {
        return Verdict::Pending;
    }
    let quiet = quiet_for(obs);
    if quiet < window_s {
        return Verdict::Pending;
    }
    Verdict::Complete {
        evidence: truncate(format!(
            "no size/mtime change across {} members for {quiet}s (window {window_s}s)",
            obs.members.len()
        )),
        instrument_mtime: obs.members.iter().map(|m| m.mtime).max(),
    }
}

/// The sentinel caps evidence at 4 KiB.
fn truncate(mut s: String) -> String {
    const MAX: usize = 4096;
    if s.len() > MAX {
        let mut end = MAX;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;
    use std::time::Duration;

    const T0: u64 = 1_800_000_000;

    fn at(s: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(T0 + s)
    }

    fn member(path: &str, mtime: u64) -> Member {
        Member {
            path: PathBuf::from(path),
            size: 10,
            mtime: at(mtime),
        }
    }

    fn marker_boundary() -> Boundary {
        Boundary {
            method: Method::MarkerFile,
            markers: vec!["done.txt".to_string()],
            marker_must_be_newest: true,
            confirm_window_s: 120,
            stable_for_s: 900,
            min_members: 1,
            manifest_kind: None,
        }
    }

    fn decide_at(
        b: &Boundary,
        window: u64,
        members: &[Member],
        last_change: u64,
        now: u64,
    ) -> Verdict {
        decide(
            b,
            window,
            &Observation {
                members,
                last_change: at(last_change),
                now: at(now),
            },
        )
    }

    #[test]
    fn marker_absent_is_pending() {
        let m = [member("a.fcs", 0)];
        assert_eq!(
            decide_at(&marker_boundary(), 120, &m, 0, 10_000),
            Verdict::Pending
        );
    }

    #[test]
    fn marker_waits_out_the_window() {
        let m = [member("a.fcs", 0), member("done.txt", 10)];
        assert_eq!(
            decide_at(&marker_boundary(), 120, &m, 10, 100),
            Verdict::Pending
        );
        assert!(matches!(
            decide_at(&marker_boundary(), 120, &m, 10, 130),
            Verdict::Complete { instrument_mtime: Some(t), .. } if t == at(10)
        ));
    }

    /// SP-2 case (c): the marker lands before the last member. Never complete
    /// while that member is still being written; complete once it has been
    /// still for the window, and say it came late.
    #[test]
    fn late_member_rearms_instead_of_parking() {
        let m = [
            member("a.fcs", 0),
            member("done.txt", 10),
            member("b.fcs", 40),
        ];
        assert_eq!(
            decide_at(&marker_boundary(), 120, &m, 40, 130),
            Verdict::Pending
        );
        match decide_at(&marker_boundary(), 120, &m, 40, 160) {
            Verdict::Complete { evidence, .. } => {
                assert!(evidence.contains("re-armed"), "{evidence}");
                assert!(evidence.contains("b.fcs"), "{evidence}");
            }
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    #[test]
    fn ttl_floor_holds_a_marker_run_back() {
        let m = [member("a.fcs", 0), member("done.txt", 10)];
        // The 120 s confirm window is met at 130, but the mount's cache TTL floors it at 300.
        assert_eq!(
            decide_at(&marker_boundary(), 300, &m, 10, 130),
            Verdict::Pending
        );
        assert!(matches!(
            decide_at(&marker_boundary(), 300, &m, 10, 310),
            Verdict::Complete { .. }
        ));
    }

    #[test]
    fn size_stable_completes_after_quiet() {
        let b = Boundary {
            method: Method::SizeStable,
            ..marker_boundary()
        };
        let m = [member("a.raw", 0)];
        assert_eq!(decide_at(&b, 900, &m, 0, 899), Verdict::Pending);
        assert!(matches!(
            decide_at(&b, 900, &m, 0, 900),
            Verdict::Complete { .. }
        ));
    }

    #[test]
    fn vendor_manifest_without_parser_is_suspect() {
        let b = Boundary {
            method: Method::VendorManifest,
            manifest_kind: Some("ont_final_summary".to_string()),
            ..marker_boundary()
        };
        assert!(matches!(
            decide_at(&b, 60, &[member("x", 0)], 0, 10_000),
            Verdict::Suspect(r) if r.contains("ont_final_summary")
        ));
    }
}
