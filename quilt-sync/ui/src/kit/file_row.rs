//! One recently-changed file.
//!
//! The row opens the file; the package tag navigates to its package. Both are
//! primary enough to be one click, which is why the tag is a real link and the row
//! is not an anchor.
//!
//! Three secondary actions live in the row and appear on hover or focus. They are
//! not on the packages view's rows, and the asymmetry is deliberate: a file is a
//! thing you act on, a package is a place you go.

use leptos::ev::MouseEvent;
use leptos::prelude::*;

use super::IconButton;
use super::IconButtonVariant;
use super::RelativeTime;
use super::countdown::EpochMillis;

stylance::import_crate_style!(style, "src/kit/file_row.module.scss");

// Inline rather than shared: these three are the only icons in the kit so far, and
// a real icon set is its own component. The box-arrow specifically means "leaves
// the application" — a plain arrow would be ambiguous next to the package tag,
// which is also navigation.
fn folder_icon() -> AnyView {
    view! {
        <svg viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor"
            stroke-width="1.3" stroke-linejoin="round">
            <path d="M1.75 4.25h3.9l1.4 1.9h7.2v7.6H1.75z" />
        </svg>
    }
    .into_any()
}

fn catalog_icon() -> AnyView {
    view! {
        <svg viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor"
            stroke-width="1.3" stroke-linejoin="round">
            <path d="M12.25 9.5v4.25H2.25V3.75H6.5" />
            <path d="M9.5 2.25h4.25V6.5M13.75 2.25 8 8" />
        </svg>
    }
    .into_any()
}

fn copy_icon() -> AnyView {
    view! {
        <svg viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor"
            stroke-width="1.3">
            <rect x="5.6" y="5.6" width="8.1" height="8.1" rx="1.4" />
            <path d="M10.4 3.3V2.9c0-.33-.27-.6-.6-.6H2.9c-.33 0-.6.27-.6.6v6.9c0 .33.27.6.6.6h.4" />
        </svg>
    }
    .into_any()
}

#[component]
pub fn FileRow(
    /// Logical key, shown whole and truncated from the left when it will not fit.
    #[prop(into)]
    path: String,
    #[prop(into)] package: String,
    #[prop(into)] package_href: String,
    at: EpochMillis,
    /// Opens the file in whatever application the OS associates with it. Bound to
    /// the row, not to a button — a button that duplicated the row's own affordance
    /// would be chrome.
    on_open: impl Fn(MouseEvent) + 'static,
    on_reveal: impl Fn(MouseEvent) + 'static,
    on_open_catalog: impl Fn(MouseEvent) + 'static,
    on_copy_uri: impl Fn(MouseEvent) + 'static,
) -> impl IntoView {
    view! {
        <div class=style::root role="button" tabindex="0" on:click=on_open>
            <span class=style::path>{path}</span>
            // Stops propagation, or going to the package would also open the file.
            <a
                class=style::tag
                href=package_href
                on:click=|ev: MouseEvent| ev.stop_propagation()
            >
                {package}
            </a>
            <span class=style::time>
                <RelativeTime at=at />
            </span>
            <span class=style::actions on:click=|ev: MouseEvent| ev.stop_propagation()>
                <IconButton
                    icon=folder_icon()
                    aria_label="Reveal in directory"
                    variant=IconButtonVariant::Bare
                    on_click=on_reveal
                />
                <IconButton
                    icon=catalog_icon()
                    aria_label="Open in catalog"
                    variant=IconButtonVariant::Bare
                    on_click=on_open_catalog
                />
                <IconButton
                    icon=copy_icon()
                    aria_label="Copy Quilt+S3 URI"
                    variant=IconButtonVariant::Bare
                    on_click=on_copy_uri
                />
            </span>
        </div>
    }
}
