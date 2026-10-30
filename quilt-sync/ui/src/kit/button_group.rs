//! Related commands joined into one control, such as a package's local folder
//! beside its catalog page.
//!
//! # Labels give way before the row does
//!
//! A segment drawn with a label shows it while its container is wider than
//! 800px, the width where the installed-package page stacks its panes. Below
//! that every segment is its icon, and its name moves to a [`Tooltip`]. The
//! measure is the nearest `inline-size` container's, not the window's, so a
//! gallery cell can show either arrangement.
//!
//! A segment whose label is showing opens no tooltip: the words would repeat
//! what is already drawn.
//!
//! # Feedback
//!
//! A segment can carry a `done` signal, owned by the caller as the clipboard
//! is. While it is true the segment's icon is a check, at the same size so the
//! segment keeps its width, and a status region says the done words to a
//! screen reader. The tooltip keeps the segment's name and does not open by
//! itself: the check under the pointer is the answer.
//!
//! [`Tooltip`]: super::Tooltip

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::Align;
use super::TooltipHandle;
use super::icons;
use super::tooltip;

stylance::import_crate_style!(style, "src/kit/button_group.module.scss");

/// One segment of a [`ButtonGroup`].
#[derive(Clone)]
pub struct GroupButton {
    /// A glyph from [`icons`](super::icons).
    pub icon: fn() -> AnyView,
    /// The accessible name, always; drawn beside the icon too when `labelled`.
    pub label: String,
    /// Draws the label while the group has room.
    pub labelled: bool,
    pub on_click: Callback<()>,
    /// What the status region says while `done` is true, and the signal itself.
    pub done: Option<(String, Signal<bool>)>,
    pub disabled: Signal<bool>,
}

impl GroupButton {
    /// An icon-only segment.
    #[must_use]
    pub fn icon(icon: fn() -> AnyView, label: impl Into<String>, on_click: Callback<()>) -> Self {
        Self {
            icon,
            label: label.into(),
            labelled: false,
            on_click,
            done: None,
            disabled: Signal::stored(false),
        }
    }

    /// A segment that shows its label while there is room.
    #[must_use]
    pub fn labelled(
        icon: fn() -> AnyView,
        label: impl Into<String>,
        on_click: Callback<()>,
    ) -> Self {
        Self {
            labelled: true,
            ..Self::icon(icon, label, on_click)
        }
    }

    /// Draws a check and says `words` while `done` is true.
    #[must_use]
    pub fn with_done(self, words: impl Into<String>, done: Signal<bool>) -> Self {
        Self {
            done: Some((words.into(), done)),
            ..self
        }
    }

    /// Refuses clicks while `disabled` is true.
    #[must_use]
    pub fn with_disabled(self, disabled: Signal<bool>) -> Self {
        Self { disabled, ..self }
    }
}

#[component]
pub fn ButtonGroup(
    /// In reading order.
    buttons: Vec<GroupButton>,
    /// Names the group, such as `Package location`.
    #[prop(into)]
    aria_label: String,
) -> impl IntoView {
    view! {
        <div class=style::root role="group" aria-label=aria_label>
            {buttons.into_iter().map(segment).collect_view()}
        </div>
    }
}

fn segment(button: GroupButton) -> impl IntoView {
    let GroupButton {
        icon,
        label,
        labelled,
        on_click,
        done,
        disabled,
    } = button;
    let handle = TooltipHandle::new();
    let wrapper: NodeRef<leptos::html::Span> = NodeRef::new();
    let words: NodeRef<leptos::html::Span> = NodeRef::new();
    let anchor = Signal::derive_local(move || wrapper.get().map(JsCast::unchecked_into));
    let shown = label.clone();

    let is_done = done.as_ref().map(|(_, signal)| *signal);
    let done_now = move || is_done.is_some_and(|signal| signal.get());
    let text = Signal::stored(label.clone());

    // The label is `display: none` in the narrow arrangement, which leaves it
    // no box.
    let label_hidden = move || {
        !labelled
            || words
                .get_untracked()
                .is_none_or(|el| el.offset_width() == 0)
    };

    view! {
        <span
            class=style::segment
            node_ref=wrapper
            on:pointerenter=move |event| {
                if label_hidden() {
                    handle.pointer_enter(&event);
                }
            }
            on:pointerleave=move |_| handle.leave()
            on:focusin=move |event| {
                if label_hidden() {
                    handle.focus_in(&event);
                }
            }
            on:focusout=move |_| handle.focus_out()
        >
            <button
                type="button"
                class=style::button
                class=(style::done, done_now)
                aria-label=label
                disabled=move || disabled.get()
                on:click=move |_| on_click.run(())
            >
                {move || if done_now() { icons::check() } else { icon() }}
                {labelled.then(|| view! { <span class=style::label node_ref=words>{shown}</span> })}
            </button>
            {tooltip::surface(handle, text, Signal::default(), anchor, Align::End, tooltip::style::surface.to_string())}
            {done
                .map(|(words, signal)| {
                    view! {
                        <span role="status" data-sr-only>
                            {move || signal.get().then(|| words.clone())}
                        </span>
                    }
                })}
        </span>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::icons;
    use crate::test_support::mount;
    use wasm_bindgen_test::*;

    fn buttons(root: &web_sys::Element) -> Vec<web_sys::HtmlElement> {
        let list = root.query_selector_all("button").unwrap();
        (0..list.length())
            .filter_map(|i| list.item(i))
            .map(|n| n.dyn_into::<web_sys::HtmlElement>().unwrap())
            .collect()
    }

    /// Every segment keeps its name when its label is hidden, and carries no
    /// `title` to cover the tooltip.
    #[wasm_bindgen_test]
    fn every_segment_is_named_without_a_title() {
        let el = mount(|| {
            view! {
                <ButtonGroup
                    aria_label="Package location"
                    buttons=vec![
                        GroupButton::labelled(icons::folder, "Open folder", Callback::new(|()| ())),
                        GroupButton::icon(icons::copy, "Copy Quilt URI", Callback::new(|()| ())),
                    ]
                />
            }
        });
        let names: Vec<_> = buttons(&el)
            .iter()
            .map(|b| {
                assert!(!b.has_attribute("title"));
                b.get_attribute("aria-label").unwrap()
            })
            .collect();
        assert_eq!(names, ["Open folder", "Copy Quilt URI"]);
    }

    /// A click runs the segment's own command. While the caller's signal is
    /// true the icon is a check and the status region says the done words;
    /// the tooltip keeps the name and stays shut.
    #[wasm_bindgen_test]
    async fn done_draws_a_check_and_leaves_the_tooltip_shut() {
        let copied = RwSignal::new(false);
        let el = mount(move || {
            view! {
                <ButtonGroup
                    aria_label="Package location"
                    buttons=vec![
                        GroupButton::icon(
                            icons::copy,
                            "Copy Quilt URI",
                            Callback::new(move |()| copied.set(true)),
                        )
                        .with_done("Copied", copied.into()),
                    ]
                />
            }
        });
        let tooltip = || {
            el.query_selector("[role='tooltip']")
                .unwrap()
                .unwrap()
                .dyn_into::<web_sys::HtmlElement>()
                .unwrap()
        };
        let status = || {
            el.query_selector("[role='status']")
                .unwrap()
                .unwrap()
                .text_content()
                .unwrap()
        };
        // The first path of each Octicon tells the two apart.
        let glyph = || {
            buttons(&el)[0]
                .query_selector("svg path")
                .unwrap()
                .expect("a glyph")
                .get_attribute("d")
                .unwrap_or_default()
        };
        let (copy_glyph, check_glyph) = ("M0 6.75", "M13.78 4.22");
        assert!(glyph().starts_with(copy_glyph));
        assert_eq!(status(), "");

        buttons(&el)[0].click();
        leptos::task::tick().await;
        assert!(glyph().starts_with(check_glyph));
        assert_eq!(status(), "Copied");
        assert_eq!(tooltip().text_content().unwrap(), "Copy Quilt URI");
        assert!(
            !tooltip().matches(":popover-open").unwrap_or(false),
            "the check is the answer; nothing pops"
        );
        assert_eq!(
            buttons(&el)[0].get_attribute("aria-label").as_deref(),
            Some("Copy Quilt URI")
        );

        copied.set(false);
        leptos::task::tick().await;
        assert!(glyph().starts_with(copy_glyph));
        assert_eq!(status(), "");
    }

    /// A disabled segment follows its signal and leaves its neighbours live.
    #[wasm_bindgen_test]
    async fn a_segment_disables_on_its_own_signal() {
        let busy = RwSignal::new(false);
        let el = mount(move || {
            view! {
                <ButtonGroup
                    aria_label="Package location"
                    buttons=vec![
                        GroupButton::labelled(icons::folder, "Open folder", Callback::new(|()| ()))
                            .with_disabled(busy.into()),
                        GroupButton::icon(icons::copy, "Copy Quilt URI", Callback::new(|()| ())),
                    ]
                />
            }
        });
        let disabled = || {
            buttons(&el)
                .iter()
                .map(|b| b.has_attribute("disabled"))
                .collect::<Vec<_>>()
        };
        assert_eq!(disabled(), [false, false]);
        busy.set(true);
        leptos::task::tick().await;
        assert_eq!(disabled(), [true, false]);
    }
}
