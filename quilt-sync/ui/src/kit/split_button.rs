//! One chosen command on its face, the rest of the set behind a caret.
//!
//! # A choice, not a list of other commands
//!
//! The face shows whichever option is **selected**; the caret opens the whole
//! set with the selected one marked. Picking a different one moves the mark and
//! changes what the face will do — it does not run it. Opening a menu to change a
//! preference must not also publish to a shared bucket.
//!
//! That is why the menu lists the face's own command rather than excluding it:
//! the menu is where you *see which one is active*, so leaving the active one out
//! would leave out the thing it is for. (An earlier version of this component was
//! the other shape — face runs X, caret offers Y and Z, nothing selected — and
//! its menu deliberately omitted the face. Both are real controls; this is the
//! one with a memory.)
//!
//! # Where it sits against the Platform Owns The Keyboard Rule
//!
//! DESIGN.md draws its line with *`Select` picks a **value**, `ActionMenu` fires
//! a **command***, and bans a listbox because it replaces a native control that
//! already works. This menu does pick something, so it sits nearer that line than
//! [`ActionMenu`](super::ActionMenu) does and the justification has to be
//! narrower:
//!
//! - It is not a form control. Nothing is submitted and there is no value to read
//!   back; a `<select>` in its place would be a `<select>` that runs code.
//! - It claims no keyboard model. Both halves and every option are real buttons,
//!   reached with Tab in reading order — no `role="menu"`, no `aria-haspopup`, no
//!   roving focus. `aria-current` marks the active option, because `aria-checked`
//!   would need a `radio` or `menuitemradio` role and both promise arrow keys.
//! - Escape, light dismiss and the top layer are the platform's, through
//!   [`AnchoredOverlay`](super::AnchoredOverlay).
//!
//! If it ever needs arrow-key movement between options it has become a listbox,
//! and should be a `Select` beside a `Button` instead.
//!
//! # The selection is the caller's, and so is remembering it
//!
//! `selected` is an `RwSignal` the caller owns: this writes it when somebody
//! picks, and the page reads it to persist. The component has no idea that
//! preferences exist — the layering rule, *kit renders from props, page owns
//! state, I/O and meaning* — which is also what lets a caller that wants the
//! choice to reset every time use the same control.
//!
//! Worth knowing for whoever persists it: a face that changes on history means
//! two people at the same screen see different buttons, and muscle memory on a
//! control like this is positional. A preference somebody set deliberately is
//! fine; one inferred from what they did last is the one to be careful with.
//!
//! # The caret is drawn here, the face is not
//!
//! The face is a [`Button`](super::Button), so every variant, the disabled state
//! and the loading spinner come from the one place they are defined. The caret
//! cannot be an [`IconButton`](super::IconButton): that has no primary variant,
//! and a navy face beside a white caret is two controls rather than one. So the
//! caret's fill is restated in this module's stylesheet — the only duplication
//! here, and deliberate over a variant nothing else would use.

use leptos::prelude::*;

use super::Align;
use super::AnchoredOverlay;
use super::Button;
use super::ButtonVariant;
use super::action_menu;
use super::icons;

stylance::import_crate_style!(style, "src/kit/split_button.module.scss");

/// One of the commands a [`SplitButton`] can be set to.
#[derive(Clone)]
pub struct SplitOption {
    /// What the face reads when this one is selected, and what the menu lists.
    pub label: String,
    /// Runs when the **face** is clicked while this one is selected. Picking this
    /// option out of the menu does not call it.
    pub on_run: Callback<()>,
}

impl SplitOption {
    #[must_use]
    pub fn new(label: impl Into<String>, on_run: Callback<()>) -> Self {
        Self {
            label: label.into(),
            on_run,
        }
    }
}

#[component]
pub fn SplitButton(
    /// Every option, in the order the menu lists them. The first is the sensible
    /// default for a caller with nothing remembered.
    options: Vec<SplitOption>,
    /// Which option is on the face, as an index into `options`. Written here when
    /// somebody picks from the menu; read by the caller to persist.
    ///
    /// Out of range degrades to `0` rather than panicking: a persisted index
    /// outlives the list it indexed, and a preference stored by an older build
    /// should fall back to the default rather than take the page down.
    selected: RwSignal<usize>,
    /// Names the caret, and through it the surface — `Change what this button
    /// does`, not `More`. The face already has a name; this has to say what
    /// opening it would offer.
    #[prop(into)]
    menu_label: String,
    #[prop(optional)] variant: ButtonVariant,
    /// Disables both halves. A caret offering other ways to do something you
    /// cannot do is a dead end.
    #[prop(optional, into)]
    disabled: MaybeProp<bool>,
    /// Spinner on the face, and the caret goes with it: work is in flight, so
    /// neither half should start more.
    #[prop(optional, into)]
    loading: MaybeProp<bool>,
) -> impl IntoView {
    let open = RwSignal::new(false);
    let surface_label = menu_label.clone();
    let is_busy =
        Signal::derive(move || disabled.get().unwrap_or(false) || loading.get().unwrap_or(false));

    let options = StoredValue::new(options);
    let labels = options.with_value(|o| o.iter().map(|opt| opt.label.clone()).collect::<Vec<_>>());

    let index = Signal::derive(move || {
        let i = selected.get();
        if options.with_value(|o| i < o.len()) {
            i
        } else {
            0
        }
    });
    let label = Signal::derive(move || {
        options.with_value(|o| {
            o.get(index.get())
                .map(|opt| opt.label.clone())
                .unwrap_or_default()
        })
    });

    let run = move |_| {
        if let Some(option) = options.with_value(|o| o.get(index.get_untracked()).cloned()) {
            option.on_run.run(());
        }
    };

    let caret_class = if matches!(variant, ButtonVariant::Primary) {
        format!("{} {}", style::caret, style::primary)
    } else {
        String::from(style::caret)
    };

    let trigger = move |surface_id: String| {
        view! {
            <button
                type="button"
                class=caret_class
                title=menu_label.clone()
                aria-label=menu_label
                disabled=move || is_busy.get()
                aria-expanded=move || open.get().to_string()
                aria-controls=surface_id
                on:click=move |_| open.update(|o| *o = !*o)
            >
                {icons::chevron_down()}
            </button>
        }
        .into_any()
    };

    view! {
        <div class=style::root>
            <Button variant=variant disabled=disabled loading=loading on_click=run>
                {move || label.get()}
            </Button>
            <AnchoredOverlay
                trigger=trigger
                open=open
                aria_label=surface_label
                align=Align::End
                tight=true
            >
                {action_menu::choices(labels, index, selected, open)}
            </AnchoredOverlay>
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

    fn buttons(root: &web_sys::Element) -> Vec<web_sys::HtmlElement> {
        let list = root.query_selector_all("button").unwrap();
        (0..list.length())
            .filter_map(|i| list.item(i))
            .map(|n| n.dyn_into::<web_sys::HtmlElement>().unwrap())
            .collect()
    }

    fn two(ran: RwSignal<Vec<String>>) -> Vec<SplitOption> {
        vec![
            SplitOption::new(
                "Publish",
                Callback::new(move |()| ran.update(|r| r.push("publish".into()))),
            ),
            SplitOption::new(
                "Create new revision",
                Callback::new(move |()| ran.update(|r| r.push("revision".into()))),
            ),
        ]
    }

    /// The face runs whichever option is selected, not always the first.
    #[wasm_bindgen_test]
    fn the_face_runs_the_selected_option() {
        let ran = RwSignal::new(Vec::<String>::new());
        let selected = RwSignal::new(0_usize);
        let root = mount(move || {
            view! {
                <SplitButton
                    options=two(ran)
                    selected=selected
                    menu_label="Change what this button does"
                />
            }
        });

        buttons(&root)[0].click();
        assert_eq!(ran.get_untracked(), vec!["publish".to_string()]);

        selected.set(1);
        buttons(&root)[0].click();
        assert_eq!(
            ran.get_untracked(),
            vec!["publish".to_string(), "revision".to_string()],
            "the face must follow the selection rather than stay on the first option"
        );
    }

    #[wasm_bindgen_test]
    fn the_face_shows_the_selected_label() {
        let ran = RwSignal::new(Vec::<String>::new());
        let selected = RwSignal::new(1_usize);
        let root = mount(move || {
            view! {
                <SplitButton
                    options=two(ran)
                    selected=selected
                    menu_label="Change what this button does"
                />
            }
        });

        assert_eq!(
            buttons(&root)[0].text_content().unwrap().trim(),
            "Create new revision"
        );
    }

    /// The point of this shape: picking moves the default and runs nothing. If it
    /// ever ran on pick, opening the menu to look at the options would publish.
    #[wasm_bindgen_test]
    fn picking_an_option_changes_the_default_without_running_it() {
        let ran = RwSignal::new(Vec::<String>::new());
        let selected = RwSignal::new(0_usize);
        let root = mount(move || {
            view! {
                <SplitButton
                    options=two(ran)
                    selected=selected
                    menu_label="Change what this button does"
                />
            }
        });

        // face, caret, then the two options on the surface
        buttons(&root)[3].click();

        assert_eq!(selected.get_untracked(), 1, "the default moved");
        assert!(
            ran.get_untracked().is_empty(),
            "and nothing ran: {:?}",
            ran.get_untracked()
        );
    }

    /// The menu lists every option including the one on the face — it is where
    /// you see which is active, so omitting the active one omits the point.
    #[wasm_bindgen_test]
    fn the_menu_lists_every_option_and_marks_the_selected_one() {
        let ran = RwSignal::new(Vec::<String>::new());
        let selected = RwSignal::new(1_usize);
        let root = mount(move || {
            view! {
                <SplitButton
                    options=two(ran)
                    selected=selected
                    menu_label="Change what this button does"
                />
            }
        });

        let found = buttons(&root);
        let options: Vec<String> = found[2..]
            .iter()
            .map(|b| b.text_content().unwrap().trim().to_string())
            .collect();
        assert_eq!(options, vec!["Publish", "Create new revision"]);

        assert_eq!(found[2].get_attribute("aria-current"), None);
        assert_eq!(
            found[3].get_attribute("aria-current").as_deref(),
            Some("true"),
            "the selected option is the marked one"
        );
    }

    /// A persisted index outlives the list it indexed, so an older build's
    /// preference degrades to the default rather than taking the page down.
    #[wasm_bindgen_test]
    fn an_out_of_range_selection_falls_back_to_the_first_option() {
        let ran = RwSignal::new(Vec::<String>::new());
        let selected = RwSignal::new(7_usize);
        let root = mount(move || {
            view! {
                <SplitButton
                    options=two(ran)
                    selected=selected
                    menu_label="Change what this button does"
                />
            }
        });

        assert_eq!(buttons(&root)[0].text_content().unwrap().trim(), "Publish");
        buttons(&root)[0].click();
        assert_eq!(ran.get_untracked(), vec!["publish".to_string()]);

        // And the menu agrees with the face. Marking from the raw selection
        // rather than the fallback leaves every option unticked while the face
        // shows one of them — the menu would be saying "none of these" about a
        // button that is about to run `Publish`.
        let found = buttons(&root);
        assert_eq!(
            found[2].get_attribute("aria-current").as_deref(),
            Some("true"),
            "the option the face fell back to must be the marked one"
        );
        assert_eq!(found[3].get_attribute("aria-current"), None);
    }

    #[wasm_bindgen_test]
    fn disabled_stops_both_halves() {
        let ran = RwSignal::new(Vec::<String>::new());
        let selected = RwSignal::new(0_usize);
        let root = mount(move || {
            view! {
                <SplitButton
                    options=two(ran)
                    selected=selected
                    menu_label="Change what this button does"
                    disabled=true
                />
            }
        });

        let found = buttons(&root);
        let face: web_sys::HtmlButtonElement = found[0].clone().dyn_into().unwrap();
        let caret: web_sys::HtmlButtonElement = found[1].clone().dyn_into().unwrap();
        assert!(face.disabled(), "the face");
        assert!(caret.disabled(), "the caret");

        found[0].click();
        assert!(ran.get_untracked().is_empty());
    }

    #[wasm_bindgen_test]
    fn loading_stops_the_caret_too() {
        let ran = RwSignal::new(Vec::<String>::new());
        let selected = RwSignal::new(0_usize);
        let root = mount(move || {
            view! {
                <SplitButton
                    options=two(ran)
                    selected=selected
                    menu_label="Change what this button does"
                    loading=true
                />
            }
        });

        let caret: web_sys::HtmlButtonElement = buttons(&root).remove(1).dyn_into().unwrap();
        assert!(caret.disabled());
    }
}
