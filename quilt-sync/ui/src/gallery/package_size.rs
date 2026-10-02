//! The package's total size in the context pane, in two placements.
//!
//! # What the figure is
//!
//! The sum of `size` over the current revision's manifest rows, and beside it
//! the sum over the rows whose paths are downloaded. Both are read off the
//! manifest the page already holds, so nothing is stat'ed. When only some files
//! are here the pane says both; when all are, one; when none are, the total.
//!
//! It is the package's **logical** size, not what it takes on this disk. On
//! APFS the object store and the working files share blocks, so a disk figure
//! would be smaller and would move with things the reader never did. The words
//! never claim space: they say *downloaded*, about files. "Frees", in the
//! revisions surface next door, is the one figure about this computer's disk.
//!
//! # Two placements
//!
//! - **P1**, a line in Revision under the bucket: a fact about the package,
//!   beside the other facts about it.
//! - **P2**, joined to Keeping's count: "54 of 56 files · 1.9 MB of 3.4 MB
//!   downloaded". The recommended one. The count already answers *how much of
//!   this package is here*, and the size is the same question in bytes — the
//!   choice above it is what moves both. A bare "3.4 MB" under
//!   `s3://quilt-lab-plates` reads as the bucket's size, and it sits right on
//!   the "Revisions you have" trigger, whose surface is all "frees" figures.
//!
//! # A size that cannot be read says nothing
//!
//! Not a dash and not an error: the count is still true, so P2 falls back to
//! today's caption and P1 drops its line. A dash in a sentence reads as
//! punctuation, and a size the reader did not ask for is not worth a failure.
//!
//! # What the real build needs
//!
//! - `PackageContextData.size: Option<PackageSize>`, with
//!   `PackageSize { total: u64, downloaded: u64 }` in bytes, camelCase on the
//!   wire. `None` when a row's size cannot be read; summed in
//!   `get_package_page_data` from the rows it already reads for `keeping`.
//! - Because it comes with the page, the size has no loading state of its own;
//!   the skeleton cells show what it would be if it were ever fetched apart.
//!   P2's then needs `ChoiceGroup`'s caption to take a view, which is a
//!   `String` today — the bar here sits under the caption instead of in it.
//! - One byte formatter. `util::format_size` writes two decimals ("2.50 MB")
//!   and the revisions surface's helper is private and counts tenths of a MB,
//!   so [`bytes`] writes one decimal to match the surface it stands beside.
//! - The real caption prints counts ungrouped ("140000"); [`count`] groups them.

use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::kit::Button;
use crate::kit::Card;
use crate::kit::CatalogLink;
use crate::kit::Choice;
use crate::kit::ChoiceGroup;
use crate::kit::IconButton;
use crate::kit::IconButtonVariant;
use crate::kit::PaneSection;
use crate::kit::RevisionRow;
use crate::kit::SkeletonText;

const NAMESPACE: &str = "user/plate-07";
const BUCKET: &str = "s3://quilt-lab-plates";

const MINUTE: f64 = 60_000.0;
const HOUR: f64 = 60.0 * MINUTE;
const DAY: f64 = 24.0 * HOUR;

const MB: u64 = 1_000_000;

fn ago(ms: f64) -> f64 {
    js_sys::Date::now() - ms
}

/// What the page read knows about the size.
#[derive(Clone, Copy)]
enum Size {
    Loading,
    /// A row's size could not be read.
    Unread,
    /// Bytes in the revision, and in the files of it that are here.
    Known {
        total: u64,
        here: u64,
    },
}

/// One state of the package, as the pane sees it.
#[derive(Clone, Copy)]
struct Case {
    /// The Keeping choice: `pick` or `all`.
    scope: &'static str,
    files: usize,
    /// Files of the revision that are downloaded.
    present: usize,
    size: Size,
}

const ALL_HERE: Case = Case {
    scope: "all",
    files: 56,
    present: 56,
    size: Size::Known {
        total: 34 * MB / 10,
        here: 34 * MB / 10,
    },
};

const SOME_HERE: Case = Case {
    scope: "pick",
    files: 56,
    present: 54,
    size: Size::Known {
        total: 34 * MB / 10,
        here: 19 * MB / 10,
    },
};

const NONE_HERE: Case = Case {
    scope: "pick",
    files: 56,
    present: 0,
    size: Size::Known {
        total: 34 * MB / 10,
        here: 0,
    },
};

const EMPTY: Case = Case {
    scope: "pick",
    files: 0,
    present: 0,
    size: Size::Known { total: 0, here: 0 },
};

const HUGE: Case = Case {
    scope: "pick",
    files: 140_000,
    present: 12_400,
    size: Size::Known {
        total: 1_200_000 * MB,
        here: 86_300 * MB,
    },
};

const LOADING: Case = Case {
    size: Size::Loading,
    ..SOME_HERE
};

const UNREAD: Case = Case {
    size: Size::Unread,
    ..SOME_HERE
};

/// A logical size, in decimal units and one decimal — the revisions surface's
/// "1.2 MB", carried up to TB and down to bytes. The space is a no-break one:
/// at 280px the caption wraps, and "1.2" ending a line with "TB" starting the
/// next reads as two figures.
fn bytes(n: u64) -> String {
    const UNITS: [&str; 4] = ["kB", "MB", "GB", "TB"];
    if n < 1000 {
        return format!("{n}\u{a0}B");
    }
    let mut scale = 1000u64;
    let mut tenths = 0;
    let mut unit = UNITS[0];
    for next in UNITS {
        unit = next;
        tenths = (n.saturating_mul(10) + scale / 2) / scale;
        // Rounding up into the next unit's "1000.0" moves on to that unit.
        if tenths < 10_000 {
            break;
        }
        scale *= 1000;
    }
    format!("{}.{}\u{a0}{unit}", tenths / 10, tenths % 10)
}

/// A file count, grouped in thousands — "140,000" is read, "140000" is counted.
fn count(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// P1's line under the bucket, or nothing when there is no size to say.
fn revision_line(case: Case) -> Option<AnyView> {
    let words = match case.size {
        Size::Unread => return None,
        Size::Loading => {
            return Some(
                view! {
                    <span class="g-ops-line">
                        <SkeletonText width="176px" />
                    </span>
                }
                .into_any(),
            );
        }
        Size::Known { .. } if case.files == 0 => "No files".to_string(),
        Size::Known { total, here } if here == total => bytes(total),
        Size::Known { total, here: 0 } => format!("Nothing downloaded of {}", bytes(total)),
        Size::Known { total, here } => format!("{} downloaded of {}", bytes(here), bytes(total)),
    };
    Some(view! { <span class="g-ops-line">{words}</span> }.into_any())
}

/// Keeping's count, as the pane says it today.
fn counted(case: Case) -> String {
    if case.present == case.files {
        "All files are downloaded".to_string()
    } else {
        format!(
            "{} of {} downloaded",
            count(case.present),
            count(case.files)
        )
    }
}

/// P2's count with the size joined to it. An empty revision says so instead of
/// "All files are downloaded", which is true of nothing.
fn joined(case: Case) -> String {
    match case.size {
        Size::Known { .. } if case.files == 0 => "This revision has no files".to_string(),
        Size::Known { total, .. } if case.present == case.files => {
            format!("All files are downloaded · {}", bytes(total))
        }
        Size::Known { total, .. } if case.present == 0 => {
            format!("No files are downloaded · {}", bytes(total))
        }
        Size::Known { total, here } => format!(
            "{} of {} files · {} of {} downloaded",
            count(case.present),
            count(case.files),
            bytes(here),
            bytes(total),
        ),
        Size::Loading | Size::Unread => counted(case),
    }
}

/// The caption's sentence, with the promise only the whole package makes.
fn sentence(case: Case, body: &str) -> String {
    if case.scope == "all" {
        format!("{body} — files added later are downloaded too.")
    } else {
        format!("{body}.")
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Placement {
    Revision,
    Keeping,
}

/// The pane, at 280px, with the size wherever `at` puts it.
fn pane(case: Case, at: Placement) -> AnyView {
    let selected = RwSignal::new(case.scope.to_string());
    let line = (at == Placement::Revision)
        .then(|| revision_line(case))
        .flatten();
    let caption = if at == Placement::Keeping {
        joined(case)
    } else {
        counted(case)
    };
    // The bar under the caption, standing in for the clause it will grow; see
    // the module comment for why it is not in it.
    let pending = (at == Placement::Keeping && matches!(case.size, Size::Loading))
        .then(|| view! { <SkeletonText width="200px" /> });
    view! {
        <aside aria-label="About this package" class="g-ops-pane">
            <Card label="About this package">
                <PaneSection label="Revision">
                    <RevisionRow message="Normalize well IDs" at=ago(20.0 * MINUTE) />
                    <span class="g-ops-line">{BUCKET}</span>
                    {line}
                    <Button on_click=|_| ()>"Revisions you have (6)"</Button>
                </PaneSection>
                <PaneSection>
                    <div class="g-ops-keeping">
                        <ChoiceGroup
                            label="Keeping"
                            caption=sentence(case, &caption)
                            options=vec![
                                Choice::new("pick", "Files I pick"),
                                Choice::new("all", "The whole package"),
                            ]
                            selected=selected
                        />
                        {pending}
                    </div>
                </PaneSection>
            </Card>
        </aside>
    }
    .into_any()
}

/// The revisions surface, held still: the rows and the footer the "Remove old
/// revisions" scene draws, in that scene's own classes, so this copy follows
/// its styling rather than restating it.
fn removal_surface() -> AnyView {
    let rows: [(&str, f64, Option<&str>, &str); 6] = [
        (
            "Normalize well IDs",
            20.0 * MINUTE,
            None,
            "current · not pushed",
        ),
        (
            "Re-run plate 7 with the corrected layout",
            2.0 * HOUR,
            Some("c41d8f"),
            "latest · base",
        ),
        ("Add plate 6 controls", DAY, Some("7be0c2"), "frees < 1 MB"),
        (
            "Add Caihong folder-upload note",
            3.0 * DAY,
            Some("b5e013"),
            "frees 1.2 MB",
        ),
        ("Initial upload", 12.0 * DAY, Some("06e3ad"), "frees 4.8 MB"),
        ("", 20.0 * DAY, Some("9a2b71"), "frees nothing"),
    ];
    let rows = rows
        .into_iter()
        .enumerate()
        .map(|(i, (message, at, hash, note))| {
            // The first two are kept, and hold the trash's width empty.
            let action = if i < 2 {
                view! { <span class="g-ori-gap" aria-hidden="true"></span> }.into_any()
            } else {
                view! {
                    <IconButton
                        icon=trash()
                        aria_label="Remove this revision"
                        variant=IconButtonVariant::Invisible
                        on_click=|_| ()
                    />
                }
                .into_any()
            };
            view! {
                <div class="g-ori-row">
                    <div class="g-ori-line">
                        <RevisionRow
                            message=message
                            at=ago(at)
                            published=hash.is_some()
                            catalog=hash.map(catalog)
                        />
                        {action}
                    </div>
                    <span class="g-ori-note">{note}</span>
                </div>
            }
        })
        .collect_view();
    view! {
        <div class="g-ori-surface">
            <div class="g-ori-body">
                <PaneSection>
                    <div class="g-ori-rows">{rows}</div>
                </PaneSection>
                <PaneSection>
                    <div class="g-ori-footer">
                        <Button on_click=|_| ()>
                            "Remove 4 older · frees 6.9 MB"
                        </Button>
                    </div>
                </PaneSection>
            </div>
        </div>
    }
    .into_any()
}

fn catalog(hash: &str) -> CatalogLink {
    CatalogLink::new(
        format!("https://quilt-lab.example/b/quilt-lab-plates/packages/{NAMESPACE}/tree/{hash}/"),
        Callback::new(|_url: String| ()),
    )
}

/// Octicons' `trash-16`, as the "Remove old revisions" scene draws it until the
/// kit has it. MIT, © GitHub Inc., as `kit/icons.rs` reproduces.
fn trash() -> AnyView {
    view! {
        <svg viewBox="0 0 16 16" aria-hidden="true" fill="currentColor">
            <path d="M11 1.75V3h2.25a.75.75 0 0 1 0 1.5H2.75a.75.75 0 0 1 0-1.5H5V1.75C5 .784 5.784 0 6.75 0h2.5C10.216 0 11 .784 11 1.75ZM4.496 6.675l.66 6.6a.25.25 0 0 0 .249.225h5.19a.25.25 0 0 0 .249-.225l.66-6.6a.75.75 0 0 1 1.492.149l-.66 6.6A1.748 1.748 0 0 1 10.595 15h-5.19a1.75 1.75 0 0 1-1.741-1.575l-.66-6.6a.75.75 0 1 1 1.492-.15ZM6.5 1.75V3h3V1.75a.25.25 0 0 0-.25-.25h-2.5a.25.25 0 0 0-.25.25Z" />
        </svg>
    }
    .into_any()
}

/// The surface where the popover hangs, left of the pane, over the file list.
fn beside_removal() -> AnyView {
    view! {
        <div class="g-ops-page">
            <div class="g-ops-files">"the file list"</div>
            {removal_surface()}
            {pane(SOME_HERE, Placement::Keeping)}
        </div>
    }
    .into_any()
}

const NOTE: &str = "The package's size: the sum of the current revision's manifest rows, and of \
    the rows whose files are here. It is the logical size, not disk usage — on APFS the object \
    store shares blocks with the working files, so what the package takes on disk is less, and \
    moves with nothing the reader did. The pane's words say \"downloaded\", about files; only \
    the revisions surface's \"frees\" is about this computer's disk. \
    \
    P2, joined to Keeping's count, is the recommendation: the count and the size answer one \
    question, how much of this package is here, and the choice above them moves both. P1's bare \
    figure under the bucket reads as the bucket's size, and sits on the trigger whose surface \
    speaks only of freeing space. \
    \
    A size that cannot be read says nothing: P2 keeps today's count, P1 drops its line. The \
    size comes with the page's own read, so its skeleton is what a separate fetch would show.";

#[component]
pub fn PackageSizeScene() -> impl IntoView {
    use Placement::{Keeping, Revision};
    view! {
        <Scene title="Package size" note=NOTE>
            <Cell wide=true label="P2, recommended — all downloaded">
                {pane(ALL_HERE, Keeping)}
            </Cell>
            <Cell wide=true label="P2 — some downloaded">
                {pane(SOME_HERE, Keeping)}
            </Cell>
            <Cell wide=true label="P2 — none downloaded: files I pick, nothing picked">
                {pane(NONE_HERE, Keeping)}
            </Cell>
            <Cell wide=true label="P2 — an empty package, no files">
                {pane(EMPTY, Keeping)}
            </Cell>
            <Cell wide=true label="P2 — a huge package, 1.2 TB in 140,000 files">
                {pane(HUGE, Keeping)}
            </Cell>
            <Cell wide=true label="P2 — size still loading">
                {pane(LOADING, Keeping)}
            </Cell>
            <Cell wide=true label="P2 — size could not be read: the count alone">
                {pane(UNREAD, Keeping)}
            </Cell>
            <Cell wide=true label="P1 — all downloaded">
                {pane(ALL_HERE, Revision)}
            </Cell>
            <Cell wide=true label="P1 — some downloaded">
                {pane(SOME_HERE, Revision)}
            </Cell>
            <Cell wide=true label="P1 — none downloaded: files I pick, nothing picked">
                {pane(NONE_HERE, Revision)}
            </Cell>
            <Cell wide=true label="P1 — an empty package, no files">
                {pane(EMPTY, Revision)}
            </Cell>
            <Cell wide=true label="P1 — a huge package, 1.2 TB in 140,000 files">
                {pane(HUGE, Revision)}
            </Cell>
            <Cell wide=true label="P1 — size still loading">
                {pane(LOADING, Revision)}
            </Cell>
            <Cell wide=true label="P1 — size could not be read: no line">
                {pane(UNREAD, Revision)}
            </Cell>
            <Cell
                full=true
                label="P2 beside the revisions surface — the pane says what of the package is \
                       downloaded, in files and bytes; the surface says what removing frees on \
                       this disk. No disk word on the pane, no size word on the surface, so 6.9 MB \
                       freed beside a 3.4 MB package reads as two measures"
            >
                {beside_removal()}
            </Cell>
        </Scene>
    }
}

#[cfg(test)]
mod tests {
    use super::{EMPTY, HUGE, NONE_HERE, SOME_HERE, UNREAD, bytes, count, joined};

    /// The words as read, with the no-break spaces shown as spaces.
    fn plain(words: &str) -> String {
        words.replace('\u{a0}', " ")
    }

    #[test]
    fn sizes_take_one_decimal_in_the_largest_unit() {
        assert_eq!(plain(&bytes(0)), "0 B");
        assert_eq!(plain(&bytes(999)), "999 B");
        assert_eq!(plain(&bytes(1_900_000)), "1.9 MB");
        assert_eq!(plain(&bytes(999_960)), "1.0 MB");
        assert_eq!(plain(&bytes(1_200_000_000_000)), "1.2 TB");
    }

    #[test]
    fn counts_are_grouped() {
        assert_eq!(count(56), "56");
        assert_eq!(count(12_400), "12,400");
        assert_eq!(count(140_000), "140,000");
    }

    #[test]
    fn the_caption_joins_size_to_count() {
        assert_eq!(
            plain(&joined(SOME_HERE)),
            "54 of 56 files · 1.9 MB of 3.4 MB downloaded"
        );
        assert_eq!(
            plain(&joined(NONE_HERE)),
            "No files are downloaded · 3.4 MB"
        );
        assert_eq!(plain(&joined(EMPTY)), "This revision has no files");
        assert_eq!(
            plain(&joined(HUGE)),
            "12,400 of 140,000 files · 86.3 GB of 1.2 TB downloaded"
        );
        assert_eq!(plain(&joined(UNREAD)), "54 of 56 downloaded");
    }
}
