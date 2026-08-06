//! Which remote entries are ticked for download, and the rules that resolve and
//! update that choice.
//!
//! The state itself is held by the page component, *above* the resource
//! boundary: `InstalledPackageContent` receives its data by value out of the
//! resource's `Suspend`, so every re-resolution re-runs it and every signal
//! created inside it is a brand-new signal. A selection created down there is
//! destroyed by any refresh — which is the bug this module exists to fix.
//!
//! Everything here is a pure function over the state, so the rules are testable
//! on the host target where no DOM exists.

use std::collections::BTreeSet;

/// Which remote entries are ticked for download.
///
/// Two states, and the asymmetry between them is load-bearing: [`Self::All`]
/// pins no names, so a remote entry that appears in a later refresh is already
/// covered by it, while [`Self::Subset`] pins names, so an arrival is never
/// ticked into a download the user has not seen. Both behaviours fall out of
/// [`resolve`] rather than each needing its own handling of new files.
///
/// The variants name the **extent**, not the provenance. A variant called
/// "default" would come to mean something else the day the screen stops opening
/// with everything ticked — so where the screen starts lives in one
/// [`Default`](Self::default) impl instead, and no variant can lie about itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum RemoteSelection {
    /// Every remote entry the package offers — including ones that arrive in a
    /// later refresh.
    ///
    /// This once carried a `chosen` flag distinguishing a deliberate *Select
    /// all* from the screen's opening state, reserved for the
    /// always-download-new-files opt-in. That opt-in exists now and asks
    /// outright — see the [sync scope](super::sync_scope) control — so nothing
    /// has to infer intent from a click sequence, and the flag is gone.
    All,
    /// A hand-picked set of paths — possibly empty, which is the user's decision
    /// to pick nothing and must not spring back to everything.
    Subset(BTreeSet<String>),
}

impl Default for RemoteSelection {
    /// Where the screen opens: everything ticked, not yet asked for.
    fn default() -> Self {
        Self::All
    }
}

/// The paths ticked right now: the selection resolved against the remote entries
/// the package currently offers.
///
/// Intersecting on read is what lets the selection survive a refresh with no
/// cleanup pass anywhere: a name that has since been installed, ignored, or
/// dropped from the manifest is no longer a remote entry, so it falls out here.
/// Nothing has to notice the change and prune.
pub(super) fn resolve(selection: &RemoteSelection, remote: &BTreeSet<String>) -> BTreeSet<String> {
    match selection {
        RemoteSelection::All => remote.clone(),
        RemoteSelection::Subset(picked) => picked.intersection(remote).cloned().collect(),
    }
}

/// Whether every remote entry is ticked — the header checkbox's checked state.
///
/// False with no remote entries at all: there is nothing to have selected, and
/// the control is not rendered in that case anyway.
pub(super) fn all_selected(selected: &BTreeSet<String>, remote: &BTreeSet<String>) -> bool {
    !remote.is_empty() && selected.len() == remote.len()
}

/// Whether the header checkbox should draw **indeterminate**: some remote
/// entries are ticked, but not all.
///
/// Without this a partial selection draws an *empty* box, which reads as
/// "nothing is selected" — a momentary misstatement while the selection died on
/// every refresh, a standing one now that it survives.
pub(super) fn partially_selected(selected: &BTreeSet<String>, remote: &BTreeSet<String>) -> bool {
    !selected.is_empty() && selected.len() < remote.len()
}

/// What a click on the header checkbox stores next: clear when everything is
/// already ticked, otherwise take everything.
///
/// Taking everything always yields [`RemoteSelection::All`], whatever state it
/// replaced — the click is a request for every remote entry, and `All` is the
/// only way to say that without pinning today's names.
pub(super) fn toggled_all(
    selection: &RemoteSelection,
    remote: &BTreeSet<String>,
) -> RemoteSelection {
    if all_selected(&resolve(selection, remote), remote) {
        RemoteSelection::Subset(BTreeSet::new())
    } else {
        RemoteSelection::All
    }
}

/// What a click on one row's checkbox stores next.
///
/// Two properties worth the code. It writes back the **resolved** set, so names
/// the package has stopped offering are pruned as a side effect of ordinary use.
/// And a tick that fills the list **collapses to [`RemoteSelection::All`]**:
/// a subset holding every current name would draw identically to `All` (header
/// checked, every row checked) yet diverge from it on the next refresh, and one
/// unreadable all-selected state is already one too many.
pub(super) fn toggled_path(
    selection: &RemoteSelection,
    remote: &BTreeSet<String>,
    path: &str,
) -> RemoteSelection {
    let mut picked = resolve(selection, remote);
    if !picked.remove(path) {
        picked.insert(path.to_owned());
    }
    if all_selected(&picked, remote) {
        RemoteSelection::All
    } else {
        RemoteSelection::Subset(picked)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RemoteSelection, all_selected, partially_selected, resolve, toggled_all, toggled_path,
    };
    use std::collections::BTreeSet;

    fn set<const N: usize>(names: [&str; N]) -> BTreeSet<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    /// The screen opens with everything ticked.
    #[test]
    fn the_screen_opens_at_everything() {
        assert_eq!(RemoteSelection::default(), RemoteSelection::All);
        let remote = set(["a.csv", "b.csv"]);
        assert_eq!(resolve(&RemoteSelection::default(), &remote), remote);
    }

    /// Picking nothing is a decision, not an absence of one: an explicit empty
    /// subset stays empty instead of springing back to everything.
    #[test]
    fn an_empty_subset_stays_empty() {
        let remote = set(["a.csv", "b.csv"]);
        let picked = RemoteSelection::Subset(BTreeSet::new());
        assert!(resolve(&picked, &remote).is_empty());
    }

    /// **The asymmetry, and the reason there are two states.** A file that
    /// arrives in a later refresh is covered by *everything* and is never added
    /// to a hand-picked subset — so a download being composed cannot silently
    /// gain a file the user never saw. Both directions asserted, so this cannot
    /// pass by the two states behaving alike.
    #[test]
    fn everything_covers_a_later_arrival_a_subset_does_not() {
        let before = set(["a.csv"]);
        let after = set(["a.csv", "new.csv"]);

        let everything = RemoteSelection::All;
        assert_eq!(resolve(&everything, &before), set(["a.csv"]));
        assert_eq!(resolve(&everything, &after), set(["a.csv", "new.csv"]));

        let hand_picked = RemoteSelection::Subset(set(["a.csv"]));
        assert_eq!(resolve(&hand_picked, &after), set(["a.csv"]));
    }

    /// A name the package no longer offers — installed since, ignored, gone from
    /// the manifest — falls out on read, with no cleanup pass anywhere.
    #[test]
    fn a_name_the_package_stopped_offering_falls_out() {
        let picked = RemoteSelection::Subset(set(["kept.csv", "installed.csv"]));
        assert_eq!(resolve(&picked, &set(["kept.csv"])), set(["kept.csv"]));
    }

    /// The header click clears a full selection and fills anything else.
    #[test]
    fn the_header_click_clears_when_full_and_fills_otherwise() {
        let remote = set(["a.csv", "b.csv"]);

        let full = RemoteSelection::All;
        assert_eq!(
            toggled_all(&full, &remote),
            RemoteSelection::Subset(BTreeSet::new())
        );

        for from in [
            RemoteSelection::Subset(BTreeSet::new()),
            RemoteSelection::Subset(set(["a.csv"])),
        ] {
            assert_eq!(
                toggled_all(&from, &remote),
                RemoteSelection::All,
                "filling from {from:?} takes everything"
            );
        }
    }

    /// Unticking one row from *everything* pins the rest — which is what makes
    /// the next arrival stay untucked.
    #[test]
    fn unticking_from_everything_pins_the_rest() {
        let remote = set(["a.csv", "b.csv", "c.csv"]);
        assert_eq!(
            toggled_path(&RemoteSelection::All, &remote, "b.csv"),
            RemoteSelection::Subset(set(["a.csv", "c.csv"]))
        );
    }

    /// Ticking rows up to a full list **collapses to everything** rather than
    /// leaving a subset of every name — the two would draw identically on screen
    /// and then disagree on the next refresh.
    #[test]
    fn ticking_the_last_row_collapses_to_everything() {
        let remote = set(["a.csv", "b.csv"]);
        let one_short = RemoteSelection::Subset(set(["a.csv"]));
        assert_eq!(
            toggled_path(&one_short, &remote, "b.csv"),
            RemoteSelection::All
        );
        // The contrast: one short of full stays a subset.
        assert_eq!(
            toggled_path(&RemoteSelection::Subset(BTreeSet::new()), &remote, "a.csv"),
            RemoteSelection::Subset(set(["a.csv"]))
        );
    }

    /// A row toggle writes back the *resolved* set, so a stale name the package
    /// stopped offering is pruned by ordinary use rather than accumulating.
    #[test]
    fn a_row_toggle_prunes_stale_names() {
        let remote = set(["a.csv", "b.csv"]);
        let stale = RemoteSelection::Subset(set(["a.csv", "gone.csv"]));
        assert_eq!(
            toggled_path(&stale, &remote, "b.csv"),
            RemoteSelection::All,
            "a.csv + b.csv is every remote entry; gone.csv is not carried"
        );
    }

    /// The header checkbox draws partial as partial: indeterminate for some,
    /// checked for all, and neither when nothing is ticked.
    #[test]
    fn the_header_draws_partial_as_partial() {
        let remote = set(["a.csv", "b.csv"]);

        let some = set(["a.csv"]);
        assert!(partially_selected(&some, &remote));
        assert!(!all_selected(&some, &remote));

        assert!(all_selected(&remote, &remote));
        assert!(!partially_selected(&remote, &remote));

        let none = BTreeSet::new();
        assert!(!partially_selected(&none, &remote));
        assert!(!all_selected(&none, &remote));
    }

    /// With no remote entries there is nothing selected to report, so the header
    /// is neither checked nor indeterminate.
    #[test]
    fn no_remote_entries_is_not_all_selected() {
        let empty = BTreeSet::new();
        assert!(!all_selected(&empty, &empty));
        assert!(!partially_selected(&empty, &empty));
    }
}
