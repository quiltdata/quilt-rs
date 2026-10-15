//! One recently-changed file.
//!
//! The row opens the file; the package tag navigates to its package. Both are
//! primary enough to be one click, which is why the tag is a real link and the row
//! is not an anchor.
//!
//! The rest of what can be done with the file sits behind one `[⋯]`, as on the
//! package page's file rows.
//!
//! The path and the tag carry no `title`. Each marks itself with
//! `data-full-name`, and the list's [`NameTooltip`](super::NameTooltip) shows
//! the whole value when the ellipsis cut it. The packages view's rows have none: a file is a
//! thing you act on, a package is a place you go.

use leptos::ev::MouseEvent;
use leptos::prelude::*;

use super::ActionMenu;
use super::MenuAction;
use super::RelativeTime;
use super::SkeletonBox;
use super::countdown::EpochMillis;

stylance::import_crate_style!(style, "src/kit/file_row.module.scss");

/// The check the time column shows once the row's address is on the
/// clipboard. Drawn in strokes, to sit with the row's other text rather than
/// read as a filled badge.
fn copied_icon() -> AnyView {
    view! {
        <svg viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor"
            stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
            <path d="M3.5 8.5 6.5 11.5 12.5 4.5" />
        </svg>
    }
    .into_any()
}

#[component]
pub fn FileRow(
    /// Logical key, shown whole and truncated from the left when it will not fit.
    #[prop(into)]
    path: String,
    #[prop(into)] package: String,
    #[prop(into)] package_href: String,
    at: EpochMillis,
    /// Opens the file in whatever application the OS associates with it. Bound to
    /// the row, not to a button: a button that duplicated the row's own affordance
    /// would be chrome.
    on_open: impl Fn(MouseEvent) + 'static,
    /// The `[⋯]`'s items. The caller decides which apply to the file.
    #[prop(into)]
    actions: Signal<Vec<MenuAction>>,
    /// Whether this row's address is the one currently on the clipboard.
    ///
    /// The caller's answer, not this row's: only a copy that actually landed may
    /// say so, and the row cannot know. The time column says *Copied* while it
    /// holds, so the menu that started the copy can close without the copy
    /// going unseen. It is hidden from assistive tech, which hears the card's
    /// live region instead (`pages/main_page/recent_files.rs`).
    #[prop(optional, into)]
    copied: MaybeProp<bool>,
) -> impl IntoView {
    // Both ellipsise, so each hands its whole value to the list's
    // `NameTooltip`, which shows it only when it is cut.
    let full_path = path.clone();
    let full_package = package.clone();
    let time = move || {
        if copied.get().unwrap_or(false) {
            view! {
                <span class=style::copied aria-hidden="true">
                    {copied_icon()}
                    "Copied"
                </span>
            }
            .into_any()
        } else {
            view! { <RelativeTime at=at /> }.into_any()
        }
    };
    view! {
        // A plain container, not `role="button"`: WAI-ARIA marks `button` as
        // Children Presentational, and assistive tech may prune the link and the
        // menu inside it. `on:click` is a pointer convenience, so the whole row is
        // a target for a mouse; the path below is the control that owns opening.
        <div class=style::root on:click=on_open>
            // The open control, and a real button: it carries the file's name, which
            // is what a reader needs to hear before choosing to open it. No handler
            // of its own: a native button turns Enter and Space into a click, and
            // that click bubbles to the row above, which is the one handler.
            <button type="button" class=style::path data-full-name=full_path>{path}</button>
            // Stops propagation, or going to the package would also open the file.
            <a
                class=style::tag
                data-full-name=full_package
                href=package_href
                on:click=|ev: MouseEvent| ev.stop_propagation()
            >
                {package}
            </a>
            <span class=style::time>{time}</span>
            // The menu's surface is drawn inside this span, so a click on an item
            // stops here too and does not also open the file.
            <span class=style::actions on:click=|ev: MouseEvent| ev.stop_propagation()>
                <ActionMenu aria_label="More actions for this file" actions=actions />
            </span>
        </div>
    }
}

/// A file row's shape while the feed is being read.
///
/// Without it, Recent files shows an empty region until the read answers,
/// while the packages view shows skeleton rows in the same place. Page chrome
/// gets no skeleton, but both lists do.
///
/// Three boxes, not four: the path, the package tag and the time take up most
/// of a row. The `[⋯]` is one fixed glyph, and its brief absence does not look
/// like missing content.
#[component]
pub fn FileRowSkeleton() -> impl IntoView {
    view! {
        <div class=format!("{} {}", style::root, style::skeleton)>
            <span class=style::path>
                <SkeletonBox width="52%" />
            </span>
            // Fixed widths, like `PackageRowSkeleton`'s state label: a tag is as
            // wide as the package it names and the time as wide as its words, so
            // a percentage of the row would be describing the wrong thing.
            <SkeletonBox width="96px" height="18px" />
            <span class=style::time>
                <SkeletonBox width="100%" />
            </span>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        blur, button_saying, element_saying, focus_report, keyboard_focus, mount, sleep_ms,
        unmount_earlier,
    };
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    const TRIGGER: &str = "button[aria-label='More actions for this file']";

    fn click(el: &web_sys::Element) {
        el.clone().unchecked_into::<web_sys::HtmlElement>().click();
    }

    /// Two items in two groups, each counting its own presses.
    fn actions(reveals: RwSignal<u32>, uris: RwSignal<u32>) -> Vec<MenuAction> {
        vec![
            MenuAction::new(
                "Reveal in folder",
                Callback::new(move |()| reveals.update(|n| *n += 1)),
            ),
            MenuAction::new(
                "Copy URI",
                Callback::new(move |()| uris.update(|n| *n += 1)),
            )
            .separated(),
        ]
    }

    fn row(opened: RwSignal<u32>, copied: Signal<bool>) -> web_sys::Element {
        let (reveals, uris) = (RwSignal::new(0), RwSignal::new(0));
        mount(move || {
            view! {
                <FileRow
                    path="data/one.csv"
                    package="user/alpha"
                    package_href="/installed-package?namespace=user/alpha"
                    at=0.0
                    on_open=move |_| opened.update(|n| *n += 1)
                    actions=actions(reveals, uris)
                    copied=copied
                />
            }
        })
    }

    /// The row's one control besides the path and the chip is its `[⋯]`, and
    /// the menu draws what it is handed, in its groups.
    #[wasm_bindgen_test]
    async fn the_row_s_actions_sit_behind_one_menu() {
        let el = row(RwSignal::new(0), Signal::stored(false));
        let labelled = el.query_selector_all("button[aria-label]").unwrap();
        assert_eq!(labelled.length(), 1, "one trigger, no bare icons");

        click(&el.query_selector(TRIGGER).unwrap().expect("the [⋯]"));
        leptos::task::tick().await;
        let items = el.query_selector_all("[popover] button").unwrap();
        let words: Vec<String> = (0..items.length())
            .map(|i| items.item(i).unwrap().text_content().unwrap_or_default())
            .collect();
        assert_eq!(words, ["Reveal in folder", "Copy URI"]);
        assert_eq!(
            el.query_selector_all("[popover] [role='separator']")
                .unwrap()
                .length(),
            1
        );
    }

    /// Opening the menu and choosing from it never also opens the file.
    #[wasm_bindgen_test]
    async fn the_menu_does_not_open_the_file() {
        let opened = RwSignal::new(0);
        let el = row(opened, Signal::stored(false));
        click(&el.query_selector(TRIGGER).unwrap().expect("the [⋯]"));
        leptos::task::tick().await;
        click(
            &el.query_selector("[popover] button")
                .unwrap()
                .expect("an item"),
        );
        leptos::task::tick().await;
        assert_eq!(opened.get_untracked(), 0);

        click(
            &el.query_selector("[class*=path]")
                .unwrap()
                .expect("the path"),
        );
        assert_eq!(opened.get_untracked(), 1, "and the path still does");
    }

    /// The path is the open control and the row is not a button.
    ///
    /// `button` is Children Presentational in WAI-ARIA, so a `role="button"`
    /// root would let assistive tech prune the link and the menu inside it.
    #[wasm_bindgen_test]
    fn the_row_is_not_a_button_and_the_path_is() {
        let el = row(RwSignal::new(0), Signal::stored(false));
        assert!(
            el.query_selector("[role=button]").unwrap().is_none(),
            "nothing here claims a role it cannot keep"
        );
        let path = el
            .query_selector("[class*=path]")
            .unwrap()
            .expect("the path");
        assert_eq!(
            path.tag_name(),
            "BUTTON",
            "the platform owns the keyboard, so the control is a real one"
        );
    }

    /// The path and the tag both truncate; each hands its whole value to the
    /// list's tooltip, and neither carries a `title` for the browser to draw.
    #[wasm_bindgen_test]
    fn the_path_and_the_package_tag_carry_their_whole_values() {
        let el = mount(|| {
            view! {
                <FileRow
                    path=PATH
                    package=PACKAGE
                    package_href="/installed-package"
                    at=0.0
                    on_open=|_| {}
                    actions=Vec::new()
                />
            }
        });
        assert_eq!(
            button_saying(&el, PATH)
                .get_attribute("data-full-name")
                .as_deref(),
            Some(PATH)
        );
        assert_eq!(
            element_saying(&el, PACKAGE)
                .get_attribute("data-full-name")
                .as_deref(),
            Some(PACKAGE)
        );
        assert!(
            el.query_selector("[data-full-name][title]")
                .unwrap()
                .is_none()
        );
    }

    const PATH: &str = "data/deep/nested/directory/with/a/long/name/measurements.csv";
    const PACKAGE: &str = "a-long-owner-name/a-much-longer-package-name-than-the-column";

    /// A row 360px wide inside a list's `NameTooltip`, with its stylesheet, so
    /// the path and the tag are both cut as in the app.
    fn cut_row() -> web_sys::Element {
        crate::test_support::load_module_style(
            include_str!("file_row.module.scss"),
            &[
                ("root", style::root),
                ("path", style::path),
                ("tag", style::tag),
                ("time", style::time),
                ("actions", style::actions),
            ],
        );
        mount(|| {
            view! {
                <div style="width:360px">
                    <crate::kit::NameTooltip>
                        <FileRow
                            path=PATH
                            package=PACKAGE
                            package_href="/installed-package"
                            at=0.0
                            on_open=|_| {}
                            actions=Vec::new()
                        />
                    </crate::kit::NameTooltip>
                </div>
            }
        })
    }

    fn shown(el: &web_sys::Element) -> Option<String> {
        let tip = el
            .query_selector("[role='tooltip']")
            .unwrap()
            .expect("the surface");
        tip.matches(":popover-open")
            .unwrap()
            .then(|| tip.text_content().unwrap_or_default())
    }

    #[wasm_bindgen_test]
    async fn a_cut_path_shows_whole_on_hover() {
        unmount_earlier();
        let el = cut_row();
        let path = button_saying(&el, PATH);
        assert!(path.scroll_width() > path.client_width(), "the path is cut");
        let init = web_sys::PointerEventInit::new();
        init.set_pointer_type("mouse");
        init.set_bubbles(true);
        let over = web_sys::PointerEvent::new_with_event_init_dict("pointerover", &init).unwrap();
        path.dispatch_event(&over).unwrap();
        sleep_ms(600).await;
        assert_eq!(shown(&el).as_deref(), Some(PATH));
        let out = web_sys::PointerEvent::new_with_event_init_dict("pointerout", &init).unwrap();
        path.dispatch_event(&out).unwrap();
        sleep_ms(400).await;
    }

    /// The tag is a link, so Tab reaches it, and its focus opens the package's
    /// whole name at once.
    #[wasm_bindgen_test]
    async fn keyboard_focus_on_a_cut_tag_shows_the_whole_package() {
        unmount_earlier();
        let el = cut_row();
        let tag = element_saying(&el, PACKAGE);
        assert!(tag.scroll_width() > tag.client_width(), "the tag is cut");
        keyboard_focus(&tag);
        leptos::task::tick().await;
        assert_eq!(
            shown(&el).as_deref(),
            Some(PACKAGE),
            "{}",
            focus_report(&tag)
        );
        blur(&tag);
    }

    /// A copy that landed shows in the time column while it holds, and the
    /// `[⋯]` that started it stays the same node, so focus stays on it.
    #[wasm_bindgen_test]
    async fn a_copied_row_says_so_in_place_of_its_time() {
        let copied = RwSignal::new(false);
        let el = row(RwSignal::new(0), copied.into());
        let time = || {
            el.query_selector("[class*=time]")
                .unwrap()
                .expect("the time column")
                .text_content()
                .unwrap_or_default()
        };
        let trigger = el.query_selector(TRIGGER).unwrap().expect("the [⋯]");
        assert!(!time().contains("Copied"));

        copied.set(true);
        leptos::task::tick().await;
        assert_eq!(time(), "Copied");
        assert!(
            el.query_selector("[class*=copied]")
                .unwrap()
                .expect("the mark")
                .get_attribute("aria-hidden")
                .is_some(),
            "the card's live region speaks for it"
        );
        let after = el.query_selector(TRIGGER).unwrap().expect("the [⋯], still");
        assert!(trigger.is_same_node(Some(&after)));

        copied.set(false);
        leptos::task::tick().await;
        assert!(
            !time().contains("Copied"),
            "a mark that stayed would outlive the clipboard"
        );
    }
}
