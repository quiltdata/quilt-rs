//! The state vocabulary: one function, and the only place words are chosen.
//!
//! §2 of the data record: the wire carries a discriminator and never prose, for two
//! reasons. The vocabulary is a UI artifact that will move again, and moving it must
//! not need a backend release. And **the same state needs different words in
//! different places** — which is why [`render`] takes a [`Site`] and not just a
//! state.
//!
//! The words are a property of the state AND where it draws. `Behind` and
//! `RoleDenied` are the two that differ by site; every other state says the same
//! thing wherever it draws.

use serde::Deserialize;

use super::StateTone;

/// A package's resolved state, as it crosses the wire.
///
/// Internally tagged, so the JSON is `{"kind": "diverged"}`. `Unknown` is
/// `#[serde(other)]`, which is what stops a `kind` this build has never heard of
/// from failing the whole payload — and it carries no data because
/// `#[serde(other)]` accepts only unit variants, and does not need to: the message
/// for an unexplained pause travels on the watcher payload's `paused` list, not in here.
/// `Hash`, so a queue row can be keyed on the state that draws it — see
/// `QueueItem::key` in `pages/main_page/queue.rs`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PackageState {
    Latest,
    Behind,
    PendingChanges {
        files: usize,
    },
    PendingCommit,
    Diverged,
    PullConflict {
        files: Vec<String>,
    },
    /// `None` when the denial is certain but the role query behind the wording
    /// failed. The denial still stands — the bucket refused — so suppressing the
    /// state would lose a real fact; it simply cannot be named.
    RoleDenied {
        role: Option<String>,
    },
    NoRemote,
    Unpublished,
    /// Autosync stopped for this package for a reason no other state covers —
    /// §5's row 3, which nothing rendered until this existed (qhq-8mgw.36). The
    /// pauses that DO have a state resolve into it instead, in the light phase.
    ///
    /// Declared above `Unknown` on purpose: `#[serde(other)]` swallows any kind
    /// this build does not name, so a missing arm here would not fail — it would
    /// silently render "Sync stopped", which is a different and stronger claim.
    Paused,
    #[serde(other)]
    Unknown,
}

/// Where a label is being drawn. Not decoration: two states word themselves
/// differently here, so a mapping keyed on state alone is wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Site {
    /// A row in the package list, which is quiet and pairs against `Latest`.
    ListRow,
    /// A row in the attention queue, which reads as a clause after the package's
    /// name.
    QueueRow,
    /// A cause several packages share, stated once above them — as the heading of a
    /// queue group, or as the annotation on a bucket heading in the list. A heading
    /// and not a clause, so it words itself as the list does, except that it names
    /// the role a denial was refused under: the whole point of stating a cause once
    /// is that there is room to say which one.
    Cause,
}

/// The operation a state offers, as a closed set.
///
/// An enum and not the label string: the queue turns each of these into a route
/// (`pages/main_page/queue.rs`, `action_href`), and a string makes that mapping
/// something the compiler cannot check. Adding a verb here fails every
/// non-exhaustive match that consumes one, which is the whole point — the
/// vocabulary and its routes live in different files owned by different work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageAction {
    GetLatest,
    Publish,
    /// `Resolve`, never `Merge`: no merge operation exists — resolving is a
    /// package-level choice between Certify Latest and Reset Local.
    Resolve,
    ChooseS3Bucket,
}

impl PackageAction {
    /// The button's words. Still the vocabulary's and not the caller's — this
    /// file is the only place they are chosen.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::GetLatest => "Get latest",
            Self::Publish => "Publish",
            Self::Resolve => "Resolve",
            Self::ChooseS3Bucket => "Choose S3 bucket",
        }
    }
}

/// What to draw for one state at one site.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rendered {
    pub words: String,
    pub tone: StateTone,
    /// The operation that fixes this state, or `None` when the app has none.
    pub action: Option<PackageAction>,
}

/// The vocabulary, in one place.
///
/// Three functions and not one match, because only one of the three things a state
/// resolves to depends on where it draws. Severity and the operation on offer are
/// properties of the state itself; the words are a property of the state AND the
/// site, which is why the clause table below has two arms per state and the other
/// two tables have one. Folded into a single `(state, site)` match, every tone and
/// every action would be written twice and could drift apart in the gap.
#[must_use]
pub fn render(state: &PackageState, site: Site) -> Rendered {
    Rendered {
        words: words(state, site),
        tone: tone(state),
        action: action(state),
    }
}

/// The words, which are the only part that depends on the site.
///
/// The list names a state: a short noun phrase in a chip beside the package, and a
/// shared cause is worded the same way for the same reason — it is a heading. The
/// queue row continues a sentence the package name started — `org/dataset-c` + `has
/// conflicts in 2 files` — so its words open lower-case and read as a clause about
/// that package. A state whose queue arm reads as a noun phrase is a bug the tests
/// catch two ways: the whole mapping, and the opening capital.
///
/// Counts are interpolated from the data here rather than sent as their own number
/// (§1): `PullConflict` counts the paths it was given, so the label cannot disagree
/// with the list it describes.
fn words(state: &PackageState, site: Site) -> String {
    match (state, site) {
        // The one state a cause words differently from the list, and the reason the
        // site exists: a heading has room to name the role, a chip does not. Every
        // other state falls through to the list's phrasing at the foot of the match.
        (PackageState::RoleDenied { role: Some(role) }, Site::Cause) => {
            format!("No access as {role}")
        }

        (PackageState::Latest, Site::ListRow) => "Latest".to_string(),
        // Never drawn: the queue is the states that need a decision. Worded anyway,
        // because a function that returns words for every state cannot have a hole.
        (PackageState::Latest, Site::QueueRow) => "is up to date".to_string(),

        (PackageState::Behind, Site::ListRow) => "Not the latest".to_string(),
        (PackageState::Behind, Site::QueueRow) => "has a newer revision".to_string(),

        (PackageState::PendingChanges { files: 1 }, Site::ListRow) => "1 file changed".to_string(),
        (PackageState::PendingChanges { files: 1 }, Site::QueueRow) => {
            "has 1 changed file".to_string()
        }
        (PackageState::PendingChanges { files }, Site::ListRow) => {
            format!("{files} files changed")
        }
        (PackageState::PendingChanges { files }, Site::QueueRow) => {
            format!("has {files} changed files")
        }

        (PackageState::PendingCommit, Site::ListRow) => "Revision not published".to_string(),
        (PackageState::PendingCommit, Site::QueueRow) => {
            "has a revision it has not published".to_string()
        }

        (PackageState::Diverged, Site::ListRow) => "Changed in both places".to_string(),
        (PackageState::Diverged, Site::QueueRow) => "changed in both places".to_string(),

        (PackageState::PullConflict { files }, Site::ListRow) if files.len() == 1 => {
            "conflict in 1 file".to_string()
        }
        (PackageState::PullConflict { files }, Site::QueueRow) if files.len() == 1 => {
            "has a conflict in 1 file".to_string()
        }
        (PackageState::PullConflict { files }, Site::ListRow) => {
            format!("conflicts in {} files", files.len())
        }
        (PackageState::PullConflict { files }, Site::QueueRow) => {
            format!("has conflicts in {} files", files.len())
        }

        // The list never names the role, and the queue can't when the role query
        // behind it failed — named or not, the denial is the same denial.
        (PackageState::RoleDenied { .. }, Site::ListRow) => "No access".to_string(),
        (PackageState::RoleDenied { role: None }, Site::QueueRow) => "cannot be read".to_string(),
        // The queue states a shared cause once, so this one names the role.
        (PackageState::RoleDenied { role: Some(role) }, Site::QueueRow) => {
            format!("cannot be read as {role}")
        }

        (PackageState::NoRemote, Site::ListRow) => "No S3 bucket yet".to_string(),
        (PackageState::NoRemote, Site::QueueRow) => "has no S3 bucket yet".to_string(),

        (PackageState::Unpublished, Site::ListRow) => "Not published yet".to_string(),
        (PackageState::Unpublished, Site::QueueRow) => "has never been published".to_string(),

        // Fixed words, never the backend's message as the label: the vocabulary
        // stays UI-owned, and the message renders as detail beside this.
        //
        // Distinct words from `Unknown` because the claims differ: there, the
        // upstream state could not be read at all; here it was read and the syncing
        // is what stopped.
        (PackageState::Paused, Site::ListRow) => "Sync paused".to_string(),
        (PackageState::Paused, Site::QueueRow) => "has stopped syncing".to_string(),

        (PackageState::Unknown, Site::ListRow) => "Sync stopped".to_string(),
        (PackageState::Unknown, Site::QueueRow) => "cannot be checked".to_string(),

        // Every remaining cause. Delegated rather than written out, because a
        // heading and a chip want the same noun phrase; the arms above still force
        // a new state to answer for both of the sites that choose their own words.
        (state, Site::Cause) => words(state, Site::ListRow),
    }
}

/// How loud the state is, wherever it draws.
fn tone(state: &PackageState) -> StateTone {
    match state {
        PackageState::Latest => StateTone::Success,

        PackageState::PendingChanges { .. } => StateTone::Neutral,

        PackageState::Behind
        | PackageState::PendingCommit
        | PackageState::NoRemote
        | PackageState::Unpublished => StateTone::Attention,

        // Danger and not Attention for the last three: `StateTone`'s own split is
        // "waiting on you" versus "wrong, and the row cannot fix it", and neither a
        // refused bucket nor a stopped sync is waiting on anybody.
        PackageState::Diverged
        | PackageState::PullConflict { .. }
        | PackageState::RoleDenied { .. }
        | PackageState::Paused
        | PackageState::Unknown => StateTone::Danger,
    }
}

/// The operation the state offers, wherever it draws.
fn action(state: &PackageState) -> Option<PackageAction> {
    match state {
        PackageState::Behind => Some(PackageAction::GetLatest),

        // `Publish` for the conflict too, and not `Resolve`: the merge page cannot
        // resolve one until the local changes are committed, so publishing is the
        // step that unblocks it. Publishing lands the package in `Diverged`, which
        // is the state below — and that one does offer `Resolve`.
        PackageState::PendingChanges { .. }
        | PackageState::PendingCommit
        | PackageState::Unpublished
        | PackageState::PullConflict { .. } => Some(PackageAction::Publish),

        PackageState::Diverged => Some(PackageAction::Resolve),

        PackageState::NoRemote => Some(PackageAction::ChooseS3Bucket),

        // Nothing on offer. A denial is fixed at the host, and there is no resume for
        // a pause — see `commands/main_page.rs`. §5's lattice gives the pause row
        // `[Dismiss]`, which names an operation the product does not have, exactly as
        // its `[Merge]` did at row 5.
        PackageState::Latest
        | PackageState::RoleDenied { .. }
        | PackageState::Paused
        | PackageState::Unknown => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    #[wasm_bindgen_test]
    fn behind_is_quiet_on_a_list_row_and_inviting_on_a_queue_row() {
        let s = PackageState::Behind;
        assert_eq!(render(&s, Site::ListRow).words, "Not the latest");
        assert_eq!(render(&s, Site::QueueRow).words, "has a newer revision");
    }

    #[wasm_bindgen_test]
    fn role_denied_states_the_cause_once_on_a_queue_row() {
        let s = PackageState::RoleDenied {
            role: Some("analyst".to_string()),
        };
        assert_eq!(render(&s, Site::ListRow).words, "No access");
        assert!(
            render(&s, Site::QueueRow).words.contains("analyst"),
            "the queue row names the role, so the cause is stated once"
        );
    }

    #[wasm_bindgen_test]
    fn a_denial_whose_role_could_not_be_named_still_has_words() {
        // The role query can fail while the denial is certain — the bucket said no.
        // The queue names the role when it can and says what the list says when it
        // cannot; it never renders "No access as " with nothing after it.
        let rendered = render(&PackageState::RoleDenied { role: None }, Site::QueueRow);
        assert_eq!(rendered.words, "cannot be read");
        assert_eq!(rendered.tone, StateTone::Danger);
        assert_eq!(rendered.action, None);
    }

    /// Every state with the clause the queue words it as.
    ///
    /// A table rather than a test each, because the property being asserted belongs
    /// to the whole vocabulary: all of it has to read after a package's name. A
    /// state that keeps its list-row noun phrase is what this catches.
    fn queue_clauses() -> Vec<(PackageState, &'static str)> {
        vec![
            (PackageState::Latest, "is up to date"),
            (PackageState::Behind, "has a newer revision"),
            (
                PackageState::PendingChanges { files: 1 },
                "has 1 changed file",
            ),
            (
                PackageState::PendingChanges { files: 2 },
                "has 2 changed files",
            ),
            (
                PackageState::PendingCommit,
                "has a revision it has not published",
            ),
            (PackageState::Diverged, "changed in both places"),
            (
                PackageState::PullConflict {
                    files: vec!["a.csv".to_string()],
                },
                "has a conflict in 1 file",
            ),
            (
                PackageState::PullConflict {
                    files: vec!["a.csv".to_string(), "b.csv".to_string()],
                },
                "has conflicts in 2 files",
            ),
            (
                PackageState::RoleDenied {
                    role: Some("analyst".to_string()),
                },
                "cannot be read as analyst",
            ),
            (PackageState::RoleDenied { role: None }, "cannot be read"),
            (PackageState::NoRemote, "has no S3 bucket yet"),
            (PackageState::Unpublished, "has never been published"),
            (PackageState::Paused, "has stopped syncing"),
            (PackageState::Unknown, "cannot be checked"),
        ]
    }

    #[wasm_bindgen_test]
    fn the_queue_words_every_state_as_a_clause_about_the_package() {
        for (state, clause) in queue_clauses() {
            assert_eq!(render(&state, Site::QueueRow).words, clause, "{state:?}");
        }
    }

    #[wasm_bindgen_test]
    fn a_queue_clause_never_opens_with_a_capital() {
        for (state, _) in queue_clauses() {
            let words = render(&state, Site::QueueRow).words;
            let opener = words.chars().next().expect("a state always says something");
            assert!(
                !opener.is_uppercase(),
                "{words:?} opens a sentence of its own, but it is drawn as the \
                 continuation of the package name before it"
            );
        }
    }

    #[wasm_bindgen_test]
    fn a_shared_cause_names_the_role_and_still_reads_as_a_heading() {
        // A cause is stated once above the packages it holds, with the host and the
        // bucket appended by the page — so it is a heading, and a clause about a
        // package would not survive being read as one.
        let named = PackageState::RoleDenied {
            role: Some("analyst".to_string()),
        };
        assert_eq!(render(&named, Site::Cause).words, "No access as analyst");
        assert_eq!(
            render(&PackageState::RoleDenied { role: None }, Site::Cause).words,
            "No access",
            "the denial stands even when the role query behind the wording failed"
        );
    }

    #[wasm_bindgen_test]
    fn a_stopped_sync_and_an_unreadable_state_still_make_different_claims() {
        // Merging these two arms is the change this catches. One says syncing
        // stopped for a package whose state was read; the other says the state
        // could not be read at all.
        assert_ne!(
            render(&PackageState::Paused, Site::QueueRow).words,
            render(&PackageState::Unknown, Site::QueueRow).words
        );
    }

    #[wasm_bindgen_test]
    fn a_count_is_interpolated_from_the_data() {
        let s = PackageState::PendingChanges { files: 2 };
        assert_eq!(render(&s, Site::ListRow).words, "2 files changed");
    }

    #[wasm_bindgen_test]
    fn pull_conflict_counts_the_paths_it_was_given() {
        let s = PackageState::PullConflict {
            files: vec!["a.csv".to_string(), "b.csv".to_string()],
        };
        assert_eq!(render(&s, Site::ListRow).words, "conflicts in 2 files");
    }

    #[wasm_bindgen_test]
    fn diverged_offers_resolve_and_never_merge() {
        let r = render(&PackageState::Diverged, Site::QueueRow);
        assert_eq!(r.action, Some(PackageAction::Resolve));
    }

    #[wasm_bindgen_test]
    fn pull_conflict_offers_publish_not_resolve() {
        let r = render(
            &PackageState::PullConflict { files: vec![] },
            Site::QueueRow,
        );
        assert_eq!(
            r.action,
            Some(PackageAction::Publish),
            "the merge page cannot resolve it until the changes are committed"
        );
    }

    #[wasm_bindgen_test]
    fn latest_is_the_only_success_tone() {
        assert_eq!(
            render(&PackageState::Latest, Site::ListRow).tone,
            StateTone::Success
        );
        assert_eq!(render(&PackageState::Latest, Site::ListRow).action, None);
    }

    #[wasm_bindgen_test]
    fn unknown_renders_fixed_words_and_never_the_backend_message() {
        let r = render(&PackageState::Unknown, Site::ListRow);
        assert_eq!(r.tone, StateTone::Danger);
        assert!(!r.words.is_empty(), "an unknown state still says something");
        assert_eq!(r.action, None, "the app has no operation that fixes this");
    }

    #[wasm_bindgen_test]
    fn a_pause_deserialises_to_itself_and_not_to_unknown() {
        // The one failure mode a missing arm here would NOT produce: an error.
        // `#[serde(other)]` catches every unnamed kind, so a state the backend
        // sends and this enum forgets renders "Sync stopped" — a claim about
        // being unable to read the upstream state, over a package whose state
        // was read fine. The backend's own wire test pins the other end.
        let parsed: PackageState = serde_json::from_str(r#"{"kind":"paused"}"#).unwrap();
        assert!(
            matches!(parsed, PackageState::Paused),
            "got {parsed:?}, which is what a missing arm looks like"
        );
    }

    #[wasm_bindgen_test]
    fn a_role_denial_deserialises_to_itself_and_not_to_unknown() {
        // The wire kind that had never been deserialised ANYWHERE: `role_denied`
        // gained a producer in plan 2 and no reader ever parsed one, and
        // `#[serde(other)]` turns a kind this build does not name into `Unknown`
        // — which renders "Sync stopped", reading a denial as an unreadable state.
        //
        // This was a manual check needing a machine with two roles, one of them
        // without access to a bucket. A serde arm does not deserve that.
        let parsed: PackageState =
            serde_json::from_str(r#"{"kind":"role_denied","role":"analyst"}"#).unwrap();
        assert!(
            matches!(&parsed, PackageState::RoleDenied { role } if role.as_deref() == Some("analyst")),
            "got {parsed:?}, which is what a missing arm looks like"
        );
        assert_eq!(render(&parsed, Site::ListRow).words, "No access");
    }

    #[wasm_bindgen_test]
    fn an_unrecognised_kind_deserialises_to_unknown_rather_than_failing() {
        let parsed: PackageState =
            serde_json::from_str(r#"{"kind":"something_added_next_year"}"#).unwrap();
        assert!(matches!(parsed, PackageState::Unknown));
    }

    #[wasm_bindgen_test]
    fn no_label_uses_a_banned_word() {
        const BANNED: &[&str] = &[
            "commit", "push", "pull", "remote", "behind", "ahead", "diverged", "dirty",
        ];
        let all = [
            PackageState::Latest,
            PackageState::Behind,
            PackageState::PendingChanges { files: 2 },
            PackageState::PendingCommit,
            PackageState::Diverged,
            PackageState::PullConflict { files: vec![] },
            PackageState::RoleDenied {
                role: Some("analyst".to_string()),
            },
            PackageState::NoRemote,
            PackageState::Unpublished,
            PackageState::Paused,
            PackageState::Unknown,
        ];
        for state in &all {
            for site in [Site::ListRow, Site::QueueRow, Site::Cause] {
                let words = render(state, site).words.to_lowercase();
                for bad in BANNED {
                    assert!(
                        !words
                            .split_whitespace()
                            .any(|w| w.trim_matches(|c: char| !c.is_alphanumeric()) == *bad),
                        "{words:?} contains the banned word {bad:?}"
                    );
                }
            }
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "a table of expected outputs; its length is data, not branching"
    )]
    #[wasm_bindgen_test]
    fn renders_complete_mapping_for_all_state_and_site_combinations() {
        // Table-driven test: verify words, tone, and action for every (state, site) pair.
        // Each row is (state, site, expected_words, expected_tone, expected_action).
        let cases = vec![
            (
                PackageState::Latest,
                Site::ListRow,
                "Latest",
                StateTone::Success,
                None,
            ),
            (
                PackageState::Latest,
                Site::QueueRow,
                "is up to date",
                StateTone::Success,
                None,
            ),
            (
                PackageState::Behind,
                Site::ListRow,
                "Not the latest",
                StateTone::Attention,
                Some(PackageAction::GetLatest),
            ),
            (
                PackageState::Behind,
                Site::QueueRow,
                "has a newer revision",
                StateTone::Attention,
                Some(PackageAction::GetLatest),
            ),
            (
                PackageState::PendingChanges { files: 1 },
                Site::ListRow,
                "1 file changed",
                StateTone::Neutral,
                Some(PackageAction::Publish),
            ),
            (
                PackageState::PendingChanges { files: 2 },
                Site::ListRow,
                "2 files changed",
                StateTone::Neutral,
                Some(PackageAction::Publish),
            ),
            (
                PackageState::PendingCommit,
                Site::ListRow,
                "Revision not published",
                StateTone::Attention,
                Some(PackageAction::Publish),
            ),
            (
                PackageState::Diverged,
                Site::ListRow,
                "Changed in both places",
                StateTone::Danger,
                Some(PackageAction::Resolve),
            ),
            (
                PackageState::PullConflict {
                    files: vec!["a.csv".to_string()],
                },
                Site::ListRow,
                "conflict in 1 file",
                StateTone::Danger,
                Some(PackageAction::Publish),
            ),
            (
                PackageState::PullConflict {
                    files: vec!["a.csv".to_string(), "b.csv".to_string()],
                },
                Site::ListRow,
                "conflicts in 2 files",
                StateTone::Danger,
                Some(PackageAction::Publish),
            ),
            (
                PackageState::RoleDenied {
                    role: Some("analyst".to_string()),
                },
                Site::ListRow,
                "No access",
                StateTone::Danger,
                None,
            ),
            (
                PackageState::RoleDenied {
                    role: Some("analyst".to_string()),
                },
                Site::QueueRow,
                "cannot be read as analyst",
                StateTone::Danger,
                None,
            ),
            (
                PackageState::RoleDenied { role: None },
                Site::ListRow,
                "No access",
                StateTone::Danger,
                None,
            ),
            (
                PackageState::RoleDenied { role: None },
                Site::QueueRow,
                "cannot be read",
                StateTone::Danger,
                None,
            ),
            (
                PackageState::NoRemote,
                Site::ListRow,
                "No S3 bucket yet",
                StateTone::Attention,
                Some(PackageAction::ChooseS3Bucket),
            ),
            (
                PackageState::Unpublished,
                Site::ListRow,
                "Not published yet",
                StateTone::Attention,
                Some(PackageAction::Publish),
            ),
            (
                PackageState::Unknown,
                Site::ListRow,
                "Sync stopped",
                StateTone::Danger,
                None,
            ),
        ];

        for (state, site, expected_words, expected_tone, expected_action) in cases {
            let rendered = render(&state, site);
            assert_eq!(
                rendered.words, expected_words,
                "mismatch for {:?} at {:?}: got {}, expected {}",
                state, site, rendered.words, expected_words
            );
            assert_eq!(
                rendered.tone, expected_tone,
                "tone mismatch for {:?} at {:?}: got {:?}, expected {:?}",
                state, site, rendered.tone, expected_tone
            );
            assert_eq!(
                rendered.action, expected_action,
                "action mismatch for {:?} at {:?}: got {:?}, expected {:?}",
                state, site, rendered.action, expected_action
            );
        }
    }
}
