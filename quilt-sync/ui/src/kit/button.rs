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
    /// `loading` is set the leading visual is removed and the spinner takes its place, so
    /// they can never both render and a button with a leading visual does not change
    /// width when work starts.
    #[prop(optional)]
    leading_visual: Option<AnyView>,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
    /// Renders a spinner and blocks activation. Implies `disabled` — a caller
    /// never has to set both, and a loading button must not be clickable twice.
    #[prop(optional, into)]
    loading: MaybeProp<bool>,
    /// Whether what this button opens is open. Set it whenever the button opens
    /// something — see [`AnchoredOverlay`](super::AnchoredOverlay), which hands
    /// its trigger the id to point at.
    #[prop(optional, into)]
    aria_expanded: MaybeProp<bool>,
    /// The id of what it opens.
    #[prop(optional, into)]
    aria_controls: MaybeProp<String>,
    children: Children,
) -> impl IntoView {
    let is_loading = Signal::derive(move || loading.get().unwrap_or(false));
    let is_disabled = Signal::derive(move || disabled.get().unwrap_or(false) || is_loading.get());

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

    // Rendered once, not reactively: a leading visual is a property of the call site, not
    // of state. The CSS hides it while loading rather than the Rust removing it,
    // so the spinner swap costs no re-render.
    let visual = leading_visual.map(|glyph| view! { <span class=style::icon>{glyph}</span> });

    view! {
        <button
            type="button"
            class=class
            disabled=move || is_disabled.get()
            aria-busy=move || if is_loading.get() { "true" } else { "false" }
            aria-expanded=move || aria_expanded.get().map(|v| v.to_string())
            aria-controls=move || aria_controls.get()
            on:click=move |ev| {
                if !is_disabled.get() {
                    on_click(ev);
                }
            }
        >
            {visual}
            <span class=style::label>{children()}</span>
        </button>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mount;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn click(el: &web_sys::Element) {
        let el: web_sys::HtmlElement = el.clone().dyn_into().unwrap();
        el.click();
    }

    fn button(el: &web_sys::Element) -> web_sys::Element {
        el.query_selector("button").unwrap().expect("the button")
    }

    /// `loading` implies `disabled`. Its doc says a caller never has to set both and a
    /// loading button must not be clickable twice.
    #[wasm_bindgen_test]
    async fn a_loading_button_is_disabled_busy_and_inert() {
        let clicks = RwSignal::new(0);
        let el = mount(move || {
            view! {
                <Button loading=true on_click=move |_| clicks.update(|n| *n += 1)>
                    "Publish"
                </Button>
            }
        });
        let btn = button(&el);
        assert!(btn.has_attribute("disabled"));
        assert_eq!(btn.get_attribute("aria-busy").as_deref(), Some("true"));

        click(&btn);
        leptos::task::tick().await;
        assert_eq!(clicks.get_untracked(), 0, "a second read must not go out");
    }

    /// Disabled is enforced in the handler as well as by the attribute: the attribute
    /// alone is a promise the DOM keeps, and this is the one the component makes.
    #[wasm_bindgen_test]
    async fn a_disabled_button_does_not_call_its_handler() {
        let clicks = RwSignal::new(0);
        let el = mount(move || {
            view! {
                <Button disabled=true on_click=move |_| clicks.update(|n| *n += 1)>
                    "Publish"
                </Button>
            }
        });
        click(&button(&el));
        leptos::task::tick().await;
        assert_eq!(clicks.get_untracked(), 0);
    }

    #[wasm_bindgen_test]
    async fn an_enabled_button_calls_its_handler_once() {
        let clicks = RwSignal::new(0);
        let el = mount(move || {
            view! {
                <Button on_click=move |_| clicks.update(|n| *n += 1)>"Publish"</Button>
            }
        });
        click(&button(&el));
        leptos::task::tick().await;
        assert_eq!(clicks.get_untracked(), 1);
    }

    /// The leading visual and the spinner are one slot, which is what stops an iconed
    /// button changing width when work starts. The icon stays in the DOM and the
    /// stylesheet hides it, so the swap costs no re-render.
    #[wasm_bindgen_test]
    fn the_leading_visual_keeps_its_slot_while_loading() {
        let el = mount(|| {
            view! {
                <Button
                    loading=true
                    leading_visual=view! { <svg /> }.into_any()
                    on_click=|_| {}
                >
                    "Refresh"
                </Button>
            }
        });
        assert!(
            el.query_selector("[class*=icon]").unwrap().is_some(),
            "the slot is kept, not emptied"
        );
        assert!(
            button(&el).class_name().contains("loading"),
            "and the stylesheet is what hides it"
        );
    }

    /// The slot is only one slot because the stylesheet hides the icon while loading.
    ///
    /// Read from the source, not from a computed style: no stylesheet is loaded in the
    /// test harness, so `getComputedStyle` there returns browser defaults. Same reason
    /// the kit's measure test reads its SCSS.
    ///
    /// The sizes are deliberately not asserted equal. At the large size both are 16px,
    /// but at the medium one the icon is 14px against the spinner's 12px, so the button
    /// narrows by 2px — stated in `DESIGN.md` rather than pinned as if it were exact.
    #[test]
    fn the_stylesheet_is_what_hides_the_icon_while_loading() {
        const SHEET: &str = include_str!("button.module.scss");
        let rule = SHEET
            .split(".loading .icon {")
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .expect("a rule hiding the icon while loading");

        // The declaration, not the characters. A substring passes for a commented-out
        // rule, for `none-block`, and for a `display` that a later one overrides.
        let display = rule
            .split("/*")
            .map(|part| part.split_once("*/").map_or(part, |(_, rest)| rest))
            .flat_map(|part| part.lines())
            .map(|line| line.split("//").next().unwrap_or(""))
            .flat_map(|line| line.split(';'))
            .filter_map(|declaration| declaration.split_once(':'))
            .filter(|(property, _)| property.trim() == "display")
            .map(|(_, value)| value.trim())
            .last();

        assert_eq!(
            display,
            Some("none"),
            "without it the icon and the spinner both draw: {rule}"
        );
    }

    /// Inside a form the default would submit it.
    #[wasm_bindgen_test]
    fn a_button_is_never_a_submit() {
        let el = mount(|| view! { <Button on_click=|_| {}>"Publish"</Button> });
        assert_eq!(button(&el).get_attribute("type").as_deref(), Some("button"));
    }
}
