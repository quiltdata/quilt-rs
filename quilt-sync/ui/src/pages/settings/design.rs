use std::future::Future;
use std::pin::Pin;

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use super::event_target_checked;
use crate::commands::{self, Design};
use crate::components::Notification;

// ── Design section ──

/// The design write: `update_design_settings`, or a test's stand-in.
type SaveDesign = fn(Design) -> Pin<Box<dyn Future<Output = Result<(), String>>>>;

fn save_design(design: Design) -> Pin<Box<dyn Future<Output = Result<(), String>>>> {
    Box::pin(commands::update_design_settings(design))
}

/// The one design switch: *Use the classic design*, on for this page, which is
/// the classic one. Turning it off is the way back to v2.
///
/// Saving goes to `/`, the route that asks which design is on: the save alone
/// changes nothing on this screen, which looked like it needed an app restart.
#[component]
pub(super) fn DesignSection(
    design: Design,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
    #[prop(default = save_design)] save: SaveDesign,
) -> impl IntoView {
    let classic = RwSignal::new(design == Design::V1);
    let saving = RwSignal::new(false);
    let navigate = use_navigate();

    let on_toggle = move |ev: leptos::ev::Event| {
        let on = event_target_checked(&ev);
        if saving.get_untracked() {
            return;
        }
        saving.set(true);
        classic.set(on);
        let navigate = navigate.clone();
        leptos::task::spawn_local(async move {
            let design = if on { Design::V1 } else { Design::V2 };
            match save(design).await {
                Ok(()) => {
                    notification.set(Some(Notification::Success("Design saved".into())));
                    refetch.notify();
                    navigate("/", NavigateOptions::default());
                }
                Err(e) => {
                    // Revert the optimistic toggle so the UI doesn't drift
                    // from on-disk state.
                    classic.set(!on);
                    notification.set(Some(Notification::Error(e)));
                }
            }
            saving.set(false);
        });
    };

    view! {
        <section class="settings-section qui-design-settings">
            <h2 class="section-title">"Design"</h2>
            <dl class="settings-list">
                <dt>"Use the classic design"</dt>
                <dd>
                    <label class="checkbox-option">
                        <input
                            type="checkbox"
                            prop:checked=move || classic.get()
                            prop:disabled=move || saving.get()
                            on:change=on_toggle
                        />
                        <span class="value default">
                            "QuiltSync as it looked before the redesign. Turn off for the new design."
                        </span>
                    </label>
                </dd>
            </dl>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{element_saying, mount, sleep_ms};
    use wasm_bindgen_test::*;

    thread_local! {
        static SAVED: std::cell::RefCell<Vec<Design>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    fn stub_save(design: Design) -> Pin<Box<dyn Future<Output = Result<(), String>>>> {
        SAVED.with(|s| s.borrow_mut().push(design));
        Box::pin(async { Ok(()) })
    }

    fn mount_section(design: Design) -> web_sys::Element {
        mount(move || {
            view! {
                <leptos_router::components::Router>
                    <DesignSection
                        design=design
                        notification=RwSignal::new(None)
                        refetch=Trigger::new()
                        save=stub_save
                    />
                </leptos_router::components::Router>
            }
        })
    }

    fn path() -> String {
        web_sys::window().unwrap().location().pathname().unwrap()
    }

    fn checkbox(root: &web_sys::Element) -> web_sys::HtmlInputElement {
        use wasm_bindgen::JsCast;
        root.query_selector("input[type=checkbox]")
            .unwrap()
            .expect("the design row's checkbox")
            .dyn_into()
            .unwrap()
    }

    /// One row, named for the choice it makes, on when the classic design is
    /// the stored one.
    #[wasm_bindgen_test]
    fn the_row_says_classic_and_follows_the_stored_design() {
        let root = mount_section(Design::V1);
        element_saying(&root, "Use the classic design");
        assert!(checkbox(&root).checked());
        assert!(
            !root
                .text_content()
                .unwrap_or_default()
                .contains("New design preview"),
            "the preview's label is gone"
        );

        let root = mount_section(Design::V2);
        assert!(!checkbox(&root).checked());
    }

    /// Turning it off is the way back: it saves v2 and goes to `/`, which asks
    /// again.
    #[wasm_bindgen_test]
    async fn turning_the_classic_design_off_saves_v2_and_goes_home() {
        web_sys::window()
            .unwrap()
            .history()
            .unwrap()
            .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some("/settings"))
            .unwrap();
        SAVED.with(|s| s.borrow_mut().clear());
        let root = mount_section(Design::V1);
        checkbox(&root).click();
        sleep_ms(50).await;
        assert_eq!(SAVED.with(|s| s.borrow().clone()), vec![Design::V2]);
        assert_eq!(path(), "/");
    }
}
