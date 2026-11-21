//! What the dataspace engine asks a person to decide, on the main page:
//! each row of the dataspace's `conflict.json`, kept, taken or deleted, and
//! each local commit no lock names, which can be reattached. Both lists are
//! empty while the engine is off, and the cards are then not drawn.

use leptos::prelude::*;
use quilt_ui_kit::Button;
use quilt_ui_kit::ButtonVariant;
use quilt_ui_kit::Card;

use crate::commands;
use crate::commands::ConflictKind;
use crate::commands::ConflictRow;
use crate::commands::Decision;
use crate::commands::UnattachedCommit;

/// The row's line: the package, the key, and what found it.
pub(crate) fn conflict_words(row: &ConflictRow) -> String {
    let found = match row.kind {
        ConflictKind::Read => "a newer revision changed it too",
        ConflictKind::Publish => "the destination changed it too",
    };
    let decided = match row.decision {
        None => "",
        Some(Decision::Mine) => " — keeping yours",
        Some(Decision::Theirs) => " — taking theirs",
        Some(Decision::Delete) => " — deleting it",
    };
    format!("{} · {}: {found}{decided}", row.namespace, row.key)
}

/// The commit's line: its package, and its message or else its hash.
pub(crate) fn commit_words(commit: &UnattachedCommit) -> String {
    let named = commit
        .message
        .clone()
        .unwrap_or_else(|| commit.hash.chars().take(12).collect());
    format!("{} · {named}", commit.namespace)
}

/// The two cards, read again after each decision and on `reload`.
#[component]
pub fn Decisions(reload: Trigger) -> impl IntoView {
    let decided = Trigger::new();
    let conflicts = LocalResource::new(move || {
        reload.track();
        decided.track();
        commands::get_conflicts()
    });
    let commits = LocalResource::new(move || {
        reload.track();
        decided.track();
        commands::get_unattached_commits()
    });
    let failure = RwSignal::new(None::<String>);
    let decide = move |row: ConflictRow, decision: Decision| {
        leptos::task::spawn_local(async move {
            let result = commands::decide_conflict(row.package, row.key, decision).await;
            failure.try_set(result.err());
            decided.notify();
        });
    };
    let reattach = move |commit: UnattachedCommit| {
        leptos::task::spawn_local(async move {
            let result = commands::reattach_commit(commit.namespace.to_string(), commit.hash).await;
            failure.try_set(result.err());
            decided.notify();
        });
    };
    view! {
        {move || failure.get().map(|detail| view! { <p role="alert">{detail}</p> })}
        {move || {
            let rows = conflicts.get().and_then(Result::ok).unwrap_or_default();
            (!rows.is_empty())
                .then(|| {
                    let count = rows.len();
                    view! {
                        <Card title="Conflicts" count=count>
                            {rows
                                .into_iter()
                                .map(|row| {
                                    let words = conflict_words(&row);
                                    let (mine, theirs, delete) = (row.clone(), row.clone(), row);
                                    view! {
                                        <div>
                                            <span>{words}</span>
                                            " "
                                            <Button on_click=move |_| decide(mine.clone(), Decision::Mine)>
                                                "Keep mine"
                                            </Button>
                                            " "
                                            <Button on_click=move |_| decide(theirs.clone(), Decision::Theirs)>
                                                "Take theirs"
                                            </Button>
                                            " "
                                            <Button
                                                variant=ButtonVariant::Danger
                                                on_click=move |_| decide(delete.clone(), Decision::Delete)
                                            >
                                                "Delete"
                                            </Button>
                                        </div>
                                    }
                                })
                                .collect_view()}
                        </Card>
                    }
                })
        }}
        {move || {
            let listed = commits.get().and_then(Result::ok).unwrap_or_default();
            (!listed.is_empty())
                .then(|| {
                    let count = listed.len();
                    view! {
                        <Card title="Commits not attached" count=count>
                            {listed
                                .into_iter()
                                .map(|commit| {
                                    let words = commit_words(&commit);
                                    view! {
                                        <div>
                                            <span>{words}</span>
                                            " "
                                            <Button on_click=move |_| reattach(commit.clone())>
                                                "Reattach"
                                            </Button>
                                        </div>
                                    }
                                })
                                .collect_view()}
                        </Card>
                    }
                })
        }}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    fn row(decision: Option<Decision>) -> ConflictRow {
        ConflictRow {
            package: "quilt+s3://b#package=team/raw".to_string(),
            namespace: ("team", "raw").into(),
            key: "a.txt".to_string(),
            kind: ConflictKind::Read,
            mine: true,
            theirs: true,
            decision,
        }
    }

    #[wasm_bindgen_test]
    fn a_conflict_says_what_found_it_and_what_was_decided() {
        assert_eq!(
            conflict_words(&row(None)),
            "team/raw · a.txt: a newer revision changed it too"
        );
        assert_eq!(
            conflict_words(&row(Some(Decision::Theirs))),
            "team/raw · a.txt: a newer revision changed it too — taking theirs"
        );
    }

    #[wasm_bindgen_test]
    fn a_commit_is_named_by_its_message_else_its_hash() {
        let commit = UnattachedCommit {
            namespace: ("team", "raw").into(),
            hash: "0123456789abcdef".to_string(),
            message: None,
        };
        assert_eq!(commit_words(&commit), "team/raw · 0123456789ab");
    }
}
