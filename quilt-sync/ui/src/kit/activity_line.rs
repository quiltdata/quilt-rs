//! The activity line: faint text in the appbar that says what the app is doing
//! **right now**, and nothing once it is done.
//!
//! # It is not a notification
//!
//! A notification says what happened; it is kept by the backend, stacked, and
//! dismissed by hand. This line says what is running and is never kept, never
//! dismissed and never carded: no background, border, icon, tone or close
//! control. It never shows an error either, because failures have surfaces of
//! their own. A pull uses both in turn: the line while the files change, and the
//! notification stack's entry once they have.
//!
//! # What it draws
//!
//! A list of [`Activity`]s, of which it draws the **first** label today. There is
//! one [`ActivityKind`], so there is one colour and one place. `kind` is where
//! ranking the kinds, giving each its own colour and stacking several will hang,
//! once a second producer arrives; until then nothing reads it.
//!
//! The words come from whoever sets [`Activities`]; the kit composes none.
//!
//! # Why a context
//!
//! [`Appbar`](crate::kit::Appbar) is built by `PageLayout` and by v1's `Layout`, and a
//! prop would thread through both and every page that builds them. So the app
//! provides [`Activities`] once, and the kit stays free of Tauri. Without one —
//! the gallery's and the tests' default — the line draws nothing at all.
//!
//! # Why the region stays mounted
//!
//! With a context it is always a polite `role="status"` region, empty when idle. A
//! live region inserted together with its text is not reliably announced, so the
//! element stays and only its text changes. The fade-in waits a moment (see the
//! stylesheet); the text does not, so a transfer too short to become visible is
//! still announced.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/activity_line.module.scss");

/// Who is doing the work. One variant today: the autopull tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivityKind {
    Autopull,
}

/// One thing the app is doing, in the words the line draws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Activity {
    pub kind: ActivityKind,
    pub label: String,
}

/// What the app is doing right now, provided as context for [`ActivityLine`].
///
/// The whole list each time, not deltas: a setter says what is true now.
#[derive(Clone, Copy, Debug)]
pub struct Activities(RwSignal<Vec<Activity>>);

impl Activities {
    #[must_use]
    pub fn new() -> Self {
        Self(RwSignal::new(Vec::new()))
    }

    pub fn set(&self, activities: Vec<Activity>) {
        self.0.set(activities);
    }

    /// Reactive: read inside an effect or a view, it tracks.
    #[must_use]
    pub fn get(&self) -> Vec<Activity> {
        self.0.get()
    }

    /// The label the line draws: the first activity's, or empty. Reactive, as
    /// [`Self::get`].
    #[must_use]
    pub fn first_label(&self) -> String {
        self.0
            .with(|all| all.first().map(|first| first.label.clone()))
            .unwrap_or_default()
    }
}

impl Default for Activities {
    fn default() -> Self {
        Self::new()
    }
}

/// The line itself. Nothing without [`Activities`] above it; with them, a polite
/// region holding the first activity's label, or empty.
#[component]
pub fn ActivityLine() -> impl IntoView {
    use_context::<Activities>().map(|activities| {
        view! { <p class=style::line role="status">{move || activities.first_label()}</p> }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::Appbar;
    use crate::test_support::mount;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn autopull(label: &str) -> Activity {
        Activity {
            kind: ActivityKind::Autopull,
            label: label.to_owned(),
        }
    }

    /// An appbar under a fresh `Activities`, handed back with it.
    fn appbar_with_activities() -> (web_sys::Element, Activities) {
        let activities = Activities::new();
        let el = mount(move || {
            provide_context(activities);
            view! { <Appbar /> }
        });
        (el, activities)
    }

    fn regions(el: &web_sys::Element) -> Vec<web_sys::Element> {
        let all = el.query_selector_all("[role=status]").unwrap();
        (0..all.length())
            .map(|i| all.item(i).unwrap().unchecked_into())
            .collect()
    }

    fn the_region(el: &web_sys::Element) -> web_sys::Element {
        let found = regions(el);
        assert_eq!(found.len(), 1, "one region; markup was {}", el.inner_html());
        found.into_iter().next().unwrap()
    }

    #[wasm_bindgen_test]
    fn an_idle_line_is_an_empty_polite_region() {
        let (el, _) = appbar_with_activities();
        let region = the_region(&el);
        assert_eq!(region.text_content().unwrap_or_default(), "");
    }

    #[wasm_bindgen_test]
    async fn the_line_says_the_first_activity() {
        let (el, activities) = appbar_with_activities();
        activities.set(vec![
            autopull("Getting latest for team/pkg\u{2026}"),
            autopull("Publishing team/other\u{2026}"),
        ]);
        leptos::task::tick().await;
        assert_eq!(
            the_region(&el).text_content().unwrap_or_default(),
            "Getting latest for team/pkg\u{2026}"
        );
    }

    /// Announcing a start needs a region that was already there: one inserted with
    /// its text is not reliably read.
    #[wasm_bindgen_test]
    async fn clearing_leaves_the_region_mounted() {
        let (el, activities) = appbar_with_activities();
        activities.set(vec![autopull("Publishing team/pkg\u{2026}")]);
        leptos::task::tick().await;
        activities.set(Vec::new());
        leptos::task::tick().await;
        assert_eq!(the_region(&el).text_content().unwrap_or_default(), "");
    }

    /// The harness loads no stylesheet, so a computed style there is the browser's
    /// default. This one loads the line's own, with each class resolved to the name
    /// stylance gave it, and then asks the browser rather than the source.
    #[wasm_bindgen_test]
    async fn a_long_label_is_never_cut() {
        let doc = web_sys::window().unwrap().document().unwrap();
        let sheet = doc.create_element("style").unwrap();
        sheet.set_text_content(Some(
            &include_str!("activity_line.module.scss")
                .replace(".line", &format!(".{}", style::line)),
        ));
        doc.head().unwrap().append_child(&sheet).unwrap();

        let (el, activities) = appbar_with_activities();
        activities.set(vec![autopull(
            "Getting latest for a-team-with-a-long-name/a-package-whose-name \
             goes-on-for-longer-than-any-appbar-is-wide\u{2026}",
        )]);
        leptos::task::tick().await;
        let computed = web_sys::window()
            .unwrap()
            .get_computed_style(&the_region(&el))
            .unwrap()
            .expect("a computed style");
        // Read before the sheet goes: a computed style is live.
        let overflow = computed.get_property_value("text-overflow").unwrap();
        let white_space = computed.get_property_value("white-space").unwrap();
        let wrap = computed.get_property_value("overflow-wrap").unwrap();
        sheet.remove();

        assert_ne!(overflow, "ellipsis");
        assert_ne!(white_space, "nowrap");
        assert_eq!(wrap, "anywhere", "a namespace has no spaces to break at");
    }
}
