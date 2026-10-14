//! Nothing to show, and what to do about it.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/blankslate.module.scss");

#[component]
pub fn Blankslate(
    #[prop(into)] heading: String,
    #[prop(into)] description: String,
    /// Offered when there is something to do.
    ///
    /// **A "no results" state with no action is a deferral, not a rule.** This
    /// prop used to say a button there would be filler; nobody decided that. The
    /// position (2026-09-18) is the opposite — an empty list is a frustration and
    /// the page owes the reader a way out of it — and the way out needs
    /// designing, because narrowing compounds: a file list narrowed by a facet
    /// *and* a query has two causes and clearing one can leave a second empty
    /// list. Until that is drawn, callers pass nothing here and the state is
    /// honest about being unfinished rather than principled.
    #[prop(optional)]
    primary_action: Option<AnyView>,
    /// Vertical air for a full region, dropped for one inside a box that is
    /// already short.
    ///
    /// The installed package's file list is **264px at the height floor**, and
    /// this component's `space-10` padding is taller than that — so a full-height
    /// empty state there pushes the list's own border off screen. Compact keeps
    /// the words, the type and the centring, and spends a quarter of the height:
    /// it is the padding that is short, not the heading.
    ///
    /// Not a separate component: what a narrowed list needs to say is exactly
    /// what this says, and the one thing that differs is how much room it takes.
    #[prop(optional)]
    compact: bool,
    /// A muted glyph above the heading.
    #[prop(optional)]
    visual: Option<AnyView>,
) -> impl IntoView {
    let class = if compact {
        format!("{} {}", style::root, style::compact)
    } else {
        String::from(style::root)
    };

    view! {
        <div class=class>
            {visual.map(|visual| view! { <span class=style::visual>{visual}</span> })}
            // `h3`: a blankslate stands where a card's rows would, under its `h2`.
            <h3 class=style::heading>{heading}</h3>
            <span class=style::description>{description}</span>
            {primary_action}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mount;
    use wasm_bindgen_test::*;

    /// The visual comes first, above the heading.
    #[wasm_bindgen_test]
    fn the_visual_leads() {
        let el = mount(|| {
            view! {
                <Blankslate
                    heading="Nothing"
                    description="Nothing here."
                    visual=view! { <svg data-glyph="" /> }.into_any()
                />
            }
        });
        let root = el.first_element_child().unwrap();
        let first = root.first_element_child().unwrap();
        assert!(first.query_selector("[data-glyph]").unwrap().is_some());
    }
}
