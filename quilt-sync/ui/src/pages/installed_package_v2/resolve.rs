//! The context pane's Resolve mode: this copy's revision against the published
//! one, and the two whole-revision choices between them.
//!
//! The words are pure so their rules are host-tested: the sentence counts the
//! page's differing set and points at the marks only when there are some, and
//! the replace confirmation names each extent the reset reaches that is not
//! zero, or, with none, what the reset still does (`#resolve-flow`).

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use leptos::ev::MouseEvent;
use leptos::prelude::*;
use quilt_uri::{Namespace, S3PackageUri};

use super::{Outcome, Wiring, holding, run};
use crate::commands;
use crate::kit::{
    BackLink, BannerVariant, Button, ButtonVariant, Card, ConfirmDialog, DIFFERS_ID, LoadFailure,
    PaneSection, RevisionRow, Submit,
};
use crate::routes;

stylance::import_crate_style!(style, "src/pages/installed_package_v2/resolve.module.scss");

/// The pane's one sentence, counting the page's differing set.
pub(super) fn differs_sentence(differing: usize) -> String {
    if differing == 0 {
        // Nothing is marked, so the sentence points at nothing (R12).
        return "No files differ between these revisions.".to_string();
    }
    let files = plural(differing, "file differs", "files differ");
    format!("{differing} {files} between these revisions — marked in the list.")
}

fn plural<'a>(n: usize, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 { one } else { many }
}

/// The confirmation's one sentence: each extent that is not zero, in the
/// order the reset reaches them, then the warning (`#resolve-flow`).
pub(super) fn consequence(unpublished: usize, differing: usize, uncommitted: usize) -> String {
    let mut clauses = Vec::new();
    if unpublished > 0 {
        clauses.push(format!(
            "discards {unpublished} unpublished {}",
            plural(unpublished, "revision", "revisions")
        ));
    }
    if differing > 0 {
        clauses.push(format!(
            "replaces {differing} {} from the published revision",
            plural(differing, "file that differs", "files that differ")
        ));
    }
    if uncommitted > 0 {
        clauses.push(format!(
            "overwrites uncommitted edits to {uncommitted} {}",
            plural(uncommitted, "file", "files")
        ));
    }
    let said = match clauses.as_slice() {
        [] => "replaces your revision with the published one".to_string(),
        [one] => one.clone(),
        [first, second] => format!("{first} and {second}"),
        [init @ .., last] => format!("{}, and {last}", init.join(", ")),
    };
    format!("{}. This cannot be undone.", capitalised(&said))
}

/// Upper-cases the first letter only; every clause starts with ASCII.
fn capitalised(words: &str) -> String {
    let mut chars = words.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

type Answer<T> = Pin<Box<dyn Future<Output = Result<T, String>>>>;

/// One whole-revision choice: `(namespace, uri)`, as `certify_latest` and
/// `reset_local` take them; `uri` is for their telemetry.
pub type RevisionChoice = fn(String, Option<S3PackageUri>) -> Answer<String>;

/// Resolve's two commands. Function pointers, so the DOM tests and the
/// gallery answer without a Tauri host.
#[derive(Clone, Copy)]
pub struct ResolveCommands {
    pub certify: RevisionChoice,
    pub reset: RevisionChoice,
}

/// The pane's other mode: this copy's revision beside the published one, and
/// the two choices between them. It swaps the ordinary pane whole.
#[component]
pub fn ResolvePane(
    /// The header's, so the address it leaves for is built, not parsed.
    namespace: Namespace,
    /// For the commands' telemetry.
    uri: Option<S3PackageUri>,
    /// This copy's revision: the ordinary pane's facts, drawn as *Yours*.
    revision: commands::CurrentRevisionData,
    resolve: commands::ResolveData,
    /// The page's one differing set. The sentence counts this, not `resolve`,
    /// so the count and the file pane's marks cannot disagree.
    #[prop(into)]
    marks: Signal<Option<Arc<BTreeSet<String>>>>,
    /// The plain package address on the page; the cell's own anchor in the gallery.
    #[prop(into)]
    back_href: String,
    w: Wiring,
    commands: ResolveCommands,
) -> impl IntoView {
    let count = move || marks.with(|m| m.as_ref().map_or(0, |m| m.len()));
    let (published, extents) = match resolve {
        commands::ResolveData::Compared {
            published_message,
            unpublished,
            uncommitted,
            ..
        } => (
            view! {
                <p id=DIFFERS_ID>{move || differs_sentence(count())}</p>
                <PaneSection nested=true label="Yours">
                    <RevisionRow
                        message=revision.message.clone().unwrap_or_default()
                        at=revision.obtained_at
                    />
                </PaneSection>
                <PaneSection nested=true label="Published">
                    // The kit draws `No message` for an empty one; no time, since
                    // this copy has not obtained it.
                    <RevisionRow message=published_message.unwrap_or_default() />
                </PaneSection>
            }
            .into_any(),
            Some((unpublished, uncommitted)),
        ),
        commands::ResolveData::Refused { reason } => (
            view! {
                <PaneSection nested=true label="Yours">
                    <RevisionRow
                        message=revision.message.clone().unwrap_or_default()
                        at=revision.obtained_at
                    />
                </PaneSection>
                <LoadFailure
                    words="Could not compare the revisions."
                    on_retry=Callback::new(move |()| w.reload.notify())
                />
                <p>{reason}</p>
            }
            .into_any(),
            None,
        ),
    };
    // A refused comparison cannot say what either choice costs, so neither is offered.
    let compared = extents.is_some();
    let sealed = Signal::derive(move || !compared || w.busy.get());

    let target = Target {
        namespace: namespace.to_string(),
        uri,
        plain: routes::package_page_href(&namespace),
    };
    let on_certify = certify_press(target.clone(), w, commands.certify);
    let replace = w.dialogs.replace;
    let confirmation = extents.map(|(unpublished, uncommitted)| {
        // Read once: the dialog's sentence is fixed while it is drawn, and
        // the set does not change while the mode is open on one payload.
        let differing = marks.with_untracked(|m| m.as_ref().map_or(0, |m| m.len()));
        replace_confirmation(
            target,
            w,
            commands.reset,
            consequence(unpublished, differing, uncommitted),
        )
    });

    view! {
        <aside aria-label="About this package" class=style::root>
            <Card label="About this package">
                <BackLink href=back_href label=namespace.to_string() />
                <PaneSection>
                    {published}
                    <div class=style::choices>
                        <Button
                            variant=ButtonVariant::Primary
                            wrap=true
                            disabled=sealed
                            on_click=on_certify
                        >
                            "Make mine the shared one"
                        </Button>
                        <Button wrap=true disabled=sealed on_click=move |_| replace.set(true)>
                            "Replace mine with the published one"
                        </Button>
                    </div>
                </PaneSection>
            </Card>
            {confirmation}
        </aside>
    }
}

/// What a choice acts on, and the plain address a success leaves for.
#[derive(Clone)]
struct Target {
    namespace: String,
    uri: Option<S3PackageUri>,
    plain: String,
}

/// *Make mine the shared one*: runs at once, since nothing is lost that the
/// registry does not keep (`#replace-confirmation`).
fn certify_press(target: Target, w: Wiring, certify: RevisionChoice) -> impl Fn(MouseEvent) {
    let Wiring {
        busy,
        outcome,
        reload,
        replace_to,
        ..
    } = w;
    move |_| {
        let Target {
            namespace,
            uri,
            plain,
        } = target.clone();
        let ns = namespace.clone();
        let task = async move {
            let answer = certify(ns.clone(), uri).await;
            if answer.is_ok() {
                outcome.try_set(Some(success(ns, "Your revision is now the shared one.")));
                replace_to.try_set(Some(plain));
            }
            answer
        };
        run(
            busy,
            outcome,
            namespace,
            "Could not make your revision the shared one.",
            Some(reload),
            task,
        );
    }
}

/// *Replace mine with the published one*'s confirmation, on the page's flag
/// so it outlives a re-read, as undo's does.
fn replace_confirmation(
    target: Target,
    w: Wiring,
    reset: RevisionChoice,
    consequence: String,
) -> impl IntoView {
    let Wiring {
        busy,
        outcome,
        reload,
        replace_to,
        dialogs,
        ..
    } = w;
    view! {
        <ConfirmDialog
            open=dialogs.replace
            title="Replace yours with the published revision"
            consequence=consequence
            confirm=Submit::new("Replace mine", move || {
                let Target { namespace, uri, plain } = target.clone();
                async move {
                    // A refusal answers the dialog, which draws it and stays open.
                    holding(busy, outcome, reset(namespace.clone(), uri)).await?;
                    outcome.set(Some(success(namespace, "Replaced with the published revision.")));
                    replace_to.set(Some(plain));
                    reload.notify();
                    Ok(())
                }
            })
            running=busy
        />
    }
}

/// A choice's one line on the band, keyed to its package.
fn success(namespace: String, lead: &str) -> Outcome {
    Outcome {
        namespace,
        variant: BannerVariant::Success,
        lead: lead.to_string(),
        detail: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Answer, ResolveCommands, ResolvePane, consequence, differs_sentence};
    use crate::commands::{CurrentRevisionData, ResolveData};
    use crate::kit::{BannerVariant, DIFFERS_ID};
    use crate::pages::installed_package_v2::{Outcome, Wiring};
    use crate::test_support::{element_saying, mount, sleep_ms};
    use leptos::prelude::*;
    use quilt_uri::{Namespace, S3PackageUri};
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeSet;
    use std::sync::Arc;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    #[test]
    fn the_sentence_counts_the_files_that_differ() {
        assert_eq!(
            differs_sentence(2),
            "2 files differ between these revisions — marked in the list."
        );
        assert_eq!(
            differs_sentence(1),
            "1 file differs between these revisions — marked in the list."
        );
    }

    #[test]
    fn with_nothing_marked_the_sentence_points_at_nothing() {
        let words = differs_sentence(0);
        assert_eq!(words, "No files differ between these revisions.");
        assert!(
            !words.contains("marked"),
            "{words:?} should not mention marks"
        );
    }

    #[test]
    fn every_extent_is_named() {
        assert_eq!(
            consequence(2, 3, 1),
            "Discards 2 unpublished revisions, replaces 3 files that differ from the published \
             revision, and overwrites uncommitted edits to 1 file. This cannot be undone."
        );
    }

    #[test]
    fn singulars_are_singular() {
        assert_eq!(
            consequence(1, 1, 1),
            "Discards 1 unpublished revision, replaces 1 file that differs from the published \
             revision, and overwrites uncommitted edits to 1 file. This cannot be undone."
        );
    }

    #[test]
    fn a_zero_extent_is_left_out() {
        assert_eq!(
            consequence(0, 3, 1),
            "Replaces 3 files that differ from the published revision and overwrites \
             uncommitted edits to 1 file. This cannot be undone."
        );
        assert_eq!(
            consequence(2, 0, 0),
            "Discards 2 unpublished revisions. This cannot be undone."
        );
        assert_eq!(
            consequence(0, 0, 4),
            "Overwrites uncommitted edits to 4 files. This cannot be undone."
        );
    }

    #[test]
    fn with_nothing_to_lose_it_still_says_what_happens() {
        assert_eq!(
            consequence(0, 0, 0),
            "Replaces your revision with the published one. This cannot be undone."
        );
    }

    const BACK: &str = "/installed-package?namespace=team%2Fdataset&filter=unmodified";

    fn ns() -> Namespace {
        Namespace::from(("team", "dataset"))
    }

    fn revision() -> CurrentRevisionData {
        CurrentRevisionData {
            message: Some("Mine".into()),
            obtained_at: 1_758_500_000_000.0,
        }
    }

    fn compared(unpublished: usize, uncommitted: usize) -> ResolveData {
        ResolveData::Compared {
            published_message: Some("Theirs".into()),
            differing: vec!["plate/a.csv".into(), "plate/b.csv".into()],
            unpublished,
            uncommitted,
        }
    }

    fn refused() -> ResolveData {
        ResolveData::Refused {
            reason: "AccessDenied".into(),
        }
    }

    fn marks(keys: &[&str]) -> Arc<BTreeSet<String>> {
        Arc::new(keys.iter().map(ToString::to_string).collect())
    }

    fn answers_nothing(_: String, _: Option<S3PackageUri>) -> Answer<String> {
        Box::pin(std::future::pending())
    }

    fn idle_commands() -> ResolveCommands {
        ResolveCommands {
            certify: answers_nothing,
            reset: answers_nothing,
        }
    }

    fn pane(
        resolve: ResolveData,
        marked: impl Into<Option<Arc<BTreeSet<String>>>>,
        w: Wiring,
    ) -> web_sys::Element {
        let marked = marked.into();
        mount(move || {
            view! {
                <ResolvePane
                    namespace=ns()
                    uri=None
                    revision=revision()
                    resolve=resolve
                    marks=Signal::stored(marked)
                    back_href=BACK
                    w=w
                    commands=idle_commands()
                />
            }
        })
    }

    /// The `section` a nested heading names.
    fn section(el: &web_sys::Element, heading: &str) -> web_sys::Element {
        let all = el.query_selector_all("h4").unwrap();
        (0..all.length())
            .map(|i| all.item(i).unwrap().unchecked_into::<web_sys::Element>())
            .find(|h| h.text_content().unwrap_or_default().trim() == heading)
            .unwrap_or_else(|| panic!("no h4 says {heading:?}; markup was {}", el.inner_html()))
            .closest("section")
            .unwrap()
            .expect("the heading's section")
    }

    fn has_heading(el: &web_sys::Element, heading: &str) -> bool {
        let all = el.query_selector_all("h4").unwrap();
        (0..all.length()).any(|i| {
            all.item(i)
                .unwrap()
                .text_content()
                .unwrap_or_default()
                .trim()
                == heading
        })
    }

    /// The button whose text is `label`, as `header.rs`'s helper finds one.
    fn button(el: &web_sys::Element, label: &str) -> web_sys::HtmlButtonElement {
        let all = el.query_selector_all("button").unwrap();
        (0..all.length())
            .map(|i| all.item(i).unwrap().unchecked_into::<web_sys::Element>())
            .find(|b| b.text_content().unwrap_or_default().trim() == label)
            .unwrap_or_else(|| panic!("no button says {label:?}; markup was {}", el.inner_html()))
            .unchecked_into()
    }

    const CERTIFY: &str = "Make mine the shared one";
    const REPLACE: &str = "Replace mine with the published one";

    #[wasm_bindgen_test]
    fn the_mode_names_both_sides() {
        let el = pane(
            compared(0, 0),
            marks(&["plate/a.csv", "plate/b.csv"]),
            Wiring::new(),
        );

        let sentence = element_saying(
            &el,
            "2 files differ between these revisions — marked in the list.",
        );
        assert_eq!(sentence.id(), DIFFERS_ID);

        let yours = section(&el, "Yours");
        assert_eq!(yours.query_selector_all("time").unwrap().length(), 1);
        element_saying(&yours, "\u{201c}Mine\u{201d}");

        let published = section(&el, "Published");
        element_saying(&published, "\u{201c}Theirs\u{201d}");
        assert!(
            published.query_selector("time").unwrap().is_none(),
            "the published side has no time; markup was {}",
            published.inner_html()
        );

        let back = el
            .query_selector(&format!("a[href='{BACK}']"))
            .unwrap()
            .expect("the back link");
        assert_eq!(
            back.text_content().unwrap_or_default().trim(),
            "team/dataset"
        );

        for label in [CERTIFY, REPLACE] {
            assert!(!button(&el, label).disabled(), "{label} is enabled");
        }
    }

    #[wasm_bindgen_test]
    fn a_published_revision_with_no_message_says_so() {
        let resolve = ResolveData::Compared {
            published_message: None,
            differing: vec!["plate/a.csv".into()],
            unpublished: 0,
            uncommitted: 0,
        };
        let el = pane(resolve, marks(&["plate/a.csv"]), Wiring::new());
        element_saying(&section(&el, "Published"), "No message");
    }

    /// Pins the seam: the count reads the page's set, not the payload's.
    #[wasm_bindgen_test]
    fn the_count_is_the_page_s_set_not_the_payload_s() {
        let resolve = ResolveData::Compared {
            published_message: Some("Theirs".into()),
            differing: vec![
                "plate/a.csv".into(),
                "plate/b.csv".into(),
                "plate/c.csv".into(),
            ],
            unpublished: 0,
            uncommitted: 0,
        };
        let el = pane(
            resolve,
            marks(&["plate/a.csv", "plate/b.csv"]),
            Wiring::new(),
        );
        element_saying(
            &el,
            "2 files differ between these revisions — marked in the list.",
        );
    }

    #[wasm_bindgen_test]
    fn a_refused_comparison_keeps_yours_and_disables_both_choices() {
        let w = Wiring::new();
        let el = pane(refused(), None, w);
        element_saying(&el, "Could not compare the revisions.");
        element_saying(&el, "AccessDenied");
        section(&el, "Yours");
        assert!(
            !has_heading(&el, "Published"),
            "markup was {}",
            el.inner_html()
        );
        button(&el, "Try again");
        assert!(!w.busy.get_untracked());
        for label in [CERTIFY, REPLACE] {
            assert!(button(&el, label).disabled(), "{label} is disabled");
        }
    }

    #[wasm_bindgen_test]
    async fn try_again_re_reads_the_page() {
        thread_local! {
            static RELOADS: Cell<u32> = const { Cell::new(0) };
        }
        let w = Wiring::new();
        let el = mount(move || {
            Effect::new(move |seen: Option<()>| {
                w.reload.track();
                // The first run is the subscription, not a re-read.
                if seen.is_some() {
                    RELOADS.set(RELOADS.get() + 1);
                }
            });
            view! {
                <ResolvePane
                    namespace=ns()
                    uri=None
                    revision=revision()
                    resolve=refused()
                    marks=Signal::stored(None)
                    back_href=BACK
                    w=w
                    commands=idle_commands()
                />
            }
        });
        leptos::task::tick().await;
        button(&el, "Try again").click();
        leptos::task::tick().await;
        assert_eq!(RELOADS.get(), 1);
    }

    #[wasm_bindgen_test]
    async fn while_another_command_runs_both_choices_are_sealed() {
        let w = Wiring::new();
        let el = pane(compared(0, 0), marks(&["plate/a.csv"]), w);
        w.busy.set(true);
        leptos::task::tick().await;
        for label in [CERTIFY, REPLACE] {
            assert!(button(&el, label).disabled(), "{label} is disabled");
        }
    }

    #[wasm_bindgen_test]
    fn both_choices_may_take_two_lines() {
        let el = pane(compared(0, 0), marks(&["plate/a.csv"]), Wiring::new());
        for label in [CERTIFY, REPLACE] {
            assert!(
                button(&el, label).has_attribute("data-wrap"),
                "{label} wraps"
            );
        }
    }

    thread_local! {
        static CERTIFIED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
        static RESET: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
        static RELOADED: Cell<u32> = const { Cell::new(0) };
    }

    /// Forget what earlier tests recorded: the cells outlive each test.
    fn clear() {
        CERTIFIED.with_borrow_mut(Vec::clear);
        RESET.with_borrow_mut(Vec::clear);
        RELOADED.set(0);
    }

    const CERTIFY_REFUSED: &str = "AccessDenied: cannot tag latest";
    const RESET_REFUSED: &str = "A file is open in another program.";

    fn certifies_ok(namespace: String, _: Option<S3PackageUri>) -> Answer<String> {
        CERTIFIED.with_borrow_mut(|c| c.push(namespace));
        Box::pin(async { Ok(String::new()) })
    }

    fn refuses_certify(namespace: String, _: Option<S3PackageUri>) -> Answer<String> {
        CERTIFIED.with_borrow_mut(|c| c.push(namespace));
        Box::pin(async { Err(CERTIFY_REFUSED.to_string()) })
    }

    fn resets_ok(namespace: String, _: Option<S3PackageUri>) -> Answer<String> {
        RESET.with_borrow_mut(|r| r.push(namespace));
        Box::pin(async { Ok(String::new()) })
    }

    fn refuses_reset(namespace: String, _: Option<S3PackageUri>) -> Answer<String> {
        RESET.with_borrow_mut(|r| r.push(namespace));
        Box::pin(async { Err(RESET_REFUSED.to_string()) })
    }

    fn reset_never(namespace: String, _: Option<S3PackageUri>) -> Answer<String> {
        RESET.with_borrow_mut(|r| r.push(namespace));
        Box::pin(std::future::pending())
    }

    fn with(certify: super::RevisionChoice, reset: super::RevisionChoice) -> ResolveCommands {
        ResolveCommands { certify, reset }
    }

    /// The pane over `commands`, with `w.reload` counted into `RELOADED`.
    async fn choosing(
        resolve: ResolveData,
        marked: Arc<BTreeSet<String>>,
        w: Wiring,
        commands: ResolveCommands,
    ) -> web_sys::Element {
        clear();
        let el = mount(move || {
            Effect::new(move |seen: Option<()>| {
                w.reload.track();
                // The first run is the subscription, not a re-read.
                if seen.is_some() {
                    RELOADED.set(RELOADED.get() + 1);
                }
            });
            view! {
                <ResolvePane
                    namespace=ns()
                    uri=None
                    revision=revision()
                    resolve=resolve
                    marks=Signal::stored(Some(marked))
                    back_href=BACK
                    w=w
                    commands=commands
                />
            }
        });
        leptos::task::tick().await;
        el
    }

    /// A task boundary and a tick: the stub settles on the event loop, then the DOM.
    async fn settle() {
        sleep_ms(0).await;
        leptos::task::tick().await;
    }

    fn open_dialog(el: &web_sys::Element) -> Option<web_sys::Element> {
        el.query_selector("dialog[open]").unwrap()
    }

    fn two_marks() -> Arc<BTreeSet<String>> {
        marks(&["plate/a.csv", "plate/b.csv"])
    }

    const TITLE: &str = "Replace yours with the published revision";

    #[wasm_bindgen_test]
    async fn making_mine_the_shared_one_runs_at_once_and_says_so() {
        let w = Wiring::new();
        let el = choosing(
            compared(0, 0),
            two_marks(),
            w,
            with(certifies_ok, resets_ok),
        )
        .await;
        button(&el, CERTIFY).click();
        settle().await;

        assert_eq!(CERTIFIED.with_borrow(Clone::clone), vec!["team/dataset"]);
        assert!(!w.dialogs.replace.get_untracked(), "no confirmation");
        assert_eq!(
            w.outcome.get_untracked(),
            Some(Outcome {
                namespace: "team/dataset".into(),
                variant: BannerVariant::Success,
                lead: "Your revision is now the shared one.".into(),
                detail: None,
            })
        );
        assert_eq!(w.replace_to.get_untracked().as_deref(), Some(BACK));
        assert_eq!(RELOADED.get(), 1, "the page re-reads");
    }

    #[wasm_bindgen_test]
    async fn a_refused_certify_tells_the_band_and_keeps_the_mode() {
        let w = Wiring::new();
        let el = choosing(
            compared(0, 0),
            two_marks(),
            w,
            with(refuses_certify, resets_ok),
        )
        .await;
        button(&el, CERTIFY).click();
        settle().await;

        assert_eq!(
            w.outcome.get_untracked(),
            Some(Outcome {
                namespace: "team/dataset".into(),
                variant: BannerVariant::Critical,
                lead: "Could not make your revision the shared one.".into(),
                detail: Some(CERTIFY_REFUSED.into()),
            })
        );
        assert_eq!(w.replace_to.get_untracked(), None);
        assert_eq!(RELOADED.get(), 0, "nothing to re-read");
        section(&el, "Yours");
    }

    #[wasm_bindgen_test]
    async fn replace_asks_first() {
        let w = Wiring::new();
        let el = choosing(
            compared(0, 0),
            two_marks(),
            w,
            with(certifies_ok, resets_ok),
        )
        .await;
        button(&el, REPLACE).click();
        leptos::task::tick().await;

        let dialog = open_dialog(&el).expect("the confirmation is open");
        element_saying(&dialog, TITLE);
        assert!(RESET.with_borrow(Vec::is_empty), "nothing ran yet");
    }

    #[wasm_bindgen_test]
    async fn the_confirmation_names_each_extent() {
        let w = Wiring::new();
        let el = choosing(
            compared(2, 1),
            two_marks(),
            w,
            with(certifies_ok, resets_ok),
        )
        .await;
        button(&el, REPLACE).click();
        leptos::task::tick().await;

        let dialog = open_dialog(&el).expect("the confirmation is open");
        element_saying(
            &dialog,
            "Discards 2 unpublished revisions, replaces 2 files that differ from the \
             published revision, and overwrites uncommitted edits to 1 file. This cannot \
             be undone.",
        );
    }

    #[wasm_bindgen_test]
    async fn confirming_replaces_and_says_so() {
        let w = Wiring::new();
        let el = choosing(
            compared(0, 0),
            two_marks(),
            w,
            with(certifies_ok, resets_ok),
        )
        .await;
        button(&el, REPLACE).click();
        leptos::task::tick().await;
        button(&el, "Replace mine").click();
        settle().await;
        settle().await;

        assert_eq!(RESET.with_borrow(Clone::clone), vec!["team/dataset"]);
        assert!(open_dialog(&el).is_none(), "the confirmation closed");
        assert_eq!(
            w.outcome.get_untracked(),
            Some(Outcome {
                namespace: "team/dataset".into(),
                variant: BannerVariant::Success,
                lead: "Replaced with the published revision.".into(),
                detail: None,
            })
        );
        assert_eq!(w.replace_to.get_untracked().as_deref(), Some(BACK));
        assert_eq!(RELOADED.get(), 1, "the page re-reads");
    }

    #[wasm_bindgen_test]
    async fn a_refused_replace_stays_in_the_dialog_with_the_mode_intact() {
        let w = Wiring::new();
        let el = choosing(
            compared(0, 0),
            two_marks(),
            w,
            with(certifies_ok, refuses_reset),
        )
        .await;
        button(&el, REPLACE).click();
        leptos::task::tick().await;
        button(&el, "Replace mine").click();
        settle().await;

        let dialog = open_dialog(&el).expect("the confirmation stays open");
        let alert = dialog
            .query_selector("[role=alert]")
            .unwrap()
            .expect("the refusal, inside the dialog");
        assert!(
            alert
                .text_content()
                .unwrap_or_default()
                .contains(RESET_REFUSED),
            "it says what the backend said: {:?}",
            alert.text_content()
        );
        assert_eq!(
            w.outcome.get_untracked(),
            None,
            "the band is not its channel"
        );
        assert_eq!(w.replace_to.get_untracked(), None);
        section(&el, "Yours");
        element_saying(
            &el,
            "2 files differ between these revisions — marked in the list.",
        );
    }

    #[wasm_bindgen_test]
    async fn a_running_replace_holds_the_page() {
        let w = Wiring::new();
        let el = choosing(
            compared(0, 0),
            two_marks(),
            w,
            with(certifies_ok, reset_never),
        )
        .await;
        button(&el, REPLACE).click();
        leptos::task::tick().await;
        button(&el, "Replace mine").click();
        settle().await;

        assert!(w.busy.get_untracked(), "the page is held");
        for label in [CERTIFY, REPLACE] {
            assert!(button(&el, label).disabled(), "{label} is disabled");
        }
    }

    #[wasm_bindgen_test]
    async fn a_dialog_rebuilt_mid_reset_is_drawn_sealed() {
        let w = Wiring::new();
        w.busy.set(true);
        w.dialogs.replace.set(true);
        let el = choosing(
            compared(0, 0),
            two_marks(),
            w,
            with(certifies_ok, resets_ok),
        )
        .await;

        let dialog = open_dialog(&el).expect("the confirmation is open");
        let verb = button(&dialog, "Replace mine");
        assert!(
            verb.get_attribute("aria-busy").as_deref() == Some("true") || verb.disabled(),
            "the verb is sealed; markup was {}",
            dialog.inner_html()
        );
    }
}
