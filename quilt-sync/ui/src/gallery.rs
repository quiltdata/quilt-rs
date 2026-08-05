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

use kit::Button;
use kit::ButtonSize;
use kit::ButtonVariant;
use leptos::prelude::*;

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

    view! {
        <div class="g-page">
            <header class="g-head">
                <h1>"QuiltSync design system"</h1>
                <p>
                    "Every cell is one state. Tab through them — the focus ring is
                     part of what is being reviewed."
                </p>
                <Button on_click=move |_| dark.update(|d| *d = !*d)>
                    {move || if dark.get() { "Light theme" } else { "Dark theme" }}
                </Button>
            </header>
            <ButtonSection />
        </div>
    }
}

#[component]
fn Section(title: &'static str, note: &'static str, children: Children) -> impl IntoView {
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
#[component]
fn Cell(label: &'static str, children: Children) -> impl IntoView {
    view! {
        <div class="g-cell">
            <span class="g-cell__label">{label}</span>
            <div class="g-cell__body">{children()}</div>
        </div>
    }
}

/// Stand-in glyphs. A real icon set is a later component; these exist so the
/// leading slot can be reviewed now, including its collision with the spinner.
fn plus_icon() -> AnyView {
    view! {
        <svg viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor"
            stroke-width="1.6" stroke-linecap="round">
            <path d="M8 3.5v9M3.5 8h9" />
        </svg>
    }
    .into_any()
}

fn download_icon() -> AnyView {
    view! {
        <svg viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor"
            stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
            <path d="M8 2.5v7.5M4.75 7l3.25 3 3.25-3M2.5 13.5h11" />
        </svg>
    }
    .into_any()
}

#[component]
fn ButtonSection() -> impl IntoView {
    view! {
        <Section
            title="Button"
            note="Two variants. Hover and active are pointer states — point at the first \
                  two cells to review them. Disabled is not focusable; loading is."
        >
            <Cell label="default">
                <Button on_click=|_| ()>"Get latest"</Button>
            </Cell>
            <Cell label="default · disabled">
                <Button on_click=|_| () disabled=true>"Get latest"</Button>
            </Cell>
            <Cell label="default · loading">
                <Button on_click=|_| () loading=true>"Checking\u{2026}"</Button>
            </Cell>
            <Cell label="default · long label">
                <Button on_click=|_| ()>"Choose S3 bucket for this package"</Button>
            </Cell>

            <Cell label="primary">
                <Button on_click=|_| () variant=ButtonVariant::Primary>"Publish"</Button>
            </Cell>
            <Cell label="primary · disabled">
                <Button on_click=|_| () variant=ButtonVariant::Primary disabled=true>
                    "Publish"
                </Button>
            </Cell>
            <Cell label="primary · loading">
                <Button on_click=|_| () variant=ButtonVariant::Primary loading=true>
                    "Publishing\u{2026}"
                </Button>
            </Cell>
            <Cell label="primary · long label">
                <Button on_click=|_| () variant=ButtonVariant::Primary>
                    "Publish your changes to s3://vir-quilt-res-3-in-progress"
                </Button>
            </Cell>

            <Cell label="loading implies disabled — setting both changes nothing">
                <Button on_click=|_| () loading=true disabled=true>"Retry"</Button>
            </Cell>
            <Cell label="a pair, as the queue uses them">
                <div class="g-inline">
                    <Button on_click=|_| () variant=ButtonVariant::Primary>"Resolve"</Button>
                    <Button on_click=|_| ()>"Dismiss"</Button>
                </div>
            </Cell>
        </Section>

        <Section
            title="Button · with icon"
            note="The icon and the loading spinner are ONE slot, never two. Compare \
                  `icon · primary` with `icon · loading`: the spinner replaces the icon, so \
                  the width does not move. A button with no icon does grow when loading \
                  starts — see the main section — which is accepted because callers swap \
                  the label at the same moment anyway."
        >
            <Cell label="icon + label">
                <Button on_click=|_| () icon=plus_icon()>"Create package"</Button>
            </Cell>
            <Cell label="icon · primary">
                <Button on_click=|_| () variant=ButtonVariant::Primary icon=download_icon()>
                    "Get latest"
                </Button>
            </Cell>
            <Cell label="icon · loading — spinner replaces the icon">
                <Button
                    on_click=|_| ()
                    variant=ButtonVariant::Primary
                    icon=download_icon()
                    loading=true
                >
                    "Get latest"
                </Button>
            </Cell>
            <Cell label="icon · disabled">
                <Button on_click=|_| () icon=plus_icon() disabled=true>"Create package"</Button>
            </Cell>
            <Cell label="icon · long label">
                <Button on_click=|_| () icon=download_icon()>
                    "Get latest revision of this package"
                </Button>
            </Cell>
        </Section>

        <Section
            title="Button · large"
            note="One step up, for page-level and dialog-confirm actions. Size is \
                  orthogonal to variant, so every weight is available at both sizes, and \
                  the leading slot scales with it."
        >
            <Cell label="large">
                <Button on_click=|_| () size=ButtonSize::Large>"Create package"</Button>
            </Cell>
            <Cell label="large · primary">
                <Button on_click=|_| () size=ButtonSize::Large variant=ButtonVariant::Primary>
                    "Publish"
                </Button>
            </Cell>
            <Cell label="large · disabled">
                <Button on_click=|_| () size=ButtonSize::Large disabled=true>
                    "Create package"
                </Button>
            </Cell>
            <Cell label="large · icon">
                <Button on_click=|_| () size=ButtonSize::Large icon=plus_icon()>
                    "Create package"
                </Button>
            </Cell>
            <Cell label="large · primary · icon">
                <Button
                    on_click=|_| ()
                    size=ButtonSize::Large
                    variant=ButtonVariant::Primary
                    icon=download_icon()
                >
                    "Get latest"
                </Button>
            </Cell>
            <Cell label="large · loading — slot scales to 16px">
                <Button
                    on_click=|_| ()
                    size=ButtonSize::Large
                    variant=ButtonVariant::Primary
                    icon=download_icon()
                    loading=true
                >
                    "Publishing\u{2026}"
                </Button>
            </Cell>
        </Section>
    }
}
