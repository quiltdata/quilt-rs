//! A label, a control, and what to say about it.
//!
//! Primer's `FormControl` under a shorter name.
//!
//! # Props, not child components
//!
//! Primer composes this from children — `FormControl.Label`, `.Caption`, `.Validation`.
//! In JSX that is free; in Leptos `children` is an opaque closure, so `FormControl` cannot reach
//! inside it to wire `for`/`id`. A `FieldLabel` child would have to pull the id from
//! context, and a forgotten provider gives you a label pointing at nothing, with no error
//! anywhere. Primer's reason for children — arbitrary ordering and omission — is not a
//! need we have.
//!
//! # The type system does the job Primer gives to a linter
//!
//! Primer's `FormControl` is optional at the type level, so a bare unlabelled `Select` is
//! constructible; eslint and axe catch it in CI. We have neither, and we already shipped
//! that exact bug once — `Select`'s label rendered nowhere and only clippy's
//! unused-parameter warning noticed.
//!
//! So [`ControlId`]'s constructor is private to this module. The only way to get one is from
//! the closure `FormControl` hands you, and every control that belongs in a form demands one:
//!
//! ```ignore
//! <FormControl label="Bucket" control=move |id| view! { <TextInput id=id value=bucket /> }.into_any() />
//! ```
//!
//! An unlabelled control is now a compile error rather than an accessibility bug found by
//! someone using a screen reader.

use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use leptos::prelude::*;

use super::state_label::StateTone;

stylance::import_crate_style!(style, "src/kit/form_control.module.scss");

/// Ids only have to be unique within a document, and this is a single-threaded wasm
/// document. `Relaxed` is enough: nothing orders anything against this counter.
static NEXT: AtomicUsize = AtomicUsize::new(0);

/// The ids one [`FormControl`] allocated for its control.
///
/// Constructed only by `FormControl` — see the module docs for why that is the whole point.
#[derive(Clone, Debug)]
pub struct ControlId {
    control: String,
    described_by: String,
}

impl ControlId {
    /// Unpacks into `(id, aria-describedby)`. Consuming rather than lending, because both
    /// values go straight onto one element and neither is wanted again.
    ///
    /// `id` is what the field's `<label for>` points at.
    ///
    /// `aria-describedby` is what the previous wrapping-`<label>` design could not do at
    /// all. Everything inside a `<label>` contributes to the accessible *name*, so a
    /// caption inside one gives you "Package name owner/package-name Use owner/name" —
    /// which is why the caption and the message sat outside it, visually associated and
    /// programmatically invisible. A description needs ids on both ends, and now there are
    /// ids.
    ///
    /// It always names the validation id, even with no message showing: a message can
    /// appear on any keystroke, and `aria-describedby` pointing at an absent id is defined
    /// to be ignored rather than to be an error.
    #[must_use]
    pub fn into_attrs(self) -> (String, String) {
        (self.control, self.described_by)
    }
}

/// How a control gets its accessible name.
///
/// There is no "unnamed" variant, which is the point. Every control that takes this is
/// named one of three ways, and the caller has to say which.
#[derive(Clone, Debug)]
pub enum Naming {
    /// Named by a [`FormControl`]'s label, and described by its caption and validation message.
    FormControl(ControlId),
    /// Named by an `aria-label` that is never drawn. For a control whose job is clear from
    /// where it sits — a list toolbar's filters — where a stacked label would cost a line
    /// and say nothing the user did not already know.
    Hidden(String),
    /// Named by a `Name:` prefix drawn inside the control, so the control reads
    /// `Group by: Bucket`.
    ///
    /// Only [`Select`](super::Select) takes this. The other controls have nowhere to put a
    /// prefix, and the enum they accept is the one without this variant — which is why
    /// `TextInput` takes a bare [`ControlId`] and not a `Naming`.
    Prefix(String),
}

#[component]
pub fn FormControl(
    #[prop(into)] label: String,
    /// A hint that is always shown — the shape wanted, or a consequence worth stating
    /// before the user commits. Not an error.
    #[prop(optional, into)]
    caption: Option<String>,
    /// What is wrong, shown only when there is something. Paired with `TextInput`'s
    /// `invalid`, which draws the border and sets `aria-invalid`; this says why.
    ///
    /// Not Primer's `Validation` with a required `variant`: theirs also has a `success`
    /// tone, and we have no success validation and no plan for one.
    #[prop(optional, into)]
    error: MaybeProp<String>,
    /// Marks the field required and appends an indicator to the label. The word rather than
    /// an asterisk: an asterisk is a convention you have to have learned, and there is room.
    #[prop(optional)]
    required: bool,
    /// The control, built from the ids this `FormControl` allocated for it. Taking a
    /// closure rather than `children` is what makes the wiring checkable — see the module
    /// docs.
    control: impl FnOnce(ControlId) -> AnyView,
) -> impl IntoView {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let control_id = format!("q-control-{n}");
    let caption_id = format!("q-control-{n}-caption");
    let error_id = format!("q-control-{n}-error");

    let described_by = if caption.is_some() {
        format!("{caption_id} {error_id}")
    } else {
        error_id.clone()
    };
    let control = control(ControlId {
        control: control_id.clone(),
        described_by,
    });

    view! {
        <div class=style::root>
            // `for` rather than wrapping the control. Wrapping was what held the
            // association together before there were ids; with the id required by the
            // control's own signature it cannot come apart either, and not wrapping is
            // what lets the caption and the message be a description instead of part of
            // the name.
            <label class=style::name for=control_id>
                {label}
                {required.then(|| view! { <span class=style::required>"(required)"</span> })}
            </label>
            {control}
            {caption.map(|caption| view! { <p class=style::caption id=caption_id>{caption}</p> })}
            // The tone's own glyph, so an error here and an error on a row agree about what
            // red means — and so the message survives greyscale like everything else.
            <Show when=move || error.get().is_some()>
                <p class=style::error id=error_id.clone()>
                    {StateTone::Danger.glyph()}
                    <span>{move || error.get().unwrap_or_default()}</span>
                </p>
            </Show>
        </div>
    }
}
