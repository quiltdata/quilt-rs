//! The page's one way to the engine: its commands and its events.
//!
//! Every window and tab is a client of the engine that served it, over one
//! WebSocket ([`browser`]). The connection itself ([`link`]) knows nothing of
//! the socket, so another transport, or a test's fake, plugs in beside it.
//!
//! A page with no token has no way to the engine: a test harness, the gallery,
//! or a page opened without its link. There every call fails quietly.

use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use leptos::prelude::*;
use serde::Serialize;
use serde::de::DeserializeOwned;

mod browser;
#[cfg(test)]
pub(crate) mod fake;
mod link;

/// Where the page's connection to the engine stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connection {
    /// The first socket is on its way.
    Connecting,
    /// The engine said hello, for the `n`th time.
    Open(u64),
    /// A socket closed; the page is reconnecting.
    Lost,
    /// The page stopped trying, and says why.
    Rejected(&'static str),
}

/// What a page that can't keep its token says.
const NO_STORAGE: &str =
    "QuiltSync in a browser needs site storage. Turn it on for this site and open the link again.";

thread_local! {
    static LINKED: Cell<bool> = const { Cell::new(false) };
}

/// Read the token the page was opened with and choose the way to the engine.
/// Once, before the router reads the address: a link's token leaves it here.
pub fn start() {
    let token = browser::read_link();
    LINKED.set(token.is_some());
    match token {
        Some(browser::Token::Kept(token)) => {
            link::install(Some(Rc::new(browser::Browser { token })));
        }
        Some(browser::Token::Unkept) => {
            link::install(None);
            link::refuse(NO_STORAGE);
        }
        None => link::install(None),
    }
}

/// This page's id, for the engine to send it somewhere: the desktop window's,
/// or one this tab made up and keeps across its reloads.
pub fn page_id() -> String {
    browser::page_id()
}

/// Whether this page is the desktop window rather than a browser tab.
pub fn is_window() -> bool {
    page_id() == browser::WINDOW
}

/// Whether the page was given a link to the engine, now or before.
pub fn linked() -> bool {
    LINKED.get()
}

/// The page's connection, as it changes.
pub fn connection() -> Signal<Connection> {
    link::connection().read_only().into()
}

/// Notify `reload` each time the page reconnects, so a page reads again what
/// it may have missed while the socket was down. Not at the first hello: a
/// read made before it waits for it.
pub fn reread_on_reconnect(reload: Trigger) {
    let reconnects: Signal<u64> = link::reconnects().read_only().into();
    Effect::new(move |seen: Option<u64>| {
        let now = reconnects.get();
        if seen.is_some_and(|seen| seen != now) {
            reload.notify();
        }
        now
    });
}

/// Call a command and ignore the outcome — no `await`, no future, no unwinding.
///
/// For the one caller that cannot await: a **panic hook**. Awaiting means driving a
/// Rust future, and a panicking WASM module traps as soon as the hook returns, so a
/// queued future would never run. Sending is synchronous; only the reply is lost,
/// which there is nothing left to do with.
///
/// Every failure is swallowed deliberately. This runs while the module is already
/// dying, and the panic it is reporting is more informative than anything that could
/// go wrong here.
pub fn invoke_and_forget<A: Serialize>(cmd: &str, args: &A) {
    if let Ok(args) = serde_json::to_value(args) {
        link::send_and_forget(cmd, &args);
    }
}

/// Call an engine command with typed arguments and return type.
///
/// The engine reads arguments by their camelCase names. Use
/// `#[serde(rename_all = "camelCase")]` on arg structs.
pub async fn invoke<A: Serialize, R: DeserializeOwned>(cmd: &str, args: &A) -> Result<R, String> {
    let args = serde_json::to_value(args).map_err(|e| e.to_string())?;
    let reply = link::request(cmd, args).await?;
    serde_json::from_str(&reply).map_err(|e| e.to_string())
}

/// Call an engine command with no arguments.
pub async fn invoke_unit<R: DeserializeOwned>(cmd: &str) -> Result<R, String> {
    #[derive(Serialize)]
    struct NoArgs {}
    invoke(cmd, &NoArgs {}).await
}

/// Subscribe to an engine event. The returned `EventListener` unsubscribes on
/// drop — pair with Leptos `on_cleanup`.
pub fn listen<T: DeserializeOwned + 'static>(
    event: &str,
    callback: impl FnMut(T) + 'static,
) -> EventListener {
    subscribe(event, callback, None)
}

/// [`listen`], then `on_live` each time events start to flow: once the
/// listener is live, and again after every reconnect. Never once dropped, nor
/// without a way to the engine.
///
/// For a caller that hydrates a state: a read taken before events flow can
/// miss a change made meanwhile, and a reconnect means events were lost.
pub fn listen_then<T: DeserializeOwned + 'static>(
    event: &str,
    callback: impl FnMut(T) + 'static,
    on_live: impl FnMut() + 'static,
) -> EventListener {
    subscribe(event, callback, Some(Rc::new(RefCell::new(on_live))))
}

fn subscribe<T: DeserializeOwned + 'static>(
    event: &str,
    mut callback: impl FnMut(T) + 'static,
    on_live: Option<link::OnLive>,
) -> EventListener {
    let event_name = event.to_string();
    let callback: link::Callback =
        Rc::new(RefCell::new(
            move |payload: &serde_json::Value| match serde_json::from_value::<T>(payload.clone()) {
                Ok(payload) => callback(payload),
                Err(err) => web_sys::console::error_1(
                    &format!("listen: failed to deserialize {event_name}: {err}").into(),
                ),
            },
        ));
    match link::listen(event, callback, on_live) {
        Ok(id) => EventListener { id: Some(id) },
        Err(err) => {
            web_sys::console::error_1(&format!("listen: no connection for {event}: {err}").into());
            EventListener { id: None }
        }
    }
}

pub struct EventListener {
    id: Option<u64>,
}

impl Drop for EventListener {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            link::unlisten(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    /// Firing a command where the page has no way to the engine must be silent.
    ///
    /// This test environment *is* that case, which is why it can assert the
    /// property at all. It matters because the one caller is a panic hook: a
    /// throw here would replace the panic being reported with a failure to
    /// report it.
    #[wasm_bindgen_test]
    fn invoking_without_an_engine_does_not_throw() {
        super::invoke_and_forget("report_ui_panic", &serde_json::json!({"message": "x"}));
    }

    /// Without an engine nothing is registered, so `on_live` must never run — a
    /// caller would read a state it then gets no events for — and the handle
    /// must still be safe to drop.
    #[wasm_bindgen_test]
    async fn listening_without_an_engine_never_goes_live() {
        let live = std::rc::Rc::new(std::cell::Cell::new(false));
        let flag = std::rc::Rc::clone(&live);
        let listener = super::listen_then::<serde_json::Value>(
            "package-status-changed",
            |_| (),
            move || flag.set(true),
        );
        crate::test_support::sleep_ms(0).await;
        drop(listener);
        assert!(!live.get());
    }

    use std::cell::Cell;
    use std::rc::Rc;

    use leptos::prelude::*;
    use serde_json::json;

    use super::Connection;
    use super::fake::{FakeEngine, ask};
    use crate::test_support::{mount, sleep_ms};

    /// A request made before the hello waits for it, then goes out once, and
    /// its reply lands.
    #[wasm_bindgen_test]
    async fn a_request_waits_for_the_hello_and_gets_its_reply() {
        let engine = FakeEngine::install();
        let asked = ask("get_toasts");
        sleep_ms(0).await;
        assert!(engine.requests().is_empty(), "nothing before the hello");

        engine.hello();
        let requests = engine.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["cmd"], "get_toasts");
        engine.reply(&requests[0]["id"], &json!([1]));
        sleep_ms(0).await;
        assert_eq!(asked.borrow().clone(), Some(Ok(json!([1]))));
    }

    /// A socket that closes before its hello fails what waits; the next
    /// socket's hello sends none of it again.
    #[wasm_bindgen_test]
    async fn a_close_before_the_hello_fails_what_waits_without_a_replay() {
        let engine = FakeEngine::install();
        let asked = ask("get_toasts");
        sleep_ms(0).await;
        engine.close();
        sleep_ms(0).await;
        assert_eq!(
            asked.borrow().clone(),
            Some(Err(super::link::LOST.to_owned()))
        );

        assert_eq!(engine.sockets_opened(), 2, "it reconnects");
        engine.hello();
        assert!(
            engine.all_requests().is_empty(),
            "the failed request is not sent"
        );
    }

    /// A socket that closes after its hello fails what waits; the next one
    /// sends none of it again.
    #[wasm_bindgen_test]
    async fn a_close_after_the_hello_fails_what_waits_without_a_replay() {
        let engine = FakeEngine::install();
        let asked = ask("get_toasts");
        sleep_ms(0).await;
        engine.hello();
        assert_eq!(engine.requests().len(), 1);
        engine.close();
        sleep_ms(0).await;
        assert_eq!(
            asked.borrow().clone(),
            Some(Err(super::link::LOST.to_owned()))
        );

        engine.hello();
        assert!(
            engine.requests().is_empty(),
            "the new socket sends nothing again"
        );
    }

    /// Each reconnect counts a new hello, and a page that reads through
    /// `reread_on_reconnect` reads again; the first hello doesn't make it.
    #[wasm_bindgen_test]
    async fn a_reconnect_opens_a_new_epoch_and_pages_read_again() {
        let engine = FakeEngine::install();
        let reads = Rc::new(Cell::new(0));
        let counted = Rc::clone(&reads);
        mount(move || {
            let reload = Trigger::new();
            super::reread_on_reconnect(reload);
            Effect::new(move |_| {
                reload.track();
                counted.set(counted.get() + 1);
            });
        });
        let connection = super::connection();
        let _listener = super::listen::<serde_json::Value>("toast", |_| ());
        sleep_ms(0).await;
        let first = reads.get();

        engine.hello();
        sleep_ms(0).await;
        assert_eq!(connection.get_untracked(), Connection::Open(1));
        assert_eq!(reads.get(), first, "the first hello reads nothing again");

        engine.close();
        sleep_ms(0).await;
        assert_eq!(connection.get_untracked(), Connection::Lost);
        engine.hello();
        sleep_ms(0).await;
        assert_eq!(connection.get_untracked(), Connection::Open(2));
        assert_eq!(reads.get(), first + 1, "a reconnect reads again");
    }

    /// A listener goes live at its hello and again at every reconnect, when
    /// events it missed may have been lost.
    #[wasm_bindgen_test]
    async fn a_listener_goes_live_at_every_hello() {
        let engine = FakeEngine::install();
        let lives = Rc::new(Cell::new(0));
        let counted = Rc::clone(&lives);
        let heard = Rc::new(Cell::new(0));
        let got = Rc::clone(&heard);
        let _listener = super::listen_then::<u64>(
            "toast",
            move |n| got.set(n),
            move || counted.set(counted.get() + 1),
        );
        engine.hello();
        engine.event("toast", &json!(7));
        assert_eq!((lives.get(), heard.get()), (1, 7));

        engine.close();
        sleep_ms(0).await;
        engine.hello();
        assert_eq!(lives.get(), 2);
    }

    /// An engine of another version than the page's makes it load again, once
    /// for that version, and nothing is sent to it meanwhile.
    #[wasm_bindgen_test]
    async fn another_engine_version_reloads_the_page_once() {
        let engine = FakeEngine::install();
        let _asked = ask("get_toasts");
        sleep_ms(0).await;
        engine.hello_from("0.0.0-other");
        assert_eq!(engine.reloads(), vec!["0.0.0-other".to_owned()]);
        assert!(engine.requests().is_empty(), "nothing is sent to it");

        // Loaded again and still another version: carry on rather than loop.
        engine.hello_from("0.0.0-other");
        assert_eq!(engine.reloads().len(), 2);
        assert_eq!(engine.requests().len(), 1);
    }

    /// Retries back off from half a second to five, and start over once a
    /// hello lands.
    #[wasm_bindgen_test]
    async fn retries_back_off_to_five_seconds() {
        let engine = FakeEngine::install();
        let _listener = super::listen::<serde_json::Value>("toast", |_| ());
        for _ in 0..6 {
            engine.close();
            sleep_ms(0).await;
        }
        assert_eq!(engine.delays(), vec![500, 1000, 2000, 4000, 5000, 5000]);
        engine.hello();
        engine.close();
        sleep_ms(0).await;
        assert_eq!(engine.delays().last(), Some(&500));
    }

    #[wasm_bindgen_test]
    async fn a_command_without_an_engine_fails() {
        let answer = super::invoke_unit::<()>("get_toasts").await;
        assert_eq!(answer, Err(super::link::NOT_LINKED.to_owned()));
    }
}
