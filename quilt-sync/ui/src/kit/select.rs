//! Single-choice control over a native `<select>`.
//!
//! §2 of the design record eliminates the tooltip/dropdown/combobox class by
//! committing to this: our closed state, the OS's open dropdown. It holds only
//! while every choice is a plain string — a choice needing rich formatting means
//! the control is wrong, and the answer is an inline list or radio group.

use leptos::prelude::*;

use super::Naming;

stylance::import_crate_style!(style, "src/kit/select.module.scss");

#[component]
pub fn Select(
    /// How this select is named, and the reason there is no way to build an anonymous one.
    /// All three [`Naming`] variants apply here: inside a [`FormControl`](super::FormControl), or
    /// standalone in a toolbar with the name hidden or drawn as a `Group by:` prefix.
    naming: Naming,
    /// Rendered in order. A single-option select is a dead control; callers
    /// check first (`role_switch_host` is `Some` only for multi-role hosts).
    options: Vec<String>,
    selected: RwSignal<String>,
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

    // The prefix is `aria-hidden` and the name is repeated in `aria-label`, rather than the
    // prefix being the name: the visible text ends in a colon and the name should not.
    let (prefix, aria_label, id, described_by) = match naming {
        Naming::FormControl(ids) => {
            let (id, described_by) = ids.into_attrs();
            (None, None, Some(id), Some(described_by))
        }
        Naming::Hidden(name) => (None, Some(name), None, None),
        Naming::Prefix(name) => (Some(format!("{name}:")), Some(name), None, None),
    };

    view! {
        // A `div`, not a `label`. The `select` is stretched over the whole box at
        // `opacity: 0` (see the stylesheet), so every pixel already opens the dropdown
        // without a label to forward the click — and when a `FormControl` names this, the
        // `<label for>` is the field's, not ours.
        <div class=class>
            {prefix
                .map(|text| view! { <span class=style::prefix aria-hidden="true">{text}</span> })}
            // `aria-hidden`, like the prefix and the caret: this is the closed state
            // we drew, and the real `select` underneath already reports the value.
            <span class=style::value aria-hidden="true">{move || selected.get()}</span>
            <span class=style::caret aria-hidden="true">
                <svg viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5"
                    stroke-linecap="round" stroke-linejoin="round">
                    <path d="M3 4.75 6 7.75l3-3" />
                </svg>
            </span>
            <select
                class=style::field
                id=id
                aria-label=aria_label
                aria-describedby=described_by
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
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn mount<N: IntoView + 'static>(f: impl FnOnce() -> N + 'static) -> web_sys::Element {
        let doc = web_sys::window().unwrap().document().unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        leptos::mount::mount_to(container.clone(), f).forget();
        container.into()
    }

    /// The closed state is ours; the value belongs to the `select` underneath it.
    /// Both exposed, a reader hears the bucket twice.
    #[wasm_bindgen_test]
    fn the_drawn_value_is_hidden_from_the_accessibility_tree() {
        let el = mount(|| {
            view! {
                <Select
                    naming=Naming::Hidden("Bucket".to_string())
                    options=vec!["one".to_string(), "two".to_string()]
                    selected=RwSignal::new("one".to_string())
                />
            }
        });
        let drawn = el
            .query_selector("[class*=value]")
            .unwrap()
            .expect("the drawn value");
        assert_eq!(drawn.get_attribute("aria-hidden").as_deref(), Some("true"));
    }
}
