//! A labelled checkbox row with a sub-label and a trailing slot.
//!
//! The two Autosync toggles are the first callers; Settings has five sections of
//! the same shape.

use leptos::prelude::*;

use super::Checkbox;

stylance::import_crate_style!(style, "src/kit/toggle_row.module.scss");

#[component]
pub fn ToggleRow(
    #[prop(into)] label: String,
    /// One line of explanation under the label. Wraps rather than truncating —
    /// it explains a setting, so losing the end of it defeats the purpose.
    #[prop(into)]
    sublabel: String,
    checked: RwSignal<bool>,
    /// A countdown, or a note like "nothing to publish". Sits *outside* the
    /// label, so clicking it does nothing — a clock is information, not a
    /// control, and flipping a setting because someone clicked a clock would be a
    /// bad surprise. That also means it may safely hold something interactive if
    /// a caller ever needs it to.
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
        <div class=class>
            // Only this part is a label, so only this part toggles.
            <label class=style::main>
                // No `aria_label`: this `<label>` is the box's name, and a second
                // one would win over the words the reader can actually see.
                <Checkbox
                    state=Signal::derive(move || checked.get().into())
                    on_toggle=move |next| checked.set(next)
                    disabled=is_disabled
                />
                <span class=style::text>
                    <span class=style::label>{label}</span>
                    <span class=style::sublabel>{sublabel}</span>
                </span>
            </label>
            {trailing.map(|slot| view! { <span class=style::trailing>{slot}</span> })}
        </div>
    }
}
