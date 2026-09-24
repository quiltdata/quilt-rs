//! Text button.
//!
//! Three variants and six interaction states. `loading` and `disabled` are not
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
    /// The verb on a confirmation — the one button a region steers you *away* from, so it
    /// is never also `Primary`. Drawn in the danger tone's muted trio and never a solid
    /// red: the tokens carry no emphasis role for a status tone, because no foreground
    /// passes on a step-9 fill. Only `ConfirmDialog` draws one; a Danger button anywhere
    /// else is a command that skipped its confirmation.
    Danger,
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
    /// Submits the form named by `form` rather than doing nothing on its own.
    ///
    /// The default is `type="button"` and stays that way: a `<button>` inside a `<form>`
    /// submits it unless told otherwise, which is the accidental-submit bug every codebase
    /// ships once. Opting in is the only way to get it.
    #[prop(optional)]
    submit: bool,
    /// The `<form>` this button belongs to, by id — for a button that sits **outside** it.
    ///
    /// A [`FormDialog`](super::FormDialog)'s primary is exactly that case: the fields are a
    /// `<form>` in the dialog's body and the buttons are in its footer, a sibling. The
    /// association is what makes the form's Enter key reach this button, so it is not
    /// cosmetic.
    #[prop(optional, into)]
    form: MaybeProp<String>,
    /// Focused when the dialog holding it opens: `showModal()` hands focus to the first
    /// `autofocus` inside the dialog. For the safe answer of a confirmation, so a stray
    /// Return does nothing destructive. Same prop `TextInput` has for a form's first field.
    #[prop(optional)]
    autofocus: bool,
    /// Lets the label take a second line rather than truncate. Opt-in, for a
    /// label that must be read whole in a narrow column; every other button
    /// keeps one line and the ellipsis. An attribute, not a class, so the
    /// stylesheet and the tests find it by the same name.
    #[prop(optional)]
    wrap: bool,
    children: Children,
) -> impl IntoView {
    let is_loading = Signal::derive(move || loading.get().unwrap_or(false));
    let is_disabled = Signal::derive(move || disabled.get().unwrap_or(false) || is_loading.get());

    // One computed `class`: Leptos permits a single `class=` per element, and
    // the module consts are plain `&'static str`, so this is just joining them.
    let class = move || {
        let mut out = String::from(style::btn);
        match variant {
            ButtonVariant::Default => {}
            ButtonVariant::Primary => {
                out.push(' ');
                out.push_str(style::primary);
            }
            ButtonVariant::Danger => {
                out.push(' ');
                out.push_str(style::danger);
            }
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

    // Fixed at the call site, not reactive: whether a button submits is what it is for.
    let button_type = if submit { "submit" } else { "button" };

    view! {
        <button
            type=button_type
            form=move || form.get()
            autofocus=autofocus
            data-wrap=wrap.then_some("")
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
    use crate::test_support::{button_saying, mount};
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
    fn a_button_does_not_submit_unless_it_was_asked_to() {
        let el = mount(|| view! { <Button on_click=|_| {}>"Publish"</Button> });
        assert_eq!(button(&el).get_attribute("type").as_deref(), Some("button"));
        assert!(
            !button(&el).has_attribute("form"),
            "and it belongs to no form it was not given"
        );
    }

    /// Rendered only when asked: a button that always asked for focus would take it from
    /// a dialog's first field.
    #[wasm_bindgen_test]
    fn a_button_asks_for_focus_only_when_told_to() {
        let plain = mount(|| view! { <Button on_click=|_| {}>"Cancel"</Button> });
        assert!(!button(&plain).has_attribute("autofocus"));
        let asked = mount(|| view! { <Button autofocus=true on_click=|_| {}>"Cancel"</Button> });
        assert!(button(&asked).has_attribute("autofocus"));
    }

    /// The opt-in, and the association that makes it reach a form it is not inside —
    /// see [`FormDialog`](super::super::FormDialog), whose footer is that case.
    #[wasm_bindgen_test]
    fn a_submit_button_names_the_form_it_submits() {
        let el = mount(|| {
            view! {
                <Button submit=true form="q-form-7" on_click=|_| {}>
                    "Save"
                </Button>
            }
        });
        assert_eq!(button(&el).get_attribute("type").as_deref(), Some("submit"));
        assert_eq!(
            button(&el).get_attribute("form").as_deref(),
            Some("q-form-7")
        );
    }

    /// The third variant. Marked in the class list, which is how the stylesheet finds it —
    /// `contains`, because stylance hashes the name but keeps it, as the loading test relies on.
    #[wasm_bindgen_test]
    fn a_danger_button_is_marked_as_one() {
        let el = mount(|| {
            view! { <Button variant=ButtonVariant::Danger on_click=|_| {}>"Remove"</Button> }
        });
        let btn = button(&el);
        assert!(
            btn.class_name().contains("danger"),
            "the variant reaches the stylesheet: {}",
            btn.class_name()
        );
        assert_eq!(
            btn.get_attribute("type").as_deref(),
            Some("button"),
            "a verb, not a submit"
        );
    }

    /// Read from the source, as the loading test does. The danger rule spends the tone's
    /// muted trio — the same three properties Banner's `.critical` reads — and no literal,
    /// so a Danger verb and the refusal above it cannot disagree about what red means.
    #[test]
    fn the_danger_rule_reads_the_tones_muted_tokens_and_no_literal() {
        const SHEET: &str = include_str!("button.module.scss");
        let rule = SHEET
            .split(".danger {")
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .expect("a `.danger` rule");
        for token in [
            "--q-bgColor-danger-muted",
            "--q-borderColor-danger-muted",
            "--q-fgColor-danger-onMuted",
        ] {
            assert!(rule.contains(token), "the rule spends {token}: {rule}");
        }
        assert!(!rule.contains('#'), "tokens only, no literal: {rule}");
    }

    /// Opt-in: every other button keeps one line and the ellipsis.
    #[wasm_bindgen_test]
    fn a_wrapping_button_says_so_and_others_do_not() {
        let el = mount(|| {
            view! {
                <Button wrap=true on_click=|_| {}>"Replace mine with the published one"</Button>
                <Button on_click=|_| {}>"Cancel"</Button>
            }
        });
        assert!(
            button_saying(&el, "Replace mine with the published one").has_attribute("data-wrap")
        );
        assert!(!button_saying(&el, "Cancel").has_attribute("data-wrap"));
    }

    /// The last value of `property` declared in the rule opened by `selector`,
    /// parsed as the loading test parses its own.
    fn declared(sheet: &str, selector: &str, property: &str) -> Option<String> {
        let rule = sheet
            .split(&format!("{selector} {{"))
            .nth(1)
            .and_then(|rest| rest.split('}').next())?;
        rule.split("/*")
            .map(|part| part.split_once("*/").map_or(part, |(_, rest)| rest))
            .flat_map(|part| part.lines())
            .map(|line| line.split("//").next().unwrap_or(""))
            .flat_map(|line| line.split(';'))
            .filter_map(|declaration| declaration.split_once(':'))
            .filter(|(name, _)| name.trim() == property)
            .map(|(_, value)| value.trim().to_string())
            .last()
    }

    /// Read from the source: the wasm runner loads no stylesheet.
    #[test]
    fn the_stylesheet_lets_a_wrapping_label_break() {
        const SHEET: &str = include_str!("button.module.scss");
        assert_eq!(
            declared(SHEET, "[data-wrap]", "white-space").as_deref(),
            Some("normal")
        );
        assert_eq!(
            declared(SHEET, "[data-wrap] .label", "text-overflow").as_deref(),
            Some("clip")
        );
    }
}
