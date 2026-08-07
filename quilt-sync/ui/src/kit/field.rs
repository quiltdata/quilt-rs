//! A label, a control, and what to say about it.
//!
//! Primer's `FormControl` under a shorter name, and **not yet aligned with it** — that work
//! is `qhq-kt31`, deferred to the wider Primer naming review. What is here is the honest
//! interim, and the two ways it differs are worth knowing:
//!
//! # It wraps the control instead of pointing at it
//!
//! The label is an actual `<label>` element with the control inside it, so the browser
//! associates the two with no `for`, no `id`, and nothing to keep in step. `ToggleRow`
//! already uses this and it is the only labelling that cannot silently come apart.
//!
//! # Which is why the caption and the message are not announced
//!
//! They sit *outside* the `<label>`, because everything inside one contributes to the
//! accessible name and "Package name owner/package-name Use owner/name" is a worse name
//! than "Package name". Outside, they are visually associated and **programmatically
//! invisible**: there is no `aria-describedby`, because that needs ids on both ends.
//!
//! That is the concrete cost of deferring `qhq-kt31`, and the concrete reason to do it: the
//! `FieldId` design there gives `Field` an id it can wire to a control that is required to
//! accept one, which fixes this and makes an unlabelled control a compile error at the same
//! time.

use leptos::prelude::*;

use super::state_label::StateTone;

stylance::import_crate_style!(style, "src/kit/field.module.scss");

#[component]
pub fn Field(
    #[prop(into)] label: String,
    /// A hint that is always shown — the shape wanted, or a consequence worth stating
    /// before the user commits. Not an error.
    #[prop(optional, into)]
    caption: Option<String>,
    /// What is wrong, shown only when there is something. Paired with `TextInput`'s
    /// `invalid`, which draws the border; this says why.
    #[prop(optional, into)]
    error: MaybeProp<String>,
    /// Marks the field required and appends an indicator to the label. The word rather than
    /// an asterisk: an asterisk is a convention you have to have learned, and there is room.
    #[prop(optional)]
    required: bool,
    children: Children,
) -> impl IntoView {
    view! {
        <div class=style::root>
            <label class=style::label>
                <span class=style::name>
                    {label}
                    {required.then(|| view! { <span class=style::required>"(required)"</span> })}
                </span>
                {children()}
            </label>
            {caption.map(|caption| view! { <p class=style::caption>{caption}</p> })}
            // The tone's own glyph, so an error here and an error on a row agree about what
            // red means — and so the message survives greyscale like everything else.
            <Show when=move || error.get().is_some()>
                <p class=style::error>
                    {StateTone::Danger.glyph()}
                    <span>{move || error.get().unwrap_or_default()}</span>
                </p>
            </Show>
        </div>
    }
}
