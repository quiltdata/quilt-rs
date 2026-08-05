//! A labelled checkbox row with a sub-label and a trailing slot.
//!
//! The two Autosync toggles are the first callers; Settings has five sections of
//! the same shape.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/toggle_row.module.scss");

#[component]
pub fn ToggleRow(
    #[prop(into)] label: String,
    /// One line of explanation under the label. Wraps rather than truncating —
    /// it explains a setting, so losing the end of it defeats the purpose.
    #[prop(into)]
    sublabel: String,
    checked: RwSignal<bool>,
    /// A countdown, or a note like "nothing to publish". **Must not be
    /// interactive**: the wrapper is a `<label>`, so a control in here would be
    /// swallowed and toggle the checkbox instead.
    #[prop(optional)]
    trailing: Option<AnyView>,
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

    view! {
        <label class=class>
            <input
                type="checkbox"
                class=style::input
                prop:checked=move || checked.get()
                disabled=move || is_disabled.get()
                on:change=move |ev| checked.set(event_target_checked(&ev))
            />
            <span class=style::indicator aria-hidden="true">
                <svg viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="2"
                    stroke-linecap="round" stroke-linejoin="round">
                    <path d="M2.5 6.25 4.75 8.5 9.5 3.75" />
                </svg>
            </span>
            <span class=style::text>
                <span class=style::label>{label}</span>
                <span class=style::sublabel>{sublabel}</span>
            </span>
            {trailing.map(|slot| view! { <span class=style::trailing>{slot}</span> })}
        </label>
    }
}
