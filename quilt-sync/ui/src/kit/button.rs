//! Text button.
//!
//! Two variants and six interaction states. `loading` and `disabled` are not
//! optional extras: the two-phase pull requires an action that is disabled
//! while checking and offers a retry afterwards, so every caller needs both.

use leptos::ev::MouseEvent;
use leptos::prelude::*;

// The component names its own stylesheet. `stylance-cli` hashes each class in
// that file and concatenates every module into one generated stylesheet, so
// there is no list of stylesheets to maintain and no way for this file's
// classes to collide with another component's.
stylance::import_crate_style!(style, "src/kit/button.module.scss");

/// Visual weight. `Primary` is the one affordance a region is steering you
/// toward; everything else is `Default`. A region with two primaries has a
/// design problem, not a prop problem.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonVariant {
    #[default]
    Default,
    Primary,
}

/// Physical size. Orthogonal to [`ButtonVariant`] — any weight can be any size,
/// which is why they are separate props rather than one combined enum.
/// `Large` is for page-level and dialog-confirm actions; list rows and toolbars
/// use `Medium`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonSize {
    #[default]
    Medium,
    Large,
}

#[component]
pub fn Button(
    on_click: impl Fn(MouseEvent) + 'static,
    #[prop(optional)] variant: ButtonVariant,
    #[prop(optional)] size: ButtonSize,
    /// Optional leading glyph. **The same slot the loading spinner uses**: while
    /// `loading` is set the icon is removed and the spinner takes its place, so
    /// they can never both render and a button with an icon does not change
    /// width when work starts.
    #[prop(optional)]
    icon: Option<AnyView>,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
    /// Renders a spinner and blocks activation. Implies `disabled` — a caller
    /// never has to set both, and a loading button must not be clickable twice.
    #[prop(optional, into)]
    loading: MaybeProp<bool>,
    children: Children,
) -> impl IntoView {
    let is_loading = Signal::derive(move || loading.get().unwrap_or(false));
    let is_disabled =
        Signal::derive(move || disabled.get().unwrap_or(false) || is_loading.get());

    // One computed `class`: Leptos permits a single `class=` per element, and
    // the module consts are plain `&'static str`, so this is just joining them.
    let class = move || {
        let mut out = String::from(style::btn);
        if matches!(variant, ButtonVariant::Primary) {
            out.push(' ');
            out.push_str(style::primary);
        }
        if matches!(size, ButtonSize::Large) {
            out.push(' ');
            out.push_str(style::large);
        }
        if is_loading.get() {
            out.push(' ');
            out.push_str(style::loading);
        }
        out
    };

    // Rendered once, not reactively: an icon is a property of the call site, not
    // of state. The CSS hides it while loading rather than the Rust removing it,
    // so the spinner swap costs no re-render.
    let icon_view = icon.map(|glyph| view! { <span class=style::icon>{glyph}</span> });

    view! {
        <button
            type="button"
            class=class
            disabled=move || is_disabled.get()
            aria-busy=move || if is_loading.get() { "true" } else { "false" }
            on:click=move |ev| {
                if !is_disabled.get() {
                    on_click(ev);
                }
            }
        >
            {icon_view}
            <span class=style::label>{children()}</span>
        </button>
    }
}
