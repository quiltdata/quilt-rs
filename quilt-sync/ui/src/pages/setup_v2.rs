//! The setup page v2: on the first run, the folder `QuiltSync` keeps packages
//! in.
//!
//! [`SetupCard`] is the page's one region over props: the gallery draws it
//! over fixtures, and [`SetupV2`] fills it from `get_setup_data`. The folder
//! starts at the read's suggestion; from then on it is the page's, so a
//! re-read does not put the suggestion back over a folder picked or typed.

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use crate::commands::{self, SetupData};
use crate::engine;
use crate::kit::{
    Button, ButtonVariant, Card, FormControl, LoadFailure, PageLayout, SkeletonBox, TextInput,
    readable,
};

use super::path_field::PathField;
use super::v2_page::{PageHandle, PageRead, V2Page, critical, hold, still_here};

stylance::import_crate_style!(style, "src/pages/setup_v2.module.css");

/// The read's only address.
const HERE: &str = "setup";

/// The folder, chosen with the system's picker or typed, and Save.
#[component]
pub fn SetupCard(
    directory: RwSignal<String>,
    /// Typed rather than picked: headless has no picker.
    #[prop(optional, into)]
    typed: Signal<bool>,
    /// The picker is open.
    #[prop(into)]
    choosing: Signal<bool>,
    /// The save is out.
    #[prop(into)]
    saving: Signal<bool>,
    on_choose: Callback<()>,
    on_save: Callback<()>,
) -> impl IntoView {
    let locked = move || choosing.get() || saving.get();
    view! {
        <div class=style::page>
            <Card title="Set up QuiltSync">
                <form
                    class=style::form
                    on:submit=move |ev| {
                        ev.prevent_default();
                        on_save.run(());
                    }
                >
                    <p class=style::quiet>
                        "Select a directory where QuiltSync will store your packages, ex. ~/QuiltSync"
                    </p>
                    <Show
                        when=move || !typed.get()
                        fallback=move || {
                            view! {
                                <PathField label="Set home directory" value=directory may_be_new=true />
                            }
                        }
                    >
                        <div class=style::choose>
                            <FormControl
                                label="Set home directory"
                                control=move |id| {
                                    view! { <TextInput id=id value=directory readonly=true /> }
                                        .into_any()
                                }
                            />
                            <Button
                                loading=choosing
                                disabled=Signal::derive(locked)
                                on_click=move |_| on_choose.run(())
                            >
                                "Browse"
                            </Button>
                        </div>
                    </Show>
                    <div>
                        <Button
                            variant=ButtonVariant::Primary
                            submit=true
                            loading=saving
                            disabled=Signal::derive(move || {
                                locked() || directory.with(String::is_empty)
                            })
                            on_click=|_| {}
                        >
                            "Save"
                        </Button>
                    </div>
                </form>
            </Card>
        </div>
    }
}

/// The card before the read answers: its outline, busy.
#[component]
pub fn SetupCardSkeleton() -> impl IntoView {
    view! {
        <div class=style::page>
            <Card title="Set up QuiltSync" busy=true>
                <div class=style::form>
                    <SkeletonBox width="80%" />
                    <SkeletonBox width="100%" height="32px" />
                    <SkeletonBox width="64px" height="32px" />
                </div>
            </Card>
        </div>
    }
}

/// What `/setup` renders in the v2 design.
#[component]
pub fn SetupV2() -> impl IntoView {
    view! { <SetupScreen read=read_setup /> }
}

/// What `/setup` draws while it reads which page it is, for a v2 reader: the
/// page's own first paint.
#[component]
pub fn SetupV2Skeleton(actions: AnyView) -> impl IntoView {
    view! {
        <PageLayout heading="Set up QuiltSync" banner=().into_any() actions=actions>
            <SetupCardSkeleton />
        </PageLayout>
    }
}

fn read_setup(_: String) -> std::pin::Pin<Box<dyn Future<Output = Result<SetupData, String>>>> {
    Box::pin(commands::get_setup_data())
}

#[component]
pub(crate) fn SetupScreen(read: PageRead<SetupData>) -> impl IntoView {
    let key = Memo::new(|_| HERE.to_string());
    let page = PageHandle::new();
    let PageHandle {
        busy,
        outcome,
        reload,
        ..
    } = page;
    let answer = page.read(key, read);

    let directory = RwSignal::new(String::new());
    let chosen = StoredValue::new(false);
    Effect::new(move |_| {
        if let Some(Ok(d)) = answer.get()
            && !chosen.get_value()
        {
            chosen.set_value(true);
            directory.set(d.default_home);
        }
    });

    let choosing = RwSignal::new(false);
    let on_choose = pick_folder(page, choosing, chosen, directory);

    let navigate = use_navigate();
    let on_save = Callback::new(move |()| {
        let chosen = directory.get_untracked();
        if busy.get_untracked() || choosing.get_untracked() || chosen.is_empty() {
            return;
        }
        let navigate = navigate.clone();
        leptos::task::spawn_local(async move {
            match hold(busy, outcome, commands::setup(chosen)).await {
                Ok(_) if still_here(key, HERE) => navigate("/", NavigateOptions::default()),
                Ok(_) => {}
                Err(e) => {
                    outcome.try_set(Some(critical(
                        HERE.into(),
                        "Could not set up the directory.",
                        Some(e),
                    )));
                }
            }
        });
    });

    view! {
        <V2Page
            heading="Set up QuiltSync"
            page=page
            answer=answer
            showing=key
            skeleton=|| view! { <SetupCardSkeleton /> }.into_any()
            bands=|_| ().into_any()
            body=move |_: SetupData| {
                view! {
                    <SetupCard
                        directory=directory
                        typed=engine::headless()
                        choosing=choosing
                        saving=busy
                        on_choose=on_choose
                        on_save=on_save
                    />
                }
                .into_any()
            }
            failure=move || {
                let detail = match answer.get_untracked() {
                    Some(Err(e)) => readable(&e),
                    _ => String::new(),
                };
                view! {
                    <LoadFailure
                        words="Could not load the setup page."
                        detail=detail
                        on_retry=Callback::new(move |()| reload.notify())
                    />
                }
                .into_any()
            }
        />
    }
}

/// Browse: the system's picker fills the folder.
fn pick_folder(
    page: PageHandle,
    choosing: RwSignal<bool>,
    chosen: StoredValue<bool>,
    directory: RwSignal<String>,
) -> Callback<()> {
    let PageHandle { busy, outcome, .. } = page;
    Callback::new(move |()| {
        if choosing.get_untracked() || busy.get_untracked() {
            return;
        }
        choosing.set(true);
        outcome.set(None);
        leptos::task::spawn_local(async move {
            match commands::open_directory_picker().await {
                Ok(path) => {
                    chosen.try_set_value(true);
                    directory.try_set(path);
                }
                Err(e) => {
                    outcome.try_set(Some(critical(
                        HERE.into(),
                        "Could not choose the directory.",
                        Some(e),
                    )));
                }
            }
            choosing.try_set(false);
        });
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{button_saying, mount, sleep_ms, unmount_earlier};
    use leptos_router::components::Router;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn suggests(_: String) -> std::pin::Pin<Box<dyn Future<Output = Result<SetupData, String>>>> {
        Box::pin(async {
            Ok(SetupData {
                default_home: "/home/a-user/QuiltSync".to_string(),
            })
        })
    }

    fn refused(_: String) -> std::pin::Pin<Box<dyn Future<Output = Result<SetupData, String>>>> {
        Box::pin(async { Err("no home".to_string()) })
    }

    async fn setup(read: PageRead<SetupData>) -> web_sys::Element {
        unmount_earlier();
        let el = mount(move || {
            view! {
                <Router>
                    <Suspense fallback=|| ()>
                        <SetupScreen read=read />
                    </Suspense>
                </Router>
            }
        });
        sleep_ms(30).await;
        el
    }

    /// The field starts at the suggestion and is chosen, not typed.
    #[wasm_bindgen_test]
    async fn the_field_starts_at_the_suggested_directory() {
        let el = setup(suggests).await;
        let input: web_sys::HtmlInputElement = el
            .query_selector("form input")
            .unwrap()
            .expect("the directory field")
            .unchecked_into();
        assert_eq!(input.value(), "/home/a-user/QuiltSync");
        assert!(input.read_only(), "the picker chooses it");
        button_saying(&el, "Browse");
        assert!(!button_saying(&el, "Save").disabled());
    }

    #[wasm_bindgen_test]
    async fn a_failed_read_offers_to_try_again() {
        let el = setup(refused).await;
        button_saying(&el, "Try again");
    }
}
