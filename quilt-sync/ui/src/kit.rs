//! The UI kit — hand-written components for the v2 main page.
//!
//! Named `kit` because v1 still owns `components`; when v1 retires, that module
//! goes with it and nothing here needs renaming.
//!
//! **Components here are presentation only:** props in, view out, callbacks for
//! anything with an effect. No `commands`, no `tauri`, no fetching. That is
//! enforced rather than encouraged — `gallery.rs` mounts this module in a plain
//! browser with no Tauri runtime, so a component that calls a command cannot
//! render, and therefore cannot be reviewed.
//!
//! Each component is a `<name>.rs` with its stylesheet beside it as
//! `<name>.module.scss` (see `Cargo.toml` for how those are bundled).
//!
//! **`@keyframes` names are global.** `stylance` hashes class names, not keyframe
//! names, and every module is concatenated into one stylesheet — so two components
//! defining `spin` would silently share whichever definition came last. Prefix every
//! keyframe with its component: `button-spin`, `icon-spin`, `countdown-sweep`,
//! `skeleton-box-pulse`, `spinner-rotate`.

/// A document-unique id, for an element that has to be pointed at.
///
/// Ids only have to be unique within a document, and this is a single-threaded wasm
/// document, so `Relaxed` is enough. The prefix keeps one caller's ids from colliding
/// with another's counter.
pub(crate) fn unique_id(prefix: &str) -> String {
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    format!("{prefix}-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

pub mod action_menu;
pub mod anchored_overlay;
pub mod back_link;
pub mod banner;
pub mod blankslate;
pub mod button;
pub mod card;
pub mod cause_row;
pub mod checkbox;
pub mod choice_group;
pub mod countdown;
pub mod dialog;
pub mod entry_group;
pub mod entry_row;
pub mod file_row;
pub mod form_control;
pub mod group_heading;
pub mod host_row;
pub mod icon_button;
pub mod icons;
pub mod list_toolbar;
pub mod load_failure;
pub mod package_row;
pub mod package_state;
pub mod page_layout;
pub mod pane_section;
pub mod queue_row;
pub mod relative_time;
pub mod revision_row;
pub mod search_input;
pub mod segmented_control;
pub mod select;
pub mod select_all;
pub mod skeleton_box;
pub mod spinner;
pub mod split_button;
pub mod state_label;
pub mod text_input;
pub mod toggle_row;
pub mod zero_line;

pub use action_menu::ActionMenu;
pub use action_menu::ActionTone;
pub use action_menu::MenuAction;
pub use anchored_overlay::Align;
pub use anchored_overlay::AnchoredOverlay;
pub use back_link::BackLink;
pub use banner::Banner;
pub use banner::BannerVariant;
pub use blankslate::Blankslate;
pub use button::Button;
pub use button::ButtonSize;
pub use button::ButtonVariant;
pub use card::Card;
pub use cause_row::CauseRow;
pub use checkbox::CheckState;
pub use checkbox::Checkbox;
pub use choice_group::Choice;
pub use choice_group::ChoiceGroup;
pub use countdown::Countdown;
pub use dialog::Dialog;
pub use entry_group::EntryGroup;
pub use entry_group::GroupSelection;
pub use entry_row::DIFFERS_TITLE;
pub use entry_row::EntryAction;
pub use entry_row::EntryRow;
pub use entry_row::EntrySelection;
pub use file_row::FileRow;
pub use file_row::FileRowSkeleton;
pub use form_control::ControlId;
pub use form_control::FormControl;
pub use form_control::Naming;
pub use group_heading::GroupHeading;
pub use host_row::HostRow;
pub use icon_button::IconButton;
pub use icon_button::IconButtonVariant;
pub use list_toolbar::ListToolbar;
pub use load_failure::LoadFailure;
pub use package_row::PackageRow;
pub use package_row::PackageRowSkeleton;
pub use package_state::PackageAction;
pub use package_state::PackageState;
pub use package_state::Rendered;
pub use package_state::Site;
pub use package_state::render;
pub use page_layout::PageLayout;
pub use pane_section::PaneSection;
pub use queue_row::QueueRow;
pub use queue_row::QueueRowSkeleton;
pub use queue_row::Remedy;
pub use relative_time::RelativeTime;
pub use revision_row::RevisionRow;
pub use search_input::SearchInput;
pub use segmented_control::SegmentedControl;
pub use select::Select;
pub use select_all::SelectAll;
pub use skeleton_box::SkeletonBox;
pub use spinner::Spinner;
pub use spinner::SpinnerVariant;
pub use split_button::SplitButton;
pub use split_button::SplitOption;
pub use state_label::StateLabel;
pub use state_label::StateTone;
pub use text_input::TextInput;
pub use toggle_row::ToggleRow;
pub use zero_line::ZeroLine;
pub use zero_line::ZeroLineSkeleton;

#[cfg(test)]
mod tests {
    /// Every `selector { body }` pair. Correct only for a flat stylesheet, which
    /// the kit's are; callers name the rules they want rather than sweep the file.
    fn rules(sheet: &str) -> impl Iterator<Item = (&str, &str)> {
        sheet.match_indices('{').filter_map(move |(open, _)| {
            let before = &sheet[..open];
            let start = before
                .rfind(['}', ';'])
                .map_or(0, |i| i + 1)
                .max(before.rfind("*/").map_or(0, |i| i + 2));
            let body = sheet[open + 1..].split('}').next()?;
            Some((before[start..].trim(), body))
        })
    }

    /// Whether a selector list picks out `class`, allowing for grouping and for
    /// compound selectors such as `.root.disabled .sublabel`.
    fn selects(selector: &str, class: &str) -> bool {
        selector
            .split(',')
            .filter_map(|one| one.split_whitespace().last())
            .any(|last| last == class)
    }

    /// The two elements read as sentences rather than labels carry a measure.
    ///
    /// The band is pinned, not the number: any cap inside 65-75ch passes, a cap in
    /// pixels does not.
    #[test]
    fn prose_is_capped_to_a_readable_measure() {
        const PROSE: [(&str, &str, &str); 3] = [
            ("banner", ".message", include_str!("kit/banner.module.scss")),
            (
                "toggle_row",
                ".sublabel",
                include_str!("kit/toggle_row.module.scss"),
            ),
            (
                "queue_row",
                ".detail",
                include_str!("kit/queue_row.module.scss"),
            ),
        ];

        for (component, class, sheet) in PROSE {
            let Some((_, rule)) = rules(sheet).find(|(selector, _)| selects(selector, class))
            else {
                panic!("{component} has no `{class}` rule")
            };
            let cap = rule
                .lines()
                .find_map(|line| line.trim().strip_prefix("max-width:"))
                .unwrap_or_else(|| {
                    panic!("{component}'s `{class}` is prose and has no measure: {rule}")
                })
                .trim()
                .trim_end_matches(';');
            let width: f32 = cap
                .strip_suffix("ch")
                .unwrap_or_else(|| {
                    panic!(
                        "{component}'s `{class}` caps at `{cap}`, which is not a character count"
                    )
                })
                .parse()
                .expect("a number of characters");
            assert!(
                (65.0..=75.0).contains(&width),
                "{component}'s `{class}` caps at {width}ch, outside the readable 65-75"
            );
        }
    }
    /// An ellipsis is a promise the box can end up narrower than its text, and a
    /// flex item does not get that for free: its automatic minimum size is its
    /// max-content width. Two ways to allow it — clear the minimum with
    /// `min-width: 0`, or clamp it with a `max-width`, which is what a file row's
    /// package tag does at 30%.
    ///
    /// With neither, the text never ellipsises: the item holds its full width and
    /// the row overflows its card instead. Measured at a 700px window before this
    /// test existed — a queue row's name held 485px against a 414px line.
    #[test]
    fn a_column_that_ellipsises_can_end_up_narrower_than_its_text() {
        const SHEETS: [(&str, &str); 4] = [
            ("queue_row", include_str!("kit/queue_row.module.scss")),
            ("package_row", include_str!("kit/package_row.module.scss")),
            ("file_row", include_str!("kit/file_row.module.scss")),
            ("cause_row", include_str!("kit/cause_row.module.scss")),
        ];

        for (component, sheet) in SHEETS {
            for (selector, body) in rules(sheet) {
                if !body.contains("text-overflow: ellipsis") {
                    continue;
                }
                assert!(
                    body.contains("min-width: 0") || body.contains("max-width:"),
                    "{component}'s `{selector}` ellipsises, but nothing lets it get \
                     narrower than its own text — it will push the row instead: \
                     {body}"
                );
            }
        }
    }

    /// A queue row's detail is engine prose, and a workflow rejection is a sentence
    /// followed by one indented line per broken rule. Collapsing the whitespace loses
    /// which rules those are.
    #[test]
    fn a_queue_row_detail_keeps_the_line_breaks_it_is_given() {
        const SHEET: &str = include_str!("kit/queue_row.module.scss");
        let rule = rules(SHEET)
            .find(|(selector, _)| selects(selector, ".detail"))
            .map(|(_, body)| body)
            .expect("a `.detail` rule");
        assert!(
            rule.contains("white-space: pre-wrap"),
            "the message's own newlines have to survive: {rule}"
        );
    }
}
