//! One command on its face, its alternatives behind a caret.
//!
//! # Legal for the same reason [`ActionMenu`](super::ActionMenu) is
//!
//! DESIGN.md's **Platform Owns The Keyboard Rule** bans a listbox and a combobox
//! because each replaces a native form control that already works. This replaces
//! nothing: both halves are real `<button>`s, Tab reaches each in reading order,
//! and the surface holds commands rather than values. `Select` picks a **value**;
//! this fires a **command**, and so does everything behind its caret.
//!
//! No `role="menu"` and no `aria-haspopup`, for the reason `ActionMenu` spells
//! out: both promise arrow-key navigation this kit deliberately does not
//! hand-write. The caret carries `aria-expanded` and `aria-controls`, which say
//! that it opens something and which surface it means, without claiming a
//! keyboard model.
//!
//! # Which command is on the face is the caller's business
//!
//! The component renders the default it is handed. It does not choose one, and it
//! does not remember one — that is the layering rule (*kit renders from props;
//! page owns state, I/O and meaning*), and it is what lets a page persist the
//! reader's last choice without this file knowing that preferences exist.
//!
//! Worth knowing before a caller does persist one: a face that changes on
//! invisible history means two readers of the same screen see different buttons,
//! and muscle memory on this kind of control is positional. Fine for a preference
//! the reader set deliberately; a trap for one inferred from what they did last.
//!
//! # The caret is drawn here, the default half is not
//!
//! The face is a [`Button`](super::Button), so every variant, the disabled state
//! and the loading spinner come from the one place they are defined. The caret
//! cannot be an [`IconButton`](super::IconButton): that has no primary variant,
//! and a navy face beside a white caret is two controls, not one. So the caret's
//! fill is restated in this module's stylesheet — the only duplication here, and
//! deliberate over adding a variant to `IconButton` that only this would use.

use leptos::ev::MouseEvent;
use leptos::prelude::*;

use super::Align;
use super::AnchoredOverlay;
use super::Button;
use super::ButtonVariant;
use super::MenuAction;
use super::action_menu;
use super::icons;

stylance::import_crate_style!(style, "src/kit/split_button.module.scss");

#[component]
pub fn SplitButton(
    /// The command on the face — the half that runs without opening anything.
    ///
    /// A `Signal`, not a plain value: a page that persists which command the
    /// reader last used writes that preference into a signal, and the face has to
    /// follow it without the caller remounting the control.
    #[prop(into)]
    label: Signal<String>,
    /// Runs the command on the face.
    on_click: impl Fn(MouseEvent) + 'static,
    /// Names the caret, and through it the surface — `Other ways to publish`,
    /// not `More`. The face already has a name; this has to say what opening it
    /// would offer.
    #[prop(into)]
    menu_label: String,
    /// The alternatives. The face's own command is **not** repeated here — it is
    /// already on screen, and a menu that lists what you just clicked reads as a
    /// mistake.
    actions: Vec<MenuAction>,
    #[prop(optional)] variant: ButtonVariant,
    /// Disables both halves. A caret that opens alternatives to a command you
    /// cannot run offers nothing.
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
            <Button variant=variant disabled=disabled loading=loading on_click=on_click>
                {move || label.get()}
            </Button>
            <AnchoredOverlay
                trigger=trigger
                open=open
                aria_label=surface_label
                align=Align::End
                tight=true
            >
                {action_menu::surface(actions, open)}
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

    fn action(label: &str, hits: RwSignal<Vec<String>>) -> MenuAction {
        let name = label.to_string();
        MenuAction::new(
            label,
            Callback::new(move |()| hits.update(|h| h.push(name.clone()))),
        )
    }

    /// The point of the control: one click runs the face, without opening
    /// anything. Deleting the `on_click` wiring must fail here.
    #[wasm_bindgen_test]
    fn the_face_runs_without_opening_the_surface() {
        let hits = RwSignal::new(Vec::<String>::new());
        let root = mount(move || {
            view! {
                <SplitButton
                    label="Publish"
                    menu_label="Other ways to publish"
                    actions=vec![action("Create new revision", hits)]
                    on_click=move |_| hits.update(|h| h.push("face".to_string()))
                />
            }
        });

        buttons(&root)[0].click();

        assert_eq!(hits.get_untracked(), vec!["face".to_string()]);
        assert!(
            !root
                .query_selector(":popover-open")
                .unwrap()
                .is_some_and(|_| true),
            "clicking the face must not open the surface"
        );
    }

    /// Two real buttons, both reachable. A single button with a click zone that
    /// behaves differently at one end would pass a render test and fail a reader.
    #[wasm_bindgen_test]
    fn both_halves_are_real_buttons() {
        let hits = RwSignal::new(Vec::<String>::new());
        let root = mount(move || {
            view! {
                <SplitButton
                    label="Publish"
                    menu_label="Other ways to publish"
                    actions=vec![action("Create new revision", hits)]
                    on_click=|_| ()
                />
            }
        });

        let found = buttons(&root);
        assert!(
            found.len() >= 2,
            "expected a face and a caret, found {}",
            found.len()
        );
        assert_eq!(found[0].text_content().unwrap().trim(), "Publish");
        assert_eq!(
            found[1].get_attribute("aria-label").as_deref(),
            Some("Other ways to publish"),
            "the caret names what it opens, not itself"
        );
    }

    /// `aria-expanded` is what tells a reader the caret opens anything at all.
    /// Removing it leaves a button that announces a name and nothing else.
    #[wasm_bindgen_test]
    fn the_caret_says_whether_it_is_open() {
        let hits = RwSignal::new(Vec::<String>::new());
        let root = mount(move || {
            view! {
                <SplitButton
                    label="Publish"
                    menu_label="Other ways to publish"
                    actions=vec![action("Create new revision", hits)]
                    on_click=|_| ()
                />
            }
        });

        let caret = buttons(&root).remove(1);
        assert_eq!(
            caret.get_attribute("aria-expanded").as_deref(),
            Some("false")
        );
        assert!(
            caret.get_attribute("aria-controls").is_some(),
            "the caret must name the surface it opens"
        );
    }

    /// Disabling the command must disable the alternatives too — a caret that
    /// offers other ways to do something you cannot do is a dead end.
    #[wasm_bindgen_test]
    fn disabled_stops_both_halves() {
        let hits = RwSignal::new(Vec::<String>::new());
        let root = mount(move || {
            view! {
                <SplitButton
                    label="Publish"
                    menu_label="Other ways to publish"
                    actions=vec![action("Create new revision", hits)]
                    disabled=true
                    on_click=move |_| hits.update(|h| h.push("face".to_string()))
                />
            }
        });

        let found = buttons(&root);
        let face: web_sys::HtmlButtonElement = found[0].clone().dyn_into().unwrap();
        let caret: web_sys::HtmlButtonElement = found[1].clone().dyn_into().unwrap();
        assert!(face.disabled(), "the face");
        assert!(caret.disabled(), "the caret");

        found[0].click();
        assert!(
            hits.get_untracked().is_empty(),
            "a disabled face must not run its command"
        );
    }

    /// Loading implies disabled, and the caret goes with it: work is in flight,
    /// so neither half should start more.
    #[wasm_bindgen_test]
    fn loading_stops_the_caret_too() {
        let hits = RwSignal::new(Vec::<String>::new());
        let root = mount(move || {
            view! {
                <SplitButton
                    label="Publish"
                    menu_label="Other ways to publish"
                    actions=vec![action("Create new revision", hits)]
                    loading=true
                    on_click=|_| ()
                />
            }
        });

        let caret: web_sys::HtmlButtonElement = buttons(&root).remove(1).dyn_into().unwrap();
        assert!(caret.disabled());
    }
}
