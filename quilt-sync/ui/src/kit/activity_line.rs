//! The activity line: faint text in the appbar that says what the app is doing
//! **right now**, and nothing once it is done.
//!
//! # It is not a notification
//!
//! A notification says what happened; it is kept by the backend, stacked, and
//! dismissed by hand. This line says what is running and is never kept, never
//! dismissed and never carded: a faint attention tint, but no border, icon or
//! close control. It never shows an error either, because failures have
//! surfaces of their own. A pull uses both in turn: the line while the files change, and the
//! notification stack's entry once they have.
//!
//! # What it draws
//!
//! A list of [`Activity`]s, of which it draws the **first** label. Each
//! [`ActivityKind`] is one producer with a slot of its own: a producer sets its
//! slot and no other, so autopull's feed, which says the whole of its state on
//! every event, never wipes the removal of old revisions the page started. The
//! list runs in the kinds' order, so autopull's entry ranks first: it is the one
//! the user did not start. One colour and one place still serve both; giving
//! each kind its own colour and stacking several are left for later.
//!
//! The words come from whoever sets [`Activities`]; the kit composes none. Each
//! activity also names the package it is about, which is how a page tells that
//! autopull is busy with the package on screen.
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

/// Who is doing the work, in the order the line ranks them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ActivityKind {
    /// The autopull tick, pulling or publishing.
    Autopull,
    /// The package page, removing old revisions.
    RemoveRevisions,
}

/// One thing the app is doing, in the words the line draws.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Activity {
    pub kind: ActivityKind,
    pub label: String,
    /// The package it is about, as `owner/name`.
    pub package: Option<String>,
}

/// What the app is doing right now, provided as context for [`ActivityLine`].
///
/// A slot per kind, each set whole, not by deltas: a setter says what is true
/// now of its own kind, and leaves the others as they are.
#[derive(Clone, Copy, Debug)]
pub struct Activities(RwSignal<Vec<Activity>>);

impl Activities {
    #[must_use]
    pub fn new() -> Self {
        Self(RwSignal::new(Vec::new()))
    }

    /// Replace `kind`'s slot with `activities`, keeping every other kind's.
    /// Every one of `activities` must be of `kind`.
    pub fn set(&self, kind: ActivityKind, activities: Vec<Activity>) {
        debug_assert!(activities.iter().all(|activity| activity.kind == kind));
        self.0.update(|all| {
            all.retain(|activity| activity.kind != kind);
            all.extend(activities);
            // Stable, so a kind's own order survives.
            all.sort_by_key(|activity| activity.kind);
        });
    }

    /// Whether `kind` is busy with `package` now. Reactive, as [`Self::get`].
    #[must_use]
    pub fn busy_with(&self, kind: ActivityKind, package: &str) -> bool {
        self.0.with(|all| {
            all.iter().any(|activity| {
                activity.kind == kind && activity.package.as_deref() == Some(package)
            })
        })
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
            package: None,
        }
    }

    fn removing(label: &str) -> Activity {
        Activity {
            kind: ActivityKind::RemoveRevisions,
            label: label.to_owned(),
            package: Some("team/pkg".to_owned()),
        }
    }

    /// Autopull's feed says the whole of its state on every event; that must
    /// not wipe the page's removal, and autopull's entry ranks first.
    #[test]
    fn a_kind_sets_only_its_own_slot() {
        let owner = Owner::new();
        owner.with(|| {
            let activities = Activities::new();
            activities.set(
                ActivityKind::RemoveRevisions,
                vec![removing("Removing 4 old revisions of team/pkg\u{2026}")],
            );
            activities.set(
                ActivityKind::Autopull,
                vec![autopull("Getting latest for team/other\u{2026}")],
            );
            assert_eq!(
                activities.first_label(),
                "Getting latest for team/other\u{2026}"
            );
            activities.set(ActivityKind::Autopull, Vec::new());
            assert_eq!(
                activities.first_label(),
                "Removing 4 old revisions of team/pkg\u{2026}"
            );
            assert!(activities.busy_with(ActivityKind::RemoveRevisions, "team/pkg"));
            assert!(!activities.busy_with(ActivityKind::Autopull, "team/pkg"));
        });
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
        activities.set(
            ActivityKind::Autopull,
            vec![
                autopull("Getting latest for team/pkg\u{2026}"),
                autopull("Publishing team/other\u{2026}"),
            ],
        );
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
        activities.set(
            ActivityKind::Autopull,
            vec![autopull("Publishing team/pkg\u{2026}")],
        );
        leptos::task::tick().await;
        activities.set(ActivityKind::Autopull, Vec::new());
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
        activities.set(
            ActivityKind::Autopull,
            vec![autopull(
                "Getting latest for a-team-with-a-long-name/a-package-whose-name \
                 goes-on-for-longer-than-any-appbar-is-wide\u{2026}",
            )],
        );
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
