//! Component gallery — a debug harness, never shipped and never linked to.
//!
//! Its own Trunk target (`gallery.html`), mounting `Gallery` and never `App`.
//! That is not a style choice: `App` mounts `UpdateChecker`, which invokes on
//! mount, and `tauri_invoke_raw` is declared without `wasm_bindgen(catch)`, so
//! outside Tauri the missing `window.__TAURI__` traps the wasm module instead
//! of returning `Err`. Mounting `App` in a browser kills the page on load.
//!
//! Run it with `trunk serve gallery.html` and iterate in Chrome or Firefox for
//! speed — then check **GNOME Web (Epiphany)**, which is `WebKitGTK`, before
//! committing a layout. Chrome is not the webview that ships on Linux.
//!
//! # Render states, not components
//!
//! One entry per *state*. A gallery listing `Button` once is useless; listing
//! its twelve is a visual-regression surface. Add a cell for every state a
//! component can reach, including the ones that only appear when something has
//! gone wrong.

// A plain `mod`, no `#[path]`. This file lives in `src/` next to `kit.rs`
// precisely so that works: a `#[path]`-included module resolves its *children*
// relative to itself, so `kit.rs`'s `pub mod button;` would have looked for
// `src/button.rs` and failed. Declared as a second `[[bin]]` in Cargo.toml.
mod kit;

// One module per component. Adding a story means adding a file here and one line
// in `Gallery` below — there is no registry to keep in step.
mod gallery {
    pub mod button;
    pub mod card;
    pub mod countdown;
    pub mod host_row;
    pub mod list_toolbar;
    pub mod packages;
    pub mod page;
    pub mod queue;
    pub mod recent_files;
    pub mod search_input;
    pub mod select;
    pub mod state_label;
    pub mod state_strip;
    pub mod toggle_row;
    pub mod view_toggle;
}

use leptos::prelude::*;

use crate::gallery::button::ButtonStories;
use crate::gallery::card::CardStories;
use crate::gallery::countdown::CountdownStories;
use crate::gallery::host_row::HostRowStories;
use crate::gallery::list_toolbar::ListToolbarScene;
use crate::gallery::packages::PackageRowStories;
use crate::gallery::packages::PackagesScene;
use crate::gallery::page::PageScene;
use crate::gallery::queue::QueueScene;
use crate::gallery::queue::QueueStories;
use crate::gallery::recent_files::RecentFilesScene;
use crate::gallery::recent_files::RecentFilesStories;
use crate::gallery::search_input::SearchInputStories;
use crate::gallery::select::SelectStories;
use crate::gallery::state_label::StateLabelStories;
use crate::gallery::state_strip::StateStripScene;
use crate::gallery::toggle_row::ToggleRowStories;
use crate::gallery::view_toggle::ViewToggleStories;
use kit::Button;

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(Gallery);
}

/// Sets `data-theme` on the **root element**, which is the only place it works.
///
/// Tier-2 tokens are declared on `:root` as `var()` references to tier 1, and a
/// custom property's `var()` is resolved where the *declaration* sits — not
/// where it is used. So `--q-fg-default` is computed once against whichever
/// tier-1 values `:root` sees, and descendants inherit that already-resolved
/// colour. Putting `.dark` on a wrapper element therefore changes nothing.
fn set_theme(dark: bool) {
    if let Some(root) = document().document_element() {
        let value = if dark { "dark" } else { "light" };
        drop(root.set_attribute("data-theme", value));
    }
}

#[component]
fn Gallery() -> impl IntoView {
    let dark = RwSignal::new(false);

    Effect::new(move |_| set_theme(dark.get()));

    // One list, used twice: the index reads the labels, the page consumes the
    // views. A second hardcoded list of section names would drift from this one
    // the first time anybody added a component.
    //
    // Anchors rather than tabs. Tabs would show one component at a time, which
    // costs the thing a design-system gallery is *for*: noticing that a Select is
    // a pixel taller than a Button, or that two components disagree about a
    // baseline. The scenes exist precisely to compare, and Ctrl+F stops working
    // across hidden panels. So the page stays one scroll and gains a way to jump.
    let sections: Vec<(&'static str, AnyView)> = vec![
        ("Button", view! { <ButtonStories /> }.into_any()),
        ("Select", view! { <SelectStories /> }.into_any()),
        ("Card", view! { <CardStories /> }.into_any()),
        ("ToggleRow", view! { <ToggleRowStories /> }.into_any()),
        ("Countdown", view! { <CountdownStories /> }.into_any()),
        ("HostRow", view! { <HostRowStories /> }.into_any()),
        ("ViewToggle", view! { <ViewToggleStories /> }.into_any()),
        ("SearchInput", view! { <SearchInputStories /> }.into_any()),
        ("StateLabel", view! { <StateLabelStories /> }.into_any()),
        ("PackageRow", view! { <PackageRowStories /> }.into_any()),
        ("Queue parts", view! { <QueueStories /> }.into_any()),
        (
            "Recent files parts",
            view! { <RecentFilesStories /> }.into_any(),
        ),
        (
            "Scene · state strip",
            view! { <StateStripScene /> }.into_any(),
        ),
        (
            "Scene · list toolbar",
            view! { <ListToolbarScene /> }.into_any(),
        ),
        (
            "Scene · recent files",
            view! { <RecentFilesScene /> }.into_any(),
        ),
        (
            "Scene · needs your attention",
            view! { <QueueScene /> }.into_any(),
        ),
        ("Scene · packages", view! { <PackagesScene /> }.into_any()),
        ("Scene · whole page", view! { <PageScene /> }.into_any()),
    ];

    let index: Vec<&'static str> = sections.iter().map(|(label, _)| *label).collect();

    view! {
        <div class="g-shell">
            <nav class="g-nav" aria-label="Components">
                <Button on_click=move |_| dark.update(|d| *d = !*d)>
                    {move || if dark.get() { "Light theme" } else { "Dark theme" }}
                </Button>
                <ul>
                    {index
                        .into_iter()
                        .map(|label| {
                            view! {
                                <li>
                                    <a href=format!("#{}", slug(label))>{label}</a>
                                </li>
                            }
                        })
                        .collect_view()}
                </ul>
            </nav>
            <main class="g-main">
                <header class="g-head">
                    <h1>"QuiltSync design system"</h1>
                    <p>
                        "Every cell is one state. Tab through them — the focus ring is
                         part of what is being reviewed."
                    </p>
                </header>
                {sections
                    .into_iter()
                    .map(|(label, body)| {
                        view! {
                            <div id=slug(label) class="g-anchor">
                                {body}
                            </div>
                        }
                    })
                    .collect_view()}
            </main>
        </div>
    }
}

/// Anchor id from a section label. Lossy on purpose — it only has to be stable
/// and unique across the index, not reversible.
fn slug(label: &str) -> String {
    label
        .chars()
        .filter_map(|c| {
            if c.is_ascii_alphanumeric() {
                Some(c.to_ascii_lowercase())
            } else if c == ' ' {
                Some('-')
            } else {
                None
            }
        })
        .collect()
}

/// A titled group of related states. One component may have several — `Button`
/// has plain, with-icon and large.
#[component]
#[allow(clippy::must_use_candidate, reason = "consumed by view!")]
pub fn Story(title: &'static str, note: &'static str, children: Children) -> impl IntoView {
    view! {
        <section class="g-section">
            <h2>{title}</h2>
            <p class="g-note">{note}</p>
            <div class="g-grid">{children()}</div>
        </section>
    }
}

/// One labelled cell. The label is what makes the gallery reviewable: a
/// screenshot of unlabelled controls cannot be discussed.
/// A composed scene: several components arranged as the real page arranges them,
/// at the width the page gives them. Stories prove a component in isolation;
/// scenes prove they work together, which is where spacing and alignment
/// mistakes actually show up.
#[component]
#[allow(clippy::must_use_candidate, reason = "consumed by view!")]
pub fn Scene(title: &'static str, note: &'static str, children: Children) -> impl IntoView {
    view! {
        <section class="g-section">
            <h2>{title}</h2>
            <p class="g-note">{note}</p>
            <div class="g-scene">{children()}</div>
        </section>
    }
}

#[component]
#[allow(clippy::must_use_candidate, reason = "consumed by view!")]
pub fn Cell(
    label: &'static str,
    /// Span two grid columns. For components that are containers — a card at one
    /// column's width reads as something it is not.
    #[prop(optional)]
    wide: bool,
    /// Span every column. For list rows, whose behaviour *is* what they do with the
    /// width they are given. Wins over `wide` if both are set.
    #[prop(optional)]
    full: bool,
    children: Children,
) -> impl IntoView {
    let class = match (full, wide) {
        (true, _) => "g-cell g-cell--full",
        (false, true) => "g-cell g-cell--wide",
        (false, false) => "g-cell",
    };
    view! {
        <div class=class>
            <span class="g-cell__label">{label}</span>
            <div class="g-cell__body">{children()}</div>
        </div>
    }
}
