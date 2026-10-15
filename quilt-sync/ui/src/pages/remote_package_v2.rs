//! The deep link relay v2, `/remote-package`: runs the link in its `uri`,
//! then hands over to the package it installed or opened.
//!
//! Mounting runs the link, so nothing here may run it again by accident: the
//! frame is a transit one (*Refresh* disabled, the appbar's ways out replace
//! this entry), and every navigation from here replaces it, so Back cannot
//! land here. A failure draws the error slate inside the same frame; its
//! *Reload page* runs the link again, in place, without reloading the window.

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::{use_navigate, use_query_map};

use crate::commands;
use crate::components::appbar::transit_appbar_actions;
use crate::error_handler::redirect;
use crate::kit::{PageLayout, Spinner, SpinnerVariant, readable};

use super::error_v2::{ErrorSlate, open_dot_quilt};
use super::remote_package::package_target;
use super::v2_page::{PageHandle, PageRead, V2Page};

const HEADING: &str = "Opening link";

/// The relay while the link runs: there is no shape to predict, so a spinner.
fn running() -> AnyView {
    view! { <Spinner variant=SpinnerVariant::Region aria_label=HEADING /> }.into_any()
}

/// What `/remote-package` renders when *New design preview* is on.
#[component]
pub fn RemotePackageV2() -> impl IntoView {
    view! { <RelayScreen read=run_link /> }
}

/// What `/remote-package` draws while it reads which page it is, for a v2
/// reader: the relay's own first paint, in its transit frame.
#[component]
pub fn RemotePackageV2Skeleton() -> impl IntoView {
    view! {
        <PageLayout heading=HEADING banner=().into_any() actions=transit_appbar_actions() replace=true>
            {running()}
        </PageLayout>
    }
}

/// Run the link, and answer the package page to go to.
fn run_link(uri: String) -> std::pin::Pin<Box<dyn Future<Output = Result<String, String>>>> {
    Box::pin(async move {
        commands::handle_remote_package(uri)
            .await
            .map(|result| package_target(&result))
    })
}

fn replacing() -> NavigateOptions {
    NavigateOptions {
        replace: true,
        ..NavigateOptions::default()
    }
}

#[component]
pub(crate) fn RelayScreen(read: PageRead<String>) -> impl IntoView {
    let query = use_query_map();
    let key = Memo::new(move |_| query.with(|q| q.get("uri").unwrap_or_default()));
    let page = PageHandle::new();
    let PageHandle {
        outcome,
        reload,
        in_flight,
        ..
    } = page;
    let answer = page.read(key, read);

    // On to the package, or to sign-in or setup where v1 goes for the same
    // failure.
    let navigate = use_navigate();
    Effect::new({
        let navigate = navigate.clone();
        move |_| match answer.get() {
            Some(Ok(path)) => navigate(&path, replacing()),
            Some(Err(e)) => {
                if let Some(target) = redirect(&e) {
                    navigate(&target, replacing());
                }
            }
            None => {}
        }
    });

    // Each run installs or opens something, so a second may not start while
    // one is out. Marked here, before the read's own effect runs, so a quick
    // second press already finds it out.
    let on_reload = Callback::new(move |()| {
        if in_flight.get_untracked() {
            return;
        }
        in_flight.set(true);
        reload.notify();
    });
    let on_dot_quilt = open_dot_quilt(outcome, key);
    let on_home = Callback::new(move |()| navigate("/", replacing()));

    view! {
        <V2Page
            heading=HEADING
            page=page
            answer=answer
            showing=key
            transit=true
            skeleton=running
            bands=|_| ().into_any()
            body=|_| running()
            failure=move || {
                let Some(Err(e)) = answer.get_untracked() else {
                    return running();
                };
                if redirect(&e).is_some() {
                    return running();
                }
                view! {
                    <ErrorSlate
                        title="Error"
                        message=readable(&e)
                        on_reload=on_reload
                        reloading=in_flight
                        on_dot_quilt=on_dot_quilt
                        on_home=on_home
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
        static RUNS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    }

    /// Fails; every run after the first takes a while.
    fn refused(_: String) -> std::pin::Pin<Box<dyn Future<Output = Result<String, String>>>> {
        let n = RUNS.with(|r| {
            r.set(r.get() + 1);
            r.get()
        });
        Box::pin(async move {
            if n > 1 {
                sleep_ms(100).await;
            }
            Err("no such package".to_string())
        })
    }

    async fn relay() -> web_sys::Element {
        unmount_earlier();
        RUNS.with(|r| r.set(0));
        let el = mount(|| {
            view! {
                <Router>
                    <Suspense fallback=|| ()>
                        <RelayScreen read=refused />
                    </Suspense>
                </Router>
            }
        });
        sleep_ms(50).await;
        el
    }

    /// The rules v1's relay keeps: one bar, Refresh disabled, the failure
    /// drawn inside the same frame.
    #[wasm_bindgen_test]
    async fn the_relay_draws_one_frame_with_refresh_disabled() {
        let el = relay().await;
        assert!(
            el.text_content()
                .unwrap_or_default()
                .contains("no such package"),
            "the failure path must have drawn; markup was {}",
            el.inner_html()
        );
        assert_eq!(
            el.query_selector_all("header").unwrap().length(),
            1,
            "one bar, and no second shell; markup was {}",
            el.inner_html()
        );
        let refresh = button_saying(&el, "Refresh");
        assert!(
            refresh.disabled(),
            "a Refresh here reloads and re-runs the deep link; markup was {}",
            el.inner_html()
        );
        let logo: web_sys::HtmlAnchorElement = el
            .query_selector("header a")
            .unwrap()
            .expect("the logo")
            .unchecked_into();
        assert_eq!(
            js_sys::Reflect::get(&logo, &"replace".into())
                .unwrap()
                .as_bool(),
            Some(true),
            "the logo leaves without a history entry"
        );
    }

    /// Mounting runs the link once; Reload page runs it again in place, and
    /// never twice at once.
    #[wasm_bindgen_test]
    async fn the_link_runs_once_and_again_on_reload() {
        let el = relay().await;
        assert_eq!(RUNS.with(std::cell::Cell::get), 1);
        let reload = button_saying(&el, "Reload page");
        reload.click();
        reload.click();
        sleep_ms(30).await;
        assert_eq!(
            RUNS.with(std::cell::Cell::get),
            2,
            "one run while one is out"
        );
        assert!(
            button_saying(&el, "Reload page").disabled(),
            "and Reload page says so"
        );
        button_saying(&el, "Reload page").click();
        sleep_ms(30).await;
        assert_eq!(RUNS.with(std::cell::Cell::get), 2);
        sleep_ms(150).await;
        assert!(!button_saying(&el, "Reload page").disabled(), "free again");
        button_saying(&el, "Open .quilt directory");
        button_saying(&el, "Go home");
    }
}
