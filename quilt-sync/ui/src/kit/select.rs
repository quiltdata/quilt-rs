//! Single-choice control over a native `<select>`.
//!
//! §2 of the design record eliminates the tooltip/dropdown/combobox class by
//! committing to this: our closed state, the OS's open dropdown. It holds only
//! while every choice is a plain string — a choice needing rich formatting means
//! the control is wrong, and the answer is an inline list or radio group.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/select.module.scss");

#[component]
pub fn Select(
    /// Accessible name, and the visible prefix when `visible_label` is set.
    /// Always rendered — off-screen when not shown — so the `<label>` names the
    /// control from markup in both configurations and there is no anonymous
    /// select.
    #[prop(into)]
    label: String,
    /// Rendered in order. A single-option select is a dead control; callers
    /// check first (`role_switch_host` is `Some` only for multi-role hosts).
    options: Vec<String>,
    selected: RwSignal<String>,
    /// Show the label as a `Name:` prefix inside the control.
    #[prop(optional)]
    visible_label: bool,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
) -> impl IntoView {
    let is_disabled = Signal::derive(move || disabled.get().unwrap_or(false));

    let class = move || {
        let mut out = String::from(style::root);
        if is_disabled.get() {
            out.push(' ');
            out.push_str(style::disabled);
        }
        out
    };

    let (label_class, label_text) = if visible_label {
        (style::prefix, format!("{label}:"))
    } else {
        (style::offscreen, label)
    };

    view! {
        <label class=class>
            <span class=label_class>{label_text}</span>
            <span class=style::value>{move || selected.get()}</span>
            <span class=style::caret aria-hidden="true">
                <svg viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5"
                    stroke-linecap="round" stroke-linejoin="round">
                    <path d="M3 4.75 6 7.75l3-3" />
                </svg>
            </span>
            <select
                class=style::field
                disabled=move || is_disabled.get()
                prop:value=move || selected.get()
                on:change=move |ev| selected.set(event_target_value(&ev))
            >
                {options
                    .into_iter()
                    .map(|option| {
                        let text = option.clone();
                        view! { <option value=option>{text}</option> }
                    })
                    .collect_view()}
            </select>
        </label>
    }
}
