//! Removing old revisions from the "Revisions you have" popover.
//!
//! The gallery's `old_revisions_inline.rs` scene is the design record; this is
//! the same surface over the backend's answer.
//!
//! # What a row says
//!
//! A removable row says under its time what removing it alone frees and ends
//! in a trash button. A protected one says why it is kept in a short tag, with
//! the reasons spelled out for the pointer, and has no button. The footer
//! removes every removable row, at the figure the backend measured for the set,
//! which can be more than the rows add up to.
//!
//! # The confirmation is the page's
//!
//! The trash and the footer open the kit's `ConfirmDialog`, mounted beside the
//! pane and not inside the popover: the popover's light dismiss would unmount a
//! dialog inside it mid-question, and `showModal()` hides every popover that is
//! not the dialog's ancestor, so opening the dialog usually closes the popover.
//! Its question and open flag live in [`Removal`], on the page's `Wiring`, so a
//! re-read that rebuilds the pane does not take the question away.
//!
//! # Remove closes the dialog; the work shows elsewhere
//!
//! The verb's action starts the removal and answers at once. While it runs the
//! app bar's activity line says so and every remove button is disabled, as it
//! is while autopull syncs the package or the page runs a command of its own.
//! The backend posts the result to the notification stack; a refusal reaches
//! the page's band. A success re-reads the page, so the count drops.

use std::future::Future;
use std::pin::Pin;

use leptos::prelude::*;

use super::Outcome;
use super::Wiring;
use crate::commands;
use crate::commands::KeptReason;
use crate::commands::RevisionHistoryData;
use crate::commands::RevisionHistoryRow;
use crate::kit::Activities;
use crate::kit::Activity;
use crate::kit::ActivityKind;
use crate::kit::BannerVariant;
use crate::kit::Button;
use crate::kit::CatalogLink;
use crate::kit::ConfirmDialog;
use crate::kit::IconButton;
use crate::kit::IconButtonVariant;
use crate::kit::PaneSection;
use crate::kit::RevisionRow;
use crate::kit::Submit;
use crate::kit::icons;

stylance::import_crate_style!(
    style,
    "src/pages/installed_package_v2/old_revisions.module.scss"
);

/// What removes revisions. A function pointer, so the DOM tests answer without
/// a Tauri host.
pub type RemoveRevisionsFn =
    fn(String, Vec<String>) -> Pin<Box<dyn Future<Output = Result<String, String>>>>;

/// The app's removal.
pub(super) fn remove_revisions(
    namespace: String,
    hashes: Vec<String>,
) -> Pin<Box<dyn Future<Output = Result<String, String>>>> {
    Box::pin(commands::remove_revisions(namespace, hashes))
}

const MB: u64 = 1_000_000;

/// A size on a row or the footer: most old revisions free little or nothing,
/// and `0.2 MB` would claim a precision nobody has a use for.
pub(super) fn size(bytes: u64) -> String {
    match bytes {
        0 => "nothing".to_string(),
        1..MB => "< 1 MB".to_string(),
        _ => {
            // Display rounding only.
            #[allow(clippy::cast_precision_loss)]
            let mb = bytes as f64 / MB as f64;
            if mb < 999.95 {
                format!("{mb:.1} MB")
            } else {
                format!("{:.1} GB", mb / 1000.0)
            }
        }
    }
}

/// The same figure in a sentence, where "frees nothing" and "< 1" read badly.
pub(super) fn spelled(bytes: u64) -> String {
    match bytes {
        0 => "no space".to_string(),
        1..MB => "less than 1 MB".to_string(),
        _ => size(bytes),
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

/// A protected row's tag: `current · not pushed`.
pub(super) fn kept_tag(kept: &[KeptReason]) -> String {
    kept.iter()
        .map(|reason| match reason {
            KeptReason::Current => "current",
            KeptReason::Latest => "latest",
            KeptReason::Base => "base",
            KeptReason::NotPushed => "not pushed",
            KeptReason::Unpublished => "unpublished",
        })
        .collect::<Vec<_>>()
        .join(" \u{b7} ")
}

/// The tag spelled out: "Kept: your files are at this revision, and it has not
/// been pushed."
pub(super) fn kept_title(kept: &[KeptReason]) -> String {
    let parts: Vec<&str> = kept
        .iter()
        .map(|reason| match reason {
            KeptReason::Current => "your files are at this revision",
            KeptReason::Latest => "the latest published revision",
            KeptReason::Base => "the one your changes are based on",
            KeptReason::NotPushed => "it has not been pushed",
            KeptReason::Unpublished => "it was never published, so this computer has the only copy",
        })
        .collect();
    let said = match parts.as_slice() {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    };
    format!("Kept: {said}.")
}

/// A row's name in a sentence: its message in quotes, or what it lacks.
fn named(row: &RevisionHistoryRow) -> String {
    match row.message.as_deref().map(str::trim) {
        None | Some("") => "the revision with no message".to_string(),
        Some(message) => format!("\u{201c}{message}\u{201d}"),
    }
}

/// The activity line's words while `count` revisions of `namespace` go.
pub(super) fn progress(count: usize, namespace: &str) -> String {
    format!(
        "Removing {} of {namespace}\u{2026}",
        plural(count, "old revision", "old revisions")
    )
}

/// What the confirmation asks: which revisions of which package, and the words
/// for its title and its one sentence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub namespace: String,
    pub hashes: Vec<String>,
    pub title: &'static str,
    pub consequence: String,
}

impl Ask {
    /// The footer's: every removable row, freed as a set.
    pub(super) fn older(namespace: &str, removable: &[&RevisionHistoryRow], frees: u64) -> Self {
        Self {
            namespace: namespace.to_string(),
            hashes: removable.iter().map(|row| row.hash.clone()).collect(),
            title: "Remove old revisions",
            consequence: format!(
                "Remove {}? This frees {}.",
                plural(removable.len(), "older revision", "older revisions"),
                spelled(frees),
            ),
        }
    }

    /// A row's: that revision alone.
    pub(super) fn one(namespace: &str, row: &RevisionHistoryRow) -> Self {
        Self {
            namespace: namespace.to_string(),
            hashes: vec![row.hash.clone()],
            title: "Remove a revision",
            consequence: format!(
                "Remove {}? This frees {}.",
                named(row),
                spelled(row.frees.unwrap_or_default()),
            ),
        }
    }
}

/// The page's half of a removal: what is running, and what the confirmation
/// asks. On `Wiring`, so the watcher's re-read neither closes the dialog nor
/// forgets that a removal runs.
#[derive(Clone, Copy)]
pub struct Removal {
    /// The package and the set being removed, while they are.
    pub running: RwSignal<Option<(String, Vec<String>)>>,
    pub ask: RwSignal<Option<Ask>>,
    pub open: RwSignal<bool>,
}

impl Removal {
    #[must_use]
    pub fn new() -> Self {
        Self {
            running: RwSignal::new(None),
            ask: RwSignal::new(None),
            open: RwSignal::new(false),
        }
    }
}

impl Default for Removal {
    fn default() -> Self {
        Self::new()
    }
}

/// What the popover's removal reads: the package, the page, and how to remove.
#[derive(Clone, Copy)]
pub(super) struct Remover {
    pub namespace: StoredValue<String>,
    /// Stored, so a `Remover` stays small enough to pass by value.
    pub w: StoredValue<Wiring>,
    pub remove: RemoveRevisionsFn,
    /// The app's activity line, read when the pane was built: none in the
    /// gallery and the tests, which then never see a sync.
    pub activities: Option<Activities>,
}

impl Remover {
    pub(super) fn new(
        namespace: StoredValue<String>,
        w: Wiring,
        remove: RemoveRevisionsFn,
    ) -> Self {
        Self {
            namespace,
            w: StoredValue::new(w),
            remove,
            activities: use_context::<Activities>(),
        }
    }

    /// Autopull is moving this package's files, so it holds the lock.
    fn syncing(self) -> bool {
        let namespace = self.namespace.get_value();
        self.activities
            .is_some_and(|activities| activities.busy_with(ActivityKind::Autopull, &namespace))
    }

    /// Every remove button's `disabled`: a sync or a page command holds the
    /// package's lock, and a removal already running owns the store.
    fn blocked(self) -> bool {
        let w = self.w.get_value();
        self.syncing() || w.busy.get() || w.removal.running.get().is_some()
    }

    fn confirm(self, ask: Ask) {
        let w = self.w.get_value();
        w.removal.ask.set(Some(ask));
        w.removal.open.set(true);
    }

    /// Start removing what `ask` names and return, as the verb's action does.
    ///
    /// Refused, with the dialog's banner, when a sync or a page command took
    /// the package after the dialog opened: the lock would refuse it anyway.
    fn start(self, ask: Ask) -> Result<(), String> {
        if untrack(|| self.blocked()) {
            return Err(format!(
                "{} is busy \u{2014} try again in a moment",
                ask.namespace
            ));
        }
        let Self {
            w,
            remove,
            activities,
            ..
        } = self;
        let w = w.get_value();
        let Ask {
            namespace, hashes, ..
        } = ask;
        w.removal
            .running
            .set(Some((namespace.clone(), hashes.clone())));
        w.outcome.set(None);
        if let Some(activities) = activities {
            activities.set(
                ActivityKind::RemoveRevisions,
                vec![Activity {
                    kind: ActivityKind::RemoveRevisions,
                    label: progress(hashes.len(), &namespace),
                    package: Some(namespace.clone()),
                }],
            );
        }
        leptos::task::spawn_local(async move {
            let answer = remove(namespace.clone(), hashes).await;
            // `try_`: the signals are the page's, and the page can be gone.
            w.removal.running.try_set(None);
            if let Some(activities) = activities {
                activities.set(ActivityKind::RemoveRevisions, Vec::new());
            }
            if let Err(detail) = answer {
                w.outcome.try_set(Some(Outcome {
                    namespace,
                    variant: BannerVariant::Critical,
                    lead: "Could not remove old revisions.".to_string(),
                    detail: Some(detail),
                }));
            }
            // A refusal re-reads too: part of a removal may have landed, and a
            // list still offering what is gone would refuse every retry.
            w.reload.notify();
        });
        Ok(())
    }
}

/// The rows, then the footer.
pub(super) fn rows_and_footer(
    data: RevisionHistoryData,
    open_catalog: Callback<String>,
    remover: Option<Remover>,
) -> AnyView {
    let RevisionHistoryData {
        rows,
        removable_frees,
    } = data;
    // Offered only with a way to remove and a measure: without the measure,
    // the backend could not read a manifest, and the removal would refuse. A
    // list with nothing removable needs no measure to say so.
    let all_kept = rows.iter().all(|row| !row.kept.is_empty());
    let offering = remover.filter(|_| removable_frees.is_some() || all_kept);
    let any_removable = offering.is_some() && rows.iter().any(|row| row.kept.is_empty());
    let drawn = rows
        .iter()
        .map(|row| row_view(row, open_catalog, offering, any_removable))
        .collect_view();
    let footer = offering
        .filter(|_| rows.len() > 1)
        .map(|remover| footer(&rows, removable_frees.unwrap_or_default(), remover));
    view! {
        <PaneSection>
            <div class=style::rows>{drawn}</div>
        </PaneSection>
        {footer}
    }
    .into_any()
}

/// One row: the kit's revision row, its detail, and its trash or a spacer.
fn row_view(
    row: &RevisionHistoryRow,
    open_catalog: Callback<String>,
    offering: Option<Remover>,
    any_removable: bool,
) -> AnyView {
    let catalog = row
        .catalog_url
        .clone()
        .map(|href| CatalogLink::new(href, open_catalog));
    let message = row.message.clone().unwrap_or_default();
    let published = row.published;
    let at = row.obtained_at;
    if !row.kept.is_empty() {
        // A kept row holds the trash's width empty, so every catalog icon in
        // the column lines up — except when no row has a trash.
        let trailing = any_removable
            .then(|| view! { <span class=style::gap aria-hidden="true"></span> }.into_any());
        return match trailing {
            Some(trailing) => view! {
                <RevisionRow
                    message=message
                    at=at
                    published=published
                    catalog=catalog
                    detail=kept_tag(&row.kept)
                    detail_title=kept_title(&row.kept)
                    trailing=trailing
                />
            }
            .into_any(),
            None => view! {
                <RevisionRow
                    message=message
                    at=at
                    published=published
                    catalog=catalog
                    detail=kept_tag(&row.kept)
                    detail_title=kept_title(&row.kept)
                />
            }
            .into_any(),
        };
    }
    let Some(remover) = offering else {
        return view! {
            <RevisionRow message=message at=at published=published catalog=catalog />
        }
        .into_any();
    };
    let detail = format!("frees {}", size(row.frees.unwrap_or_default()));
    let ask = Ask::one(&remover.namespace.get_value(), row);
    let label = format!("Remove {}", named(row));
    let trash = view! {
        <IconButton
            icon=icons::trash()
            aria_label=label
            variant=IconButtonVariant::Invisible
            disabled=Signal::derive(move || remover.blocked())
            on_click=move |_| remover.confirm(ask.clone())
        />
    }
    .into_any();
    view! {
        <RevisionRow
            message=message
            at=at
            published=published
            catalog=catalog
            detail=detail
            trailing=trash
        />
    }
    .into_any()
}

/// The footer: one button for every removable row, and a line saying why it
/// is not there or does not work now.
fn footer(rows: &[RevisionHistoryRow], frees: u64, remover: Remover) -> AnyView {
    let removable: Vec<&RevisionHistoryRow> =
        rows.iter().filter(|row| row.kept.is_empty()).collect();
    let namespace = remover.namespace.get_value();
    let busy_line = format!("{namespace} is busy syncing \u{2014} try again in a moment");
    let line = move || {
        if remover.syncing() {
            Some(busy_line.clone())
        } else {
            None
        }
    };
    let button = (!removable.is_empty()).then(|| {
        let count = removable.len();
        let ask = Ask::older(&namespace, &removable, frees);
        let hashes = ask.hashes.clone();
        // Spinning when the set it names is the one being removed; only
        // disabled when a row's removal, a sync or a page command holds it.
        let loading = Signal::derive(move || {
            remover
                .w
                .get_value()
                .removal
                .running
                .with(|running| running.as_ref().is_some_and(|(_, set)| *set == hashes))
        });
        view! {
            <Button
                loading=loading
                disabled=Signal::derive(move || remover.blocked())
                on_click=move |_| remover.confirm(ask.clone())
            >
                {format!("Remove {count} older \u{b7} frees {}", size(frees))}
            </Button>
        }
    });
    let nothing = removable.is_empty().then(|| {
        view! {
            <p class=style::muted>
                "Nothing to remove \u{2014} every revision here is in use"
            </p>
        }
    });
    view! {
        <PaneSection>
            <div class=style::footer>
                {move || line().map(|line| view! { <p class=style::muted>{line}</p> })}
                {nothing}
                {button}
            </div>
        </PaneSection>
    }
    .into_any()
}

/// The confirmation, rebuilt for each question — `ConfirmDialog` takes its
/// words once. Mounted beside the pane, never inside the popover. Draws only a
/// question about this package: one route serves every package.
pub(super) fn dialog(remover: Remover) -> impl IntoView {
    let removal = remover.w.get_value().removal;
    move || {
        removal
            .ask
            .get()
            .filter(|ask| ask.namespace == remover.namespace.get_value())
            .map(|ask| {
                let title = ask.title;
                let consequence = ask.consequence.clone();
                view! {
                    <ConfirmDialog
                        open=removal.open
                        title=title
                        consequence=consequence
                        confirm=Submit::new(
                            "Remove",
                            move || {
                                let ask = ask.clone();
                                async move {
                                    remover.start(ask)
                                }
                            },
                        )
                    />
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(message: Option<&str>, frees: Option<u64>) -> RevisionHistoryRow {
        RevisionHistoryRow {
            hash: "h".to_string(),
            message: message.map(ToString::to_string),
            obtained_at: 0.0,
            published: true,
            catalog_url: None,
            kept: Vec::new(),
            frees,
        }
    }

    #[test]
    fn sizes_are_coarse_and_never_claim_precision() {
        assert_eq!(size(0), "nothing");
        assert_eq!(size(200_000), "< 1 MB");
        assert_eq!(size(1_200_000), "1.2 MB");
        assert_eq!(size(6_900_000), "6.9 MB");
        assert_eq!(size(140_000_000), "140.0 MB");
        assert_eq!(size(2_500_000_000), "2.5 GB");
        assert_eq!(spelled(0), "no space");
        assert_eq!(spelled(200_000), "less than 1 MB");
        assert_eq!(spelled(4_800_000), "4.8 MB");
    }

    #[test]
    fn a_kept_row_says_why_short_and_long() {
        let kept = [KeptReason::Current, KeptReason::NotPushed];
        assert_eq!(kept_tag(&kept), "current \u{b7} not pushed");
        assert_eq!(
            kept_title(&kept),
            "Kept: your files are at this revision, and it has not been pushed."
        );
        let kept = [KeptReason::Latest, KeptReason::Base];
        assert_eq!(kept_tag(&kept), "latest \u{b7} base");
        assert_eq!(
            kept_title(&kept),
            "Kept: the latest published revision, and the one your changes are based on."
        );
    }

    #[test]
    fn the_questions_say_what_goes_and_what_it_frees() {
        let initial = row(Some("Initial upload"), Some(4_800_000));
        assert_eq!(
            Ask::one("user/plate-07", &initial).consequence,
            "Remove \u{201c}Initial upload\u{201d}? This frees 4.8 MB."
        );
        let unnamed = row(None, Some(0));
        assert_eq!(
            Ask::one("user/plate-07", &unnamed).consequence,
            "Remove the revision with no message? This frees no space."
        );
        let older = Ask::older("user/plate-07", &[&initial, &unnamed], 6_900_000);
        assert_eq!(older.title, "Remove old revisions");
        assert_eq!(
            older.consequence,
            "Remove 2 older revisions? This frees 6.9 MB."
        );
        assert_eq!(
            progress(4, "user/plate-07"),
            "Removing 4 old revisions of user/plate-07\u{2026}"
        );
    }
}
