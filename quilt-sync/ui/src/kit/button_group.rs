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
//! is. While it is true the tooltip says the done words, and it opens when the
//! signal turns true, so a click that copied something says so under the
//! pointer. A status region says it to a screen reader.
//!
//! [`Tooltip`]: super::Tooltip

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::Align;
use super::TooltipHandle;
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
    /// What the tooltip says while `done` is true, and the signal itself.
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

    /// Says `words` while `done` is true.
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
    let text = {
        let label = label.clone();
        let done_words = done.as_ref().map(|(words, _)| words.clone());
        Signal::derive(move || match (&done_words, is_done) {
            (Some(words), Some(signal)) if signal.get() => words.clone(),
            _ => label.clone(),
        })
    };

    // Opens on the rise only: a cell drawn already done waits for a hover.
    if let Some(signal) = is_done {
        Effect::new(move |was: Option<bool>| {
            let now = signal.get();
            if now && was == Some(false) {
                handle.show();
            }
            now
        });
    }

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
                aria-label=label
                disabled=move || disabled.get()
                on:click=move |_| {
                    on_click.run(());
                    // A repeat click while still done has no rise to open on,
                    // and its pointerdown just closed the words.
                    if is_done.is_some_and(|signal| signal.get_untracked()) {
                        handle.show();
                    }
                }
            >
                {icon()}
                {labelled.then(|| view! { <span class=style::label node_ref=words>{shown}</span> })}
            </button>
            {tooltip::surface(handle, text, Signal::default(), anchor, Align::End, tooltip::style::surface.to_string())}
            {is_done
                .map(|signal| {
                    view! {
                        <span role="status" data-sr-only>
                            {move || signal.get().then(|| text.get())}
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

    /// A click runs the segment's own command, and the done words replace the
    /// name in the tooltip and the status region while the caller's signal is
    /// true.
    #[wasm_bindgen_test]
    async fn done_words_follow_the_caller_s_signal() {
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
                .text_content()
                .unwrap()
        };
        let status = || {
            el.query_selector("[role='status']")
                .unwrap()
                .unwrap()
                .text_content()
                .unwrap()
        };
        assert_eq!(tooltip(), "Copy Quilt URI");
        assert_eq!(status(), "");

        buttons(&el)[0].click();
        leptos::task::tick().await;
        assert_eq!(tooltip(), "Copied");
        assert_eq!(status(), "Copied");

        copied.set(false);
        leptos::task::tick().await;
        assert_eq!(tooltip(), "Copy Quilt URI");
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
