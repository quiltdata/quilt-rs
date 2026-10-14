//! The error page v2, `/error`, and the page an unknown address draws.
//!
//! Both are an [`ErrorSlate`]: what went wrong, and the ways on. The gallery
//! draws it over fixtures. `/error` reads its title and message from
//! `get_login_error_data`; the deep link relay draws the same slate when the
//! link fails.

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::{use_navigate, use_query_map};

use crate::commands::{self, LoginErrorData};
use crate::components::appbar::appbar_actions;
use crate::kit::{
    BannerVariant, Blankslate, Button, ButtonVariant, LoadFailure, PageLayout, SkeletonText,
    readable,
};
use crate::routes::sign_in_href;

use super::v2_page::{Outcome, PageHandle, PageRead, V2Page, critical};

stylance::import_crate_style!(style, "src/pages/error_v2.module.scss");

/// What went wrong, and a button for each way on that the caller offers.
/// *Go home* is always one.
#[component]
pub fn ErrorSlate(
    #[prop(into)] title: String,
    #[prop(into)] message: String,
    /// Run the failed read again.
    #[prop(optional)]
    on_reload: Option<Callback<()>>,
    /// Open the app's `.quilt` folder.
    #[prop(optional)]
    on_dot_quilt: Option<Callback<()>>,
    /// Go to the sign-in page, when there is a host to sign in to.
    #[prop(optional)]
    on_login: Option<Callback<()>>,
    on_home: Callback<()>,
) -> impl IntoView {
    let actions = view! {
        <div class=style::actions>
            {on_reload
                .map(|run| {
                    view! { <Button on_click=move |_| run.run(())>"Reload page"</Button> }
                })}
            {on_dot_quilt
                .map(|run| {
                    view! { <Button on_click=move |_| run.run(())>"Open .quilt directory"</Button> }
                })}
            {on_login
                .map(|run| {
                    view! { <Button on_click=move |_| run.run(())>"Login"</Button> }
                })}
            <Button variant=ButtonVariant::Primary on_click=move |_| on_home.run(())>
                "Go home"
            </Button>
        </div>
    };
    view! {
        <div class=style::slate data-testid="error-msg">
            <Blankslate heading=title description=message primary_action=actions.into_any() />
        </div>
    }
}

/// The slate before the read answers.
#[component]
pub fn ErrorSlateSkeleton() -> impl IntoView {
    view! {
        <div class=style::skeleton aria-busy="true">
            <SkeletonText width="200px" />
            <SkeletonText width="320px" />
        </div>
    }
}

/// What `/error` renders when *New design preview* is on.
#[component]
pub fn ErrorV2() -> impl IntoView {
    view! { <ErrorScreen read=read_error /> }
}

/// What `/error` draws while it reads which page it is, for a v2 reader: the
/// page's own first paint.
#[component]
pub fn ErrorV2Skeleton(actions: AnyView) -> impl IntoView {
    view! {
        <PageLayout heading="Error" banner=().into_any() actions=actions>
            <ErrorSlateSkeleton />
        </PageLayout>
    }
}

/// What an unknown address renders when *New design preview* is on. It reads
/// nothing, so it is also its own first paint.
#[component]
pub fn NotFoundV2() -> impl IntoView {
    let navigate = use_navigate();
    let on_home = Callback::new(move |()| navigate("/", NavigateOptions::default()));
    // Nothing is read here, so Refresh reloads the window, as v1's does.
    let actions = appbar_actions(
        || {
            let _ = web_sys::window().map(|w| w.location().reload());
        },
        Signal::stored(false),
    );
    view! {
        <PageLayout heading="Page not found" actions=actions>
            <ErrorSlate
                title="Page not found"
                message="The page you are looking for does not exist."
                on_home=on_home
            />
        </PageLayout>
    }
}

/// The read's address: the query's `host`, `title` and `error`, all of which
/// the read takes.
fn error_key(host: String, title: Option<String>, error: String) -> String {
    serde_json::to_string(&(host, title, error)).unwrap_or_default()
}

fn read_error(
    key: String,
) -> std::pin::Pin<Box<dyn Future<Output = Result<LoginErrorData, String>>>> {
    Box::pin(async move {
        let (host, title, error) =
            serde_json::from_str::<(String, Option<String>, String)>(&key).unwrap_or_default();
        commands::get_login_error_data(host, title, error).await
    })
}

/// The slate's *Open .quilt directory*: what the command said goes in the
/// page's band.
pub(crate) fn open_dot_quilt(
    outcome: RwSignal<Option<Outcome>>,
    here: Memo<String>,
) -> Callback<()> {
    Callback::new(move |()| {
        outcome.set(None);
        // Taken now: the page can be gone, or on another address, by the answer.
        let here = here.get_untracked();
        leptos::task::spawn_local(async move {
            let said = match commands::debug_dot_quilt().await {
                Ok(_) => Outcome {
                    namespace: here.clone(),
                    variant: BannerVariant::Success,
                    lead: "Opened the .quilt directory.".to_string(),
                    detail: None,
                },
                Err(e) => critical(
                    here.clone(),
                    "Could not open the .quilt directory.",
                    Some(e),
                ),
            };
            outcome.try_set(Some(said));
        });
    })
}

#[component]
pub(crate) fn ErrorScreen(read: PageRead<LoginErrorData>) -> impl IntoView {
    let query = use_query_map();
    let key = Memo::new(move |_| {
        query.with(|q| {
            error_key(
                q.get("host").unwrap_or_default(),
                q.get("title"),
                q.get("error").unwrap_or_default(),
            )
        })
    });
    let page = PageHandle::new();
    let PageHandle {
        outcome, reload, ..
    } = page;
    let answer = page.read(key, read);

    let on_reload = Callback::new(move |()| reload.notify());
    let on_dot_quilt = open_dot_quilt(outcome, key);
    let navigate = use_navigate();
    let on_home = {
        let navigate = navigate.clone();
        Callback::new(move |()| navigate("/", NavigateOptions::default()))
    };

    view! {
        <V2Page
            heading="Error"
            page=page
            answer=answer
            showing=key
            skeleton=|| view! { <ErrorSlateSkeleton /> }.into_any()
            bands=|_| ().into_any()
            body=move |d: LoginErrorData| {
                let navigate = navigate.clone();
                let host = d.login_host;
                let on_login = (!host.is_empty())
                    .then(|| {
                        Callback::new(move |()| {
                            navigate(&sign_in_href(&host), NavigateOptions::default());
                        })
                    });
                match on_login {
                    Some(on_login) => view! {
                        <ErrorSlate
                            title=d.title
                            message=d.message
                            on_reload=on_reload
                            on_dot_quilt=on_dot_quilt
                            on_login=on_login
                            on_home=on_home
                        />
                    }
                    .into_any(),
                    None => view! {
                        <ErrorSlate
                            title=d.title
                            message=d.message
                            on_reload=on_reload
                            on_dot_quilt=on_dot_quilt
                            on_home=on_home
                        />
                    }
                    .into_any(),
                }
            }
            failure=move || {
                let detail = match answer.get_untracked() {
                    Some(Err(e)) => readable(&e),
                    _ => String::new(),
                };
                view! {
                    <LoadFailure
                        words="Could not load the error."
                        detail=detail
                        on_retry=Callback::new(move |()| reload.notify())
                    />
                }
                .into_any()
            }
        />
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{button_saying, element_saying, mount, sleep_ms, unmount_earlier};
    use leptos_router::components::Router;
    use wasm_bindgen_test::*;

    thread_local! {
        static HOST: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    }

    fn answered(
        _: String,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<LoginErrorData, String>>>> {
        let host = HOST.with(|h| h.borrow().clone());
        Box::pin(async move {
            Ok(LoginErrorData {
                title: "Login failed".to_string(),
                message: "The code expired.".to_string(),
                login_host: host,
            })
        })
    }

    async fn error_page(host: &str) -> web_sys::Element {
        unmount_earlier();
        HOST.with(|h| *h.borrow_mut() = host.to_string());
        let el = mount(|| {
            view! {
                <Router>
                    <Suspense fallback=|| ()>
                        <ErrorScreen read=answered />
                    </Suspense>
                </Router>
            }
        });
        sleep_ms(30).await;
        el
    }

    /// v1's four ways on, Login among them while there is a host.
    #[wasm_bindgen_test]
    async fn the_page_offers_v1_s_four_actions() {
        let el = error_page("quilt.test").await;
        element_saying(&el, "Login failed");
        element_saying(&el, "The code expired.");
        for words in ["Reload page", "Open .quilt directory", "Login", "Go home"] {
            button_saying(&el, words);
        }
        assert!(
            el.query_selector("[data-testid=error-msg]")
                .unwrap()
                .is_some(),
            "markup was {}",
            el.inner_html()
        );
    }

    /// Without a host there is nothing to sign in to.
    #[wasm_bindgen_test]
    async fn no_host_no_login() {
        let el = error_page("").await;
        let buttons = el.query_selector_all("button").unwrap();
        let said: Vec<String> = (0..buttons.length())
            .map(|i| buttons.item(i).unwrap().text_content().unwrap_or_default())
            .collect();
        assert!(!said.iter().any(|s| s.trim() == "Login"), "{said:?}");
        button_saying(&el, "Reload page");
    }

    #[wasm_bindgen_test]
    fn not_found_offers_the_way_home() {
        let el = mount(|| view! { <Router><NotFoundV2 /></Router> });
        element_saying(&el, "Page not found");
        button_saying(&el, "Go home");
        button_saying(&el, "Settings");
        assert!(!button_saying(&el, "Refresh").disabled());
        assert!(el.query_selector("[data-v2-page]").unwrap().is_some());
    }

    #[test]
    fn the_key_carries_all_three_parameters() {
        let key = error_key("h".into(), None, "boom & bust".into());
        assert_eq!(
            serde_json::from_str::<(String, Option<String>, String)>(&key).unwrap(),
            ("h".to_string(), None, "boom & bust".to_string())
        );
    }
}
