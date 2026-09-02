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
//!
//! Every `#[allow(dead_code)]` below stays: the gallery binary's `mod kit;`
//! never wires up [`render`], so this module is genuinely unused there, and
//! `#[expect(dead_code)]` fails on the app binary, where it IS used —
//! confirmed against the compiler, not carried over from habit.

use serde::Deserialize;

use super::StateTone;

/// A package's resolved state, as it crosses the wire.
///
/// Internally tagged, so the JSON is `{"kind": "diverged"}`. `Unknown` is
/// `#[serde(other)]`, which is what stops a `kind` this build has never heard of
/// from failing the whole payload — and it carries no data because
/// `#[serde(other)]` accepts only unit variants, and does not need to: the message
/// for an unexplained pause travels on the row's `paused_reason`, not in here.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PackageState {
    Latest,
    Behind,
    PendingChanges { files: usize },
    PendingCommit,
    Diverged,
    PullConflict { files: Vec<String> },
    /// `None` when the denial is certain but the role query behind the wording
    /// failed. The denial still stands — the bucket refused — so suppressing the
    /// state would lose a real fact; it simply cannot be named.
    RoleDenied { role: Option<String> },
    NoRemote,
    Unpublished,
    #[serde(other)]
    Unknown,
}

/// Where a label is being drawn. Not decoration: two states word themselves
/// differently here, so a mapping keyed on state alone is wrong.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Site {
    /// A row in the package list, which is quiet and pairs against `Latest`.
    ListRow,
    /// A row in the attention queue, which sits beside the action it offers.
    QueueRow,
}

/// What to draw for one state at one site.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rendered {
    pub words: String,
    pub tone: StateTone,
    /// The action's label, or `None` when the app has no operation that fixes it.
    /// A `&'static str` because the verbs are the vocabulary's, not the caller's.
    pub action: Option<&'static str>,
}

/// The vocabulary, in one place.
///
/// Counts are interpolated from the data here rather than sent as their own number
/// (§1): `PullConflict` counts the paths it was given, so the label cannot disagree
/// with the list it describes.
#[allow(dead_code)]
#[must_use]
pub fn render(state: &PackageState, site: Site) -> Rendered {
    let (words, tone, action) = match (state, site) {
        (PackageState::Latest, _) => ("Latest".to_string(), StateTone::Success, None),

        (PackageState::Behind, Site::ListRow) => {
            ("Not the latest".to_string(), StateTone::Attention, Some("Get latest"))
        }
        (PackageState::Behind, Site::QueueRow) => (
            "Newer revision available".to_string(),
            StateTone::Attention,
            Some("Get latest"),
        ),

        (PackageState::PendingChanges { files }, _) => (
            format!("{files} files changed"),
            StateTone::Neutral,
            Some("Publish"),
        ),

        (PackageState::PendingCommit, _) => (
            "Revision not published".to_string(),
            StateTone::Attention,
            Some("Publish"),
        ),

        // `Resolve`, never `Merge`: no merge operation exists — resolving is a
        // package-level choice between Certify Latest and Reset Local.
        (PackageState::Diverged, _) => (
            "Changed in both places".to_string(),
            StateTone::Danger,
            Some("Resolve"),
        ),

        // `Publish`, not `Resolve`: the merge page cannot resolve a conflict until
        // the local changes are committed, so publishing is the step that unblocks.
        (PackageState::PullConflict { files }, _) => (
            format!("conflicts in {} files", files.len()),
            StateTone::Danger,
            Some("Publish"),
        ),

        // The list never names the role, and the queue can't when the role query
        // behind it failed — named or not, the denial is the same denial.
        (PackageState::RoleDenied { .. }, Site::ListRow)
        | (PackageState::RoleDenied { role: None }, Site::QueueRow) => {
            ("No access".to_string(), StateTone::Danger, None)
        }
        // The queue states a shared cause once, so this one names the role.
        (PackageState::RoleDenied { role: Some(role) }, Site::QueueRow) => {
            (format!("No access as {role}"), StateTone::Danger, None)
        }

        (PackageState::NoRemote, _) => (
            "No S3 bucket yet".to_string(),
            StateTone::Attention,
            Some("Choose S3 bucket"),
        ),

        (PackageState::Unpublished, _) => (
            "Not published yet".to_string(),
            StateTone::Attention,
            Some("Publish"),
        ),

        // Fixed words, never the backend's message as the label: the vocabulary
        // stays UI-owned, and the message renders as detail beside this. No action
        // — the fix is a workflow rule or a misconfiguration, not an operation the
        // app exposes.
        (PackageState::Unknown, _) => (
            "Sync stopped".to_string(),
            StateTone::Danger,
            None,
        ),
    };

    Rendered { words, tone, action }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    #[wasm_bindgen_test]
    fn behind_is_quiet_on_a_list_row_and_inviting_on_a_queue_row() {
        let s = PackageState::Behind;
        assert_eq!(render(&s, Site::ListRow).words, "Not the latest");
        assert_eq!(render(&s, Site::QueueRow).words, "Newer revision available");
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
        assert_eq!(rendered.words, "No access");
        assert_eq!(rendered.tone, StateTone::Danger);
        assert_eq!(rendered.action, None);
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
        assert_eq!(r.action, Some("Resolve"));
    }

    #[wasm_bindgen_test]
    fn pull_conflict_offers_publish_not_resolve() {
        let r = render(
            &PackageState::PullConflict { files: vec![] },
            Site::QueueRow,
        );
        assert_eq!(
            r.action,
            Some("Publish"),
            "the merge page cannot resolve it until the changes are committed"
        );
    }

    #[wasm_bindgen_test]
    fn latest_is_the_only_success_tone() {
        assert_eq!(render(&PackageState::Latest, Site::ListRow).tone, StateTone::Success);
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
            PackageState::RoleDenied { role: Some("analyst".to_string()) },
            PackageState::NoRemote,
            PackageState::Unpublished,
            PackageState::Unknown,
        ];
        for state in &all {
            for site in [Site::ListRow, Site::QueueRow] {
                let words = render(state, site).words.to_lowercase();
                for bad in BANNED {
                    assert!(
                        !words.split_whitespace().any(|w| w.trim_matches(|c: char| !c.is_alphanumeric()) == *bad),
                        "{words:?} contains the banned word {bad:?}"
                    );
                }
            }
        }
    }

    #[wasm_bindgen_test]
    fn renders_complete_mapping_for_all_state_and_site_combinations() {
        // Table-driven test: verify words, tone, and action for every (state, site) pair.
        // Each row is (state, site, expected_words, expected_tone, expected_action).
        let cases = vec![
            (PackageState::Latest, Site::ListRow, "Latest", StateTone::Success, None),
            (PackageState::Latest, Site::QueueRow, "Latest", StateTone::Success, None),
            (PackageState::Behind, Site::ListRow, "Not the latest", StateTone::Attention, Some("Get latest")),
            (PackageState::Behind, Site::QueueRow, "Newer revision available", StateTone::Attention, Some("Get latest")),
            (PackageState::PendingChanges { files: 2 }, Site::ListRow, "2 files changed", StateTone::Neutral, Some("Publish")),
            (PackageState::PendingCommit, Site::ListRow, "Revision not published", StateTone::Attention, Some("Publish")),
            (PackageState::Diverged, Site::ListRow, "Changed in both places", StateTone::Danger, Some("Resolve")),
            (PackageState::PullConflict { files: vec!["a.csv".to_string(), "b.csv".to_string()] }, Site::ListRow, "conflicts in 2 files", StateTone::Danger, Some("Publish")),
            (PackageState::RoleDenied { role: Some("analyst".to_string()) }, Site::ListRow, "No access", StateTone::Danger, None),
            (PackageState::RoleDenied { role: Some("analyst".to_string()) }, Site::QueueRow, "No access as analyst", StateTone::Danger, None),
            (PackageState::RoleDenied { role: None }, Site::QueueRow, "No access", StateTone::Danger, None),
            (PackageState::NoRemote, Site::ListRow, "No S3 bucket yet", StateTone::Attention, Some("Choose S3 bucket")),
            (PackageState::Unpublished, Site::ListRow, "Not published yet", StateTone::Attention, Some("Publish")),
            (PackageState::Unknown, Site::ListRow, "Sync stopped", StateTone::Danger, None),
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
