//! The context pane's revision mode: the current revision whole, with its
//! hash, publication, workflow and full metadata. The page swaps it in for the
//! ordinary pane while the address carries `revision=<hash>`.

use leptos::prelude::*;
use quilt_uri::Namespace;

use super::Wiring;
use super::context_pane::RevisionHistoryFetch;
use crate::commands;
use crate::kit::{
    BackLink, BannerVariant, Button, Card, JsonDisplay, LoadFailure, PaneSection, RevisionRow,
    SkeletonText,
};

use super::Outcome;

stylance::import_crate_style!(
    style,
    "src/pages/installed_package_v2/revision_pane.module.css"
);

/// Whether the registry lists the revision, as the *History* read says.
#[derive(Clone, Debug, PartialEq)]
enum Publication {
    Loading,
    Published { catalog_url: Option<String> },
    NotPublished,
    Failed,
}

impl Publication {
    /// The current revision's row of the *History* read. A row missing from
    /// the list reads as not published.
    fn of(hash: &str, answer: Result<commands::RevisionHistoryData, String>) -> Self {
        let Ok(history) = answer else {
            return Self::Failed;
        };
        match history.rows.into_iter().find(|row| row.hash == hash) {
            Some(row) if row.published => Self::Published {
                catalog_url: row.catalog_url,
            },
            _ => Self::NotPublished,
        }
    }
}

const COPY_FAILED: &str = "Could not copy the revision's hash.";

/// The pane's revision mode. It swaps the ordinary pane whole.
#[component]
pub fn RevisionPane(
    namespace: Namespace,
    revision: commands::CurrentRevisionData,
    /// The plain package address on the page.
    #[prop(into)]
    back_href: String,
    /// The *History* popover's read, for publication.
    fetch: RevisionHistoryFetch,
    open_catalog: Callback<String>,
    w: Wiring,
) -> impl IntoView {
    let meta = revision.metadata();
    let commands::CurrentRevisionData {
        hash,
        message,
        obtained_at,
        workflow,
        ..
    } = revision;

    let publication = RwSignal::new(Publication::Loading);
    let load = {
        let (ns, hash) = (namespace.to_string(), hash.clone());
        move || {
            let (ns, hash) = (ns.clone(), hash.clone());
            publication.set(Publication::Loading);
            // `try_set`: a re-read disposes this pane, and a late answer then lands nowhere.
            leptos::task::spawn_local(async move {
                publication.try_set(Publication::of(&hash, fetch(ns).await));
            });
        }
    };
    load();
    let retry = Callback::new(move |()| load());

    let copied = RwSignal::new(false);
    let on_copy = {
        let (ns, hash) = (namespace.to_string(), hash.clone());
        move |_| {
            let (ns, hash) = (ns.clone(), hash.clone());
            leptos::task::spawn_local(async move {
                match commands::copy_to_clipboard(hash, None).await {
                    Ok(_) => {
                        copied.try_set(true);
                    }
                    Err(detail) => {
                        w.outcome.try_set(Some(Outcome {
                            namespace: ns,
                            variant: BannerVariant::Critical,
                            lead: COPY_FAILED.to_string(),
                            detail: Some(detail),
                        }));
                    }
                }
            });
        }
    };

    let metadata = match meta {
        Some(value) => view! { <JsonDisplay value=value /> }.into_any(),
        None => view! { <span class=style::muted>"No metadata"</span> }.into_any(),
    };

    let (pane, opening) = super::resolve::reveal_on_opening("revision");
    let class = if opening {
        format!("{} {}", style::root, style::opening)
    } else {
        style::root.to_string()
    };
    view! {
        <aside aria-label="About this package" class=class node_ref=pane>
            <Card label="About this package">
                // Leaving replaces the mode's entry, so Back skips it.
                <BackLink href=back_href label=namespace.to_string() replace=true />
                <PaneSection label="Revision">
                    <RevisionRow message=message.unwrap_or_default() at=obtained_at />
                    <dl class=style::facts>
                        <dt class=style::muted>"Hash"</dt>
                        <dd>
                            <div class=style::line>
                                <code class=style::hash>{hash}</code>
                                <Button on_click=on_copy>
                                    {move || if copied.get() { "Copied" } else { "Copy" }}
                                </Button>
                            </div>
                        </dd>
                        <dt class=style::muted>"Published"</dt>
                        {published_line(publication, open_catalog, retry)}
                        {workflow
                            .map(|workflow| {
                                view! {
                                    <dt class=style::muted>"Workflow"</dt>
                                    <dd>{workflow}</dd>
                                }
                            })}
                    </dl>
                </PaneSection>
                <PaneSection label="Metadata">{metadata}</PaneSection>
            </Card>
        </aside>
    }
}

/// The *Published* value: a live region, so the answer and a retry's are
/// announced, as the History popover's are.
fn published_line(
    publication: RwSignal<Publication>,
    open_catalog: Callback<String>,
    retry: Callback<()>,
) -> impl IntoView {
    let published = move || match publication.get() {
        Publication::Loading => view! { <SkeletonText width="40%" /> }.into_any(),
        Publication::Published { catalog_url } => view! {
            <div class=style::line>
                <span>"Published"</span>
                {catalog_url
                    .map(|url| {
                        view! {
                            <Button on_click=move |_| open_catalog.run(url.clone())>
                                "Open in catalog"
                            </Button>
                        }
                    })}
            </div>
        }
        .into_any(),
        Publication::NotPublished => view! { <span>"Not published"</span> }.into_any(),
        Publication::Failed => view! {
            <LoadFailure words="Could not check whether it is published." on_retry=retry />
        }
        .into_any(),
    };
    view! {
        <dd
            aria-live="polite"
            aria-busy=move || {
                if publication.with(|p| *p == Publication::Loading) { "true" } else { "false" }
            }
        >
            {published}
        </dd>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{CurrentRevisionData, RevisionHistoryData, RevisionHistoryRow};
    use crate::test_support::mount;
    use std::future::Future;
    use std::pin::Pin;
    use wasm_bindgen_test::*;

    type Answer = Pin<Box<dyn Future<Output = Result<RevisionHistoryData, String>>>>;

    const HASH: &str = "abc123";
    const URL: &str = "https://quilt.test/b/test/packages/team/dataset/tree/abc123";

    fn row(published: bool) -> RevisionHistoryRow {
        RevisionHistoryRow {
            hash: HASH.to_string(),
            message: None,
            obtained_at: 0.0,
            published,
            catalog_url: published.then(|| URL.to_string()),
            kept: Vec::new(),
            frees: None,
        }
    }

    fn history(rows: Vec<RevisionHistoryRow>) -> RevisionHistoryData {
        RevisionHistoryData {
            rows,
            removable_frees: None,
        }
    }

    #[test]
    fn publication_reads_the_current_row() {
        assert_eq!(
            Publication::of(HASH, Ok(history(vec![row(true)]))),
            Publication::Published {
                catalog_url: Some(URL.to_string())
            }
        );
        assert_eq!(
            Publication::of(HASH, Ok(history(vec![row(false)]))),
            Publication::NotPublished
        );
        assert_eq!(
            Publication::of(HASH, Ok(history(vec![]))),
            Publication::NotPublished
        );
        assert_eq!(
            Publication::of(HASH, Err("offline".to_string())),
            Publication::Failed
        );
    }

    fn published(_: String) -> Answer {
        Box::pin(async { Ok(history(vec![row(true)])) })
    }

    fn refuses(_: String) -> Answer {
        Box::pin(async { Err("offline".to_string()) })
    }

    fn pane(
        meta: Option<serde_json::Value>,
        workflow: Option<&str>,
        fetch: RevisionHistoryFetch,
    ) -> web_sys::Element {
        let revision = CurrentRevisionData {
            hash: HASH.to_string(),
            message: Some("Initial upload".to_string()),
            obtained_at: 1_758_500_000_000.0,
            meta: meta.map(|meta| meta.to_string()),
            workflow: workflow.map(str::to_string),
        };
        mount(move || {
            view! {
                <RevisionPane
                        namespace=("team", "dataset").into()
                        revision=revision.clone()
                        back_href="/installed-package?namespace=team%2Fdataset"
                        fetch=fetch
                        open_catalog=Callback::new(|_: String| {})
                        w=Wiring::new()
                    />
            }
        })
    }

    async fn settle() {
        crate::test_support::sleep_ms(0).await;
    }

    #[wasm_bindgen_test]
    async fn the_mode_shows_the_whole_revision() {
        let el = pane(
            Some(serde_json::json!({"plate": 7})),
            Some("alpha"),
            published,
        );
        settle().await;
        let text = el.text_content().unwrap_or_default();
        for said in [
            "team/dataset",
            "Initial upload",
            HASH,
            "Copy",
            "Published",
            "Open in catalog",
            "Workflow",
            "alpha",
            "plate",
        ] {
            assert!(text.contains(said), "missing {said:?} in {text:?}");
        }
        let back = el.query_selector("a").unwrap().unwrap();
        assert_eq!(
            back.text_content().unwrap_or_default().trim(),
            "team/dataset"
        );
    }

    #[wasm_bindgen_test]
    async fn no_metadata_and_no_workflow_say_so() {
        let el = pane(None, None, refuses);
        settle().await;
        let text = el.text_content().unwrap_or_default();
        assert!(text.contains("No metadata"), "{text}");
        assert!(!text.contains("Workflow"), "{text}");
        assert!(
            text.contains("Could not check whether it is published."),
            "{text}"
        );
        let live = el
            .query_selector("[aria-live='polite']")
            .unwrap()
            .expect("a live region");
        assert_eq!(live.get_attribute("aria-busy").as_deref(), Some("false"));
    }
}
