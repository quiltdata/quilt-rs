//! The login page v2: sign in to one catalog, in the browser or with a code
//! from it.
//!
//! [`LoginCard`] is the page's one region over props: the gallery draws it
//! over fixtures, and [`LoginV2`] fills it from `get_login_data`. The code
//! typed and which sign-in is out are the page's, above the rebuild line.
//!
//! A code that signs in goes to `back`, or to `/` without one. Browser sign-in
//! only opens the browser: the catalog's answer comes back as a deep link,
//! and the backend goes to `back` from there.

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use crate::commands::{self, LoginData};
use crate::engine;
use crate::kit::{
    BannerVariant, Button, ButtonSize, ButtonVariant, Card, FormControl, LoadFailure, PageLayout,
    SkeletonBox, TextInput, icons, readable,
};

use super::v2_page::{Outcome, PageHandle, PageRead, V2Page, critical, hold, still_here};

stylance::import_crate_style!(style, "src/pages/login_v2.module.css");

/// Which sign-in is out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignIn {
    Browser,
    Code,
}

/// The sign-in card: browser sign-in first, then the code.
#[component]
pub fn LoginCard(
    #[prop(into)] host: String,
    #[prop(into)] catalog_url: String,
    code: RwSignal<String>,
    /// The code alone: headless has nothing to receive a browser sign-in.
    #[prop(optional, into)]
    code_only: Signal<bool>,
    /// Which sign-in is out, if one is.
    #[prop(into)]
    running: Signal<Option<SignIn>>,
    on_browser: Callback<()>,
    on_open_catalog: Callback<()>,
    on_submit: Callback<()>,
) -> impl IntoView {
    let locked = move || running.get().is_some();
    let title = ["Log in to ".to_string(), host].concat();
    let instructions = move || {
        let lead = if code_only.get() {
            "Visit "
        } else {
            "Or visit "
        };
        [lead, &catalog_url, " to get your code:"].concat()
    };
    view! {
        <div class=style::page>
            <Card title=title>
                <div class=style::stack>
                    <Show when=move || !code_only.get()>
                        <div>
                            <Button
                                variant=ButtonVariant::Primary
                                size=ButtonSize::Large
                                leading_visual=icons::link_external()
                                loading=Signal::derive(move || {
                                    running.get() == Some(SignIn::Browser)
                                })
                                disabled=Signal::derive(locked)
                                on_click=move |_| on_browser.run(())
                            >
                                "Log in with browser"
                            </Button>
                        </div>
                        <hr class=style::divider />
                    </Show>
                    <div class=style::instructions>
                        <p class=style::quiet>{instructions}</p>
                        <div>
                            <Button
                                leading_visual=icons::link_external()
                                on_click=move |_| on_open_catalog.run(())
                            >
                                "Open browser"
                            </Button>
                        </div>
                    </div>
                    <form
                        class=style::form
                        on:submit=move |ev| {
                            ev.prevent_default();
                            on_submit.run(());
                        }
                    >
                        <FormControl
                            label="Code"
                            control=move |id| {
                                view! { <TextInput id=id value=code /> }.into_any()
                            }
                        />
                        <div>
                            <Button
                                submit=true
                                loading=Signal::derive(move || running.get() == Some(SignIn::Code))
                                disabled=Signal::derive(move || {
                                    locked() || code.with(|c| c.trim().is_empty())
                                })
                                on_click=|_| {}
                            >
                                "Submit code and Log in"
                            </Button>
                        </div>
                    </form>
                </div>
            </Card>
        </div>
    }
}

/// The card before the read answers: its outline, busy.
#[component]
pub fn LoginCardSkeleton() -> impl IntoView {
    view! {
        <div class=style::page>
            <Card label="Log in" busy=true>
                <div class=style::stack>
                    <SkeletonBox width="60%" height="20px" />
                    <SkeletonBox width="40%" height="32px" />
                    <SkeletonBox width="80%" />
                    <SkeletonBox width="100%" height="32px" />
                </div>
            </Card>
        </div>
    }
}

/// What `/login` renders in the v2 design.
#[component]
pub fn LoginV2() -> impl IntoView {
    view! { <LoginScreen read=read_login /> }
}

/// What `/login` draws while it reads which page it is, for a v2 reader: the
/// page's own first paint.
#[component]
pub fn LoginV2Skeleton(actions: AnyView) -> impl IntoView {
    view! {
        <PageLayout heading="Log in" banner=().into_any() actions=actions>
            <LoginCardSkeleton />
        </PageLayout>
    }
}

/// The read's address: the query's `host` and `back`, both of which the read
/// takes.
fn login_key(host: String, back: String) -> String {
    serde_json::to_string(&(host, back)).unwrap_or_default()
}

fn read_login(key: String) -> std::pin::Pin<Box<dyn Future<Output = Result<LoginData, String>>>> {
    Box::pin(async move {
        let (host, back) = serde_json::from_str::<(String, String)>(&key).unwrap_or_default();
        commands::get_login_data(host, back).await
    })
}

/// Where a code that signed in goes.
pub(crate) fn after_login(back: &str) -> &str {
    if back.is_empty() { "/" } else { back }
}

/// *Log in with browser*: opens the catalog's sign-in in the browser.
fn browser_sign_in(
    page: PageHandle,
    answer: Memo<Option<Result<LoginData, String>>>,
    key: Memo<String>,
    running: RwSignal<Option<SignIn>>,
) -> Callback<()> {
    let PageHandle { busy, outcome, .. } = page;
    Callback::new(move |()| {
        let Some(Ok(data)) = answer.get_untracked() else {
            return;
        };
        if busy.get_untracked() {
            return;
        }
        running.set(Some(SignIn::Browser));
        // Taken now: the page can be gone, or on another address, by the answer.
        let here = key.get_untracked();
        leptos::task::spawn_local(async move {
            let back = (!data.back.is_empty()).then_some(data.back);
            let opened = hold(busy, outcome, commands::login_oauth(data.host, back)).await;
            running.try_set(None);
            outcome.try_set(Some(match opened {
                Ok(_) => Outcome {
                    namespace: here.clone(),
                    variant: BannerVariant::Success,
                    lead: "Continue in your browser to log in.".to_string(),
                    detail: None,
                },
                Err(e) => critical(
                    here.clone(),
                    "Could not open the browser to log in.",
                    Some(e),
                ),
            }));
        });
    })
}

/// *Submit code and Log in*: signs in with the typed code, then goes `back`.
fn code_sign_in(
    page: PageHandle,
    answer: Memo<Option<Result<LoginData, String>>>,
    key: Memo<String>,
    code: RwSignal<String>,
    running: RwSignal<Option<SignIn>>,
) -> Callback<()> {
    let PageHandle { busy, outcome, .. } = page;
    let navigate = use_navigate();
    Callback::new(move |()| {
        let Some(Ok(data)) = answer.get_untracked() else {
            return;
        };
        let typed = code.get_untracked();
        if busy.get_untracked() || typed.trim().is_empty() {
            return;
        }
        running.set(Some(SignIn::Code));
        let here = key.get_untracked();
        let navigate = navigate.clone();
        leptos::task::spawn_local(async move {
            let signed_in = hold(busy, outcome, commands::login(data.host, typed)).await;
            running.try_set(None);
            if !still_here(key, &here) {
                return;
            }
            match signed_in {
                Ok(_) => navigate(after_login(&data.back), NavigateOptions::default()),
                Err(e) => {
                    outcome.try_set(Some(critical(here.clone(), "Could not log in.", Some(e))));
                }
            }
        });
    })
}

#[component]
pub(crate) fn LoginScreen(read: PageRead<LoginData>) -> impl IntoView {
    let query = crate::routes::use_query();
    let key = Memo::new(move |_| {
        query.with(|q| {
            login_key(
                q.get("host").unwrap_or_default(),
                q.get("back").unwrap_or_default(),
            )
        })
    });
    let page = PageHandle::new();
    let PageHandle {
        outcome, reload, ..
    } = page;
    let answer = page.read(key, read);

    let code = RwSignal::new(String::new());
    let running = RwSignal::new(None::<SignIn>);
    let on_browser = browser_sign_in(page, answer, key, running);
    let on_open_catalog = Callback::new(move |()| {
        let Some(Ok(data)) = answer.get_untracked() else {
            return;
        };
        // This command is now the last one, so an earlier failure is not news.
        outcome.set(None);
        // The engine would open it on its own machine, which for headless is
        // seldom the reader's.
        if engine::headless().get_untracked() {
            if let Some(window) = web_sys::window() {
                let _ = window.open_with_url_and_target(&data.catalog_url, "_blank");
            }
            return;
        }
        let here = key.get_untracked();
        leptos::task::spawn_local(async move {
            if let Err(e) = commands::open_in_web_browser(data.catalog_url).await {
                outcome.try_set(Some(critical(
                    here.clone(),
                    "Could not open the catalog.",
                    Some(e),
                )));
            }
        });
    });

    let on_submit = code_sign_in(page, answer, key, code, running);

    view! {
        <V2Page
            heading="Log in"
            page=page
            answer=answer
            showing=key
            skeleton=|| view! { <LoginCardSkeleton /> }.into_any()
            bands=|_| ().into_any()
            body=move |d: LoginData| {
                view! {
                    <LoginCard
                        host=d.host
                        catalog_url=d.catalog_url
                        code=code
                        code_only=engine::headless()
                        running=running
                        on_browser=on_browser
                        on_open_catalog=on_open_catalog
                        on_submit=on_submit
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
                        words="Could not load the login page."
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
    use crate::test_support::{button_saying, mount, sleep_ms, unmount_earlier};
    use leptos_router::components::Router;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    thread_local! {
        static ASKED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    fn answered(key: String) -> std::pin::Pin<Box<dyn Future<Output = Result<LoginData, String>>>> {
        let (host, back) = serde_json::from_str::<(String, String)>(&key).unwrap();
        ASKED.with(|a| a.borrow_mut().push(key));
        Box::pin(async move {
            Ok(LoginData {
                catalog_url: format!("https://{host}/code"),
                host,
                back,
            })
        })
    }

    fn refused(_: String) -> std::pin::Pin<Box<dyn Future<Output = Result<LoginData, String>>>> {
        Box::pin(async { Err("no host".to_string()) })
    }

    async fn login(read: PageRead<LoginData>) -> web_sys::Element {
        unmount_earlier();
        ASKED.with(|a| a.borrow_mut().clear());
        let el = mount(move || {
            view! {
                <Router>
                    <Suspense fallback=|| ()>
                        <LoginScreen read=read />
                    </Suspense>
                </Router>
            }
        });
        sleep_ms(30).await;
        el
    }

    /// The card names the host, offers both ways in, and holds Submit until
    /// there is a code to send.
    #[wasm_bindgen_test]
    async fn the_card_names_the_host_and_waits_for_a_code() {
        let el = login(answered).await;
        let text = el.text_content().unwrap_or_default();
        assert!(
            text.contains("Log in to "),
            "markup was {}",
            el.inner_html()
        );
        button_saying(&el, "Log in with browser");
        button_saying(&el, "Open browser");
        let submit = button_saying(&el, "Submit code and Log in");
        assert!(submit.disabled(), "nothing typed yet");

        let input: web_sys::HtmlInputElement = el
            .query_selector("form input")
            .unwrap()
            .expect("the code field")
            .unchecked_into();
        input.set_value("ABC123");
        input
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
        sleep_ms(10).await;
        assert!(!submit.disabled(), "a code is typed");
    }

    /// The page reads once on mount, keyed by both query parameters.
    /// Headless has nothing to receive a browser sign-in, so the card offers
    /// the code alone.
    #[wasm_bindgen_test]
    async fn a_code_only_card_has_no_browser_sign_in() {
        unmount_earlier();
        let el = mount(|| {
            view! {
                <LoginCard
                    host="quilt.test"
                    catalog_url="https://quilt.test/code"
                    code=RwSignal::new(String::new())
                    code_only=true
                    running=Signal::derive(|| None)
                    on_browser=Callback::new(|()| {})
                    on_open_catalog=Callback::new(|()| {})
                    on_submit=Callback::new(|()| {})
                />
            }
        });
        sleep_ms(0).await;
        let text = el.text_content().unwrap_or_default();
        assert!(!text.contains("Log in with browser"), "{text}");
        assert!(text.contains("Visit https://quilt.test/code"), "{text}");
        button_saying(&el, "Submit code and Log in");
    }

    #[wasm_bindgen_test]
    async fn the_page_reads_once() {
        let _el = login(answered).await;
        let asked = ASKED.with(|a| a.borrow().clone());
        assert_eq!(asked, vec![login_key(String::new(), String::new())]);
    }

    /// The key carries both parameters to the read whatever they hold.
    #[test]
    fn the_key_carries_host_and_back() {
        let key = login_key("quilt.test".into(), "/remote-package?uri=a&b".into());
        assert_eq!(
            serde_json::from_str::<(String, String)>(&key).unwrap(),
            (
                "quilt.test".to_string(),
                "/remote-package?uri=a&b".to_string()
            )
        );
    }

    #[wasm_bindgen_test]
    async fn a_failed_read_offers_to_try_again() {
        let el = login(refused).await;
        button_saying(&el, "Try again");
        assert!(
            el.text_content()
                .unwrap_or_default()
                .contains("Could not load the login page."),
            "markup was {}",
            el.inner_html()
        );
    }

    #[test]
    fn a_login_without_back_goes_home() {
        assert_eq!(after_login(""), "/");
        assert_eq!(after_login("/settings"), "/settings");
    }
}
