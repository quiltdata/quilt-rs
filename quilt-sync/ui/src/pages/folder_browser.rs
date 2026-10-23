//! A folder browser drawn by the page and fed by the backend's `list_folders`,
//! for when the native picker is not there: a browser talking to a headless
//! engine. A browser's own pickers never give an absolute path, which is what
//! every caller needs.
//!
//! [`FolderBrowser`] is mounted once, at the app's root; [`pick_folder`] is
//! what callers await instead of `open_directory_picker`.

use std::cell::{Cell, RefCell};

use leptos::prelude::*;
use wasm_bindgen::JsValue;

use crate::commands::{self, FolderListing};
use crate::kit::{Button, ButtonVariant, Dialog, FormControl, TextInput};
use crate::tauri::NOT_IN_BROWSER;

stylance::import_crate_style!(style, "src/pages/folder_browser.module.scss");

/// What a closed browser answers, as the native picker answers a cancel.
const CANCELLED: &str = "User cancelled";

thread_local! {
    /// The mounted browser's open signal.
    static OPEN: Cell<Option<RwSignal<bool>>> = const { Cell::new(None) };
    /// The resolve function of the pick in progress.
    static PENDING: RefCell<Option<js_sys::Function>> = const { RefCell::new(None) };
}

/// Ask for a folder: the native picker, or, where the backend has none, the
/// in-page browser. `Err` on cancel, as the native picker does.
///
/// # Errors
///
/// The picker's error, or a cancel.
pub async fn pick_folder() -> Result<String, String> {
    match commands::open_directory_picker().await {
        Err(err) if err == NOT_IN_BROWSER => browse().await,
        other => other,
    }
}

async fn browse() -> Result<String, String> {
    let Some(open) = OPEN.with(Cell::get) else {
        return Err(NOT_IN_BROWSER.to_string());
    };
    // One pick at a time: an earlier one still waiting is cancelled.
    settle(None);
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        PENDING.with(|p| *p.borrow_mut() = Some(resolve));
    });
    open.set(true);
    let value = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .unwrap_or(JsValue::NULL);
    value.as_string().ok_or_else(|| CANCELLED.to_string())
}

fn settle(path: Option<String>) {
    if let Some(resolve) = PENDING.with(|p| p.borrow_mut().take()) {
        let value = path.map_or(JsValue::NULL, |p| JsValue::from_str(&p));
        drop(resolve.call1(&JsValue::NULL, &value));
    }
}

#[component]
pub fn FolderBrowser() -> impl IntoView {
    let open = RwSignal::new(false);
    OPEN.with(|o| o.set(Some(open)));
    on_cleanup(move || OPEN.with(|o| o.set(None)));

    let listing = RwSignal::new(None::<FolderListing>);
    let field = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let loading = RwSignal::new(false);

    let go = move |path: Option<String>| {
        loading.set(true);
        leptos::task::spawn_local(async move {
            match commands::list_folders(path).await {
                Ok(l) => {
                    field.try_set(l.path.clone());
                    listing.try_set(Some(l));
                    error.try_set(None);
                }
                Err(err) => {
                    error.try_set(Some(err));
                }
            }
            loading.try_set(false);
        });
    };

    // Opening starts where the last pick was, or at home; closing without a
    // choice (Cancel, Escape) answers the pick with a cancel.
    Effect::new(move |was_open: Option<bool>| {
        let is_open = open.get();
        if is_open && was_open != Some(true) {
            go(listing.get_untracked().map(|l| l.path));
        }
        if !is_open && was_open == Some(true) {
            settle(None);
        }
        is_open
    });

    let current = move || listing.with(|l| l.as_ref().map(|l| l.path.clone()));
    let parent = move || listing.with(|l| l.as_ref().and_then(|l| l.parent.clone()));

    let choose = move |_| {
        if let Some(path) = current() {
            settle(Some(path));
            open.set(false);
        }
    };

    let footer = view! {
        <Button on_click=move |_| open.set(false)>"Cancel"</Button>
        <Button
            variant=ButtonVariant::Primary
            disabled=Signal::derive(move || current().is_none() || loading.get())
            on_click=choose
        >
            "Choose this folder"
        </Button>
    }
    .into_any();

    view! {
        <Dialog open=open title="Choose a folder" wide=true footer=footer>
            <form
                class=style::path_row
                on:submit=move |ev| {
                    ev.prevent_default();
                    go(Some(field.get_untracked()));
                }
            >
                <FormControl
                    label="Folder"
                    caption="Type a path and press Enter to go there."
                    error=Signal::derive(move || error.get())
                    control=move |id| view! { <TextInput id=id value=field /> }.into_any()
                />
            </form>
            <div class=style::nav>
                <Button
                    disabled=Signal::derive(move || parent().is_none() || loading.get())
                    on_click=move |_| go(parent())
                >
                    "Up"
                </Button>
                <Button disabled=loading on_click=move |_| go(None)>
                    "Home"
                </Button>
            </div>
            <ul class=style::list aria-label="Folders">
                {move || {
                    let Some(l) = listing.get() else {
                        return view! { <li class=style::empty>"Loading…"</li> }.into_any();
                    };
                    if l.folders.is_empty() {
                        return view! { <li class=style::empty>"No folders here"</li> }
                            .into_any();
                    }
                    let base = l.path;
                    l.folders
                        .into_iter()
                        .map(|name| {
                            let target = std::path::Path::new(&base)
                                .join(&name)
                                .display()
                                .to_string();
                            view! {
                                <li>
                                    <button
                                        type="button"
                                        class=style::folder
                                        on:click=move |_| go(Some(target.clone()))
                                    >
                                        {name}
                                    </button>
                                </li>
                            }
                        })
                        .collect_view()
                        .into_any()
                }}
            </ul>
        </Dialog>
    }
}
