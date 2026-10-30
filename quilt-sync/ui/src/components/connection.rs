//! What the page shows of its connection to the engine, and what the engine
//! asks of every page.

use std::cell::Cell;
use std::rc::Rc;

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use crate::commands;
use crate::engine;
use crate::engine::Connection;
use crate::kit::{Banner, BannerVariant, Blankslate};

/// A bar while the page is reconnecting, and one that stays once the engine
/// refused it.
///
/// Over the top of the page rather than above it: every page fills the window
/// exactly, so a bar that pushed it down would cut off its bottom.
#[component]
pub fn ConnectionBanner() -> impl IntoView {
    let connection = engine::connection();
    let bar = move || match connection.get() {
        Connection::Rejected(message) => {
            Some(view! { <Banner variant=BannerVariant::Critical>{message}</Banner> }.into_any())
        }
        Connection::Lost => Some(
            view! {
                <Banner variant=BannerVariant::Warning>
                    "Lost the connection to QuiltSync. Reconnecting…"
                </Banner>
            }
            .into_any(),
        ),
        Connection::Connecting | Connection::Open(_) => None,
    };
    view! { <div style="position:fixed;top:0;left:0;right:0;z-index:1000">{bar}</div> }
}

/// The page opened without its link: say how to get one, rather than fail on
/// every section.
#[component]
pub fn NoLink() -> impl IntoView {
    view! {
        <Blankslate
            heading="Open QuiltSync from its link"
            description="This page needs the link QuiltSync gives it. Open it from the tray menu: Open in browser."
        />
    }
}

/// Go where the engine sends this page, after a login or a deep link: asked at
/// each nudge, and each time the page connects, so a window that was still
/// loading goes too.
#[component]
pub fn FollowEngine() -> impl IntoView {
    let navigate = use_navigate();
    // One follow at a time, so an older answer can't land after a newer one; a
    // nudge during a follow makes it ask once more.
    let busy = Rc::new(Cell::new(false));
    let again = Rc::new(Cell::new(false));
    let go = move || {
        if busy.replace(true) {
            again.set(true);
            return;
        }
        let (busy, again, navigate) = (Rc::clone(&busy), Rc::clone(&again), navigate.clone());
        leptos::task::spawn_local(async move {
            loop {
                again.set(false);
                follow(&navigate).await;
                if !again.get() {
                    break;
                }
            }
            busy.set(false);
        });
    };
    let listener = engine::listen_then::<serde::de::IgnoredAny>(
        commands::NAVIGATE_EVENT,
        {
            let go = go.clone();
            move |_| go()
        },
        go,
    );
    on_cleanup(move || drop(listener));
}

/// Go where the engine sent this page, if anywhere, and say so.
async fn follow(navigate: &impl Fn(&str, NavigateOptions)) {
    let Ok(Some(sent)) = commands::pending_navigation().await else {
        return;
    };
    // Noted and gone in one step, with no wait between, so a reload can't come
    // after one and before the other. A reply to the done that's lost brings
    // it again, and the page skips it.
    if remember(&sent.id) {
        // The router ignores a move to where the page already is, so opening
        // the same link again reloads, which runs it again.
        if Some(&sent.page) == here().as_ref() {
            if let Some(window) = web_sys::window() {
                let _ = window.location().reload();
            }
        } else {
            navigate(&sent.page, NavigateOptions::default());
        }
    }
    let _ = commands::navigation_done(sent.id).await;
}

/// Where this tab keeps the last navigation it followed, across its reloads.
const FOLLOWED_KEY: &str = "quiltsync-followed";

/// Note navigation `id` as followed; false if it already was.
fn remember(id: &str) -> bool {
    let session = web_sys::window().and_then(|w| w.session_storage().ok().flatten());
    let Some(session) = session else {
        return true;
    };
    if session.get_item(FOLLOWED_KEY).ok().flatten().as_deref() == Some(id) {
        return false;
    }
    let _ = session.set_item(FOLLOWED_KEY, id);
    true
}

/// The page's address, without its host.
fn here() -> Option<String> {
    let location = web_sys::window()?.location();
    Some(format!(
        "{}{}",
        location.pathname().ok()?,
        location.search().ok()?
    ))
}

#[cfg(test)]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;
    use crate::engine::fake::{FakeEngine, ask};
    use crate::test_support::{mount, sleep_ms, unmount_earlier};

    fn alerts(root: &web_sys::Element) -> u32 {
        root.query_selector_all("[role=alert]").unwrap().length()
    }

    /// A refused token stops the retries, fails what waits, and says so in
    /// one bar.
    #[wasm_bindgen_test]
    async fn a_refused_token_stops_retrying_with_one_banner() {
        let engine = FakeEngine::install();
        engine.refuse_token();
        let root = mount(ConnectionBanner);
        let asked = ask("get_toasts");
        sleep_ms(0).await;
        engine.close();
        sleep_ms(0).await;

        assert!(
            asked.borrow().clone().is_some_and(|answer| answer.is_err()),
            "what waited failed"
        );
        assert_eq!(engine.sockets_opened(), 1, "no retry after a refusal");
        assert_eq!(alerts(&root), 1);
        assert!(
            engine::invoke_unit::<()>("get_toasts")
                .await
                .is_err_and(|err| err.contains("new link")),
            "a later request fails at once and says why"
        );
        sleep_ms(0).await;
        assert_eq!(engine.sockets_opened(), 1);
        assert_eq!(alerts(&root), 1);
    }

    /// The page asks where the engine sent it once it connects, says it went,
    /// and goes there; sent the same one again, as after a lost reply, it says
    /// so again but doesn't go twice.
    #[wasm_bindgen_test]
    async fn a_page_goes_where_the_engine_sent_it_once() {
        // Routers that earlier tests left would follow the address too.
        unmount_earlier();
        let window = web_sys::window().unwrap();
        let before = window.location().href().unwrap();
        // Not already at the page it's sent to, which would reload the runner.
        let _ = window.history().unwrap().replace_state_with_url(
            &wasm_bindgen::JsValue::NULL,
            "",
            Some("/"),
        );
        let engine = FakeEngine::install();
        mount(
            || view! { <leptos_router::components::Router><FollowEngine /></leptos_router::components::Router> },
        );
        sleep_ms(0).await;
        engine.hello();
        sleep_ms(0).await;

        let sent = serde_json::json!({"id": "n1", "page": "/settings"});
        let requests = engine.requests();
        assert_eq!(requests.len(), 1, "one ask, at the hello");
        assert_eq!(requests[0]["cmd"], "pending_navigation");
        assert_eq!(requests[0]["args"]["page"], engine::page_id());
        engine.reply(&requests[0]["id"], &sent);
        sleep_ms(0).await;
        let requests = engine.requests();
        assert_eq!(requests[1]["cmd"], "navigation_done");
        assert_eq!(requests[1]["args"]["id"], "n1");
        engine.reply(&requests[1]["id"], &serde_json::Value::Null);
        sleep_ms(0).await;
        assert_eq!(window.location().pathname().unwrap(), "/settings");

        let _ = window.history().unwrap().replace_state_with_url(
            &wasm_bindgen::JsValue::NULL,
            "",
            Some("/"),
        );

        // A nudge asks again, and the engine still has it.
        engine.event(crate::commands::NAVIGATE_EVENT, &serde_json::Value::Null);
        sleep_ms(0).await;
        let requests = engine.requests();
        assert_eq!(requests[2]["cmd"], "pending_navigation");
        engine.reply(&requests[2]["id"], &sent);
        sleep_ms(0).await;
        let requests = engine.requests();
        assert_eq!(requests[3]["cmd"], "navigation_done");
        engine.reply(&requests[3]["id"], &serde_json::Value::Null);
        sleep_ms(0).await;
        assert_eq!(
            window.location().pathname().unwrap(),
            "/",
            "it doesn't go twice"
        );
        assert_eq!(engine.requests().len(), 4);

        let _ = window.history().unwrap().replace_state_with_url(
            &wasm_bindgen::JsValue::NULL,
            "",
            Some(&before),
        );
        // Else it asks again at every later test's connect.
        unmount_earlier();
    }

    /// Nudges while the page is still asking make it ask once more after, not
    /// once each at the same time, so an older answer can't land last.
    #[wasm_bindgen_test]
    async fn nudges_during_an_ask_make_one_more_ask() {
        unmount_earlier();
        let engine = FakeEngine::install();
        mount(
            || view! { <leptos_router::components::Router><FollowEngine /></leptos_router::components::Router> },
        );
        sleep_ms(0).await;
        engine.hello();
        sleep_ms(0).await;
        assert_eq!(engine.requests().len(), 1);

        engine.event(crate::commands::NAVIGATE_EVENT, &serde_json::Value::Null);
        engine.event(crate::commands::NAVIGATE_EVENT, &serde_json::Value::Null);
        sleep_ms(0).await;
        assert_eq!(engine.requests().len(), 1, "no second ask while one waits");

        engine.reply(&engine.requests()[0]["id"], &serde_json::Value::Null);
        sleep_ms(0).await;
        let requests = engine.requests();
        assert_eq!(requests.len(), 2, "one more ask for both nudges");
        assert_eq!(requests[1]["cmd"], "pending_navigation");
        engine.reply(&requests[1]["id"], &serde_json::Value::Null);
        sleep_ms(0).await;
        assert_eq!(engine.requests().len(), 2);

        unmount_earlier();
    }

    /// While the page reconnects it says so, and stops saying so once back.
    #[wasm_bindgen_test]
    async fn a_lost_connection_shows_until_the_engine_is_back() {
        let engine = FakeEngine::install();
        let root = mount(ConnectionBanner);
        let _listener = engine::listen::<serde_json::Value>("toast", |_| ());
        engine.hello();
        engine.close();
        sleep_ms(0).await;
        assert!(
            root.text_content()
                .unwrap_or_default()
                .contains("Reconnecting")
        );

        engine.hello();
        sleep_ms(0).await;
        assert_eq!(root.text_content().unwrap_or_default().trim(), "");
    }
}
