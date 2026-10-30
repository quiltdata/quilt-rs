//! The browser's side: the token the page was opened with, and the WebSocket
//! to the engine that served the page.
//!
//! The page is opened with `?t=<token>`. It keeps the token in its origin's
//! `localStorage`, which, unlike a cookie, no other port can read, and drops it
//! from the address. The socket is `/ws?t=<token>` on the page's own host. The
//! desktop window's link also says `window`, which the page keeps for its
//! reloads.

use std::future::Future;
use std::pin::Pin;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::MessageEvent;
use web_sys::WebSocket;

use super::link;
use super::link::Connector;
use super::link::Socket;

/// Where the page keeps the engine's token.
const TOKEN_KEY: &str = "quiltsync-token";

/// Where the page notes the engine version it reloaded for, per tab.
const RELOADED_KEY: &str = "quiltsync-reloaded-for";

/// Where the page keeps its id across its reloads.
const PAGE_KEY: &str = "quiltsync-page";

/// The desktop window's page id; a browser tab makes up its own.
pub(super) const WINDOW: &str = "window";

/// The token this page has.
pub(super) enum Token {
    Kept(String),
    /// The address had one, but the page has no storage to keep it in.
    Unkept,
}

/// The link from the address, kept for later loads and dropped from the
/// address; or, with none there, what was kept before.
pub(super) fn read_link() -> Option<Token> {
    let window = web_sys::window()?;
    let storage = window.local_storage().ok().flatten();
    let session = window.session_storage().ok().flatten();
    let link = window
        .location()
        .href()
        .ok()
        .and_then(|href| split_link(&href));
    if let Some(link) = link {
        let kept = storage
            .as_ref()
            .is_some_and(|storage| storage.set_item(TOKEN_KEY, &link.token).is_ok());
        if link.window
            && let Some(session) = &session
        {
            let _ = session.set_item(PAGE_KEY, WINDOW);
        }
        if let Ok(history) = window.history() {
            let _ = history.replace_state_with_url(&JsValue::NULL, "", Some(&link.rest));
        }
        return Some(if kept {
            Token::Kept(link.token)
        } else {
            Token::Unkept
        });
    }
    storage?.get_item(TOKEN_KEY).ok().flatten().map(Token::Kept)
}

thread_local! {
    static PAGE_ID: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// This page's id: `window` for the desktop window, or one this tab makes up.
/// Kept in the tab's session storage, so a reload during a login still finds
/// where the login sent it.
pub(super) fn page_id() -> String {
    PAGE_ID.with_borrow_mut(|id| {
        id.get_or_insert_with(|| {
            let session = web_sys::window().and_then(|w| w.session_storage().ok().flatten());
            if let Some(kept) = session
                .as_ref()
                .and_then(|s| s.get_item(PAGE_KEY).ok().flatten())
            {
                return kept;
            }
            // Random enough to tell a user's tabs apart; nothing rests on it.
            let random = js_sys::Math::random().to_string();
            let made = format!("tab-{}", random.trim_start_matches("0."));
            if let Some(session) = session {
                let _ = session.set_item(PAGE_KEY, &made);
            }
            made
        })
        .clone()
    })
}

#[derive(Debug, PartialEq, Eq)]
struct Link {
    token: String,
    window: bool,
    /// The address without them.
    rest: String,
}

/// The `t` of `href` and whether it opens the window, and `href` without them.
fn split_link(href: &str) -> Option<Link> {
    let url = web_sys::Url::new(href).ok()?;
    let params = url.search_params();
    let token = params.get("t").filter(|t| !t.is_empty())?;
    let window = params.has("window");
    params.delete("t");
    params.delete("window");
    Some(Link {
        token,
        window,
        rest: url.href(),
    })
}

/// The engine that served this page, over a WebSocket.
pub(super) struct Browser {
    pub(super) token: String,
}

struct BrowserSocket(WebSocket);

impl Socket for BrowserSocket {
    fn send(&self, text: &str) -> Result<(), String> {
        self.0.send_with_str(text).map_err(|e| js_err(&e))
    }
}

fn js_err(err: &JsValue) -> String {
    err.as_string().unwrap_or_else(|| format!("{err:?}"))
}

impl Connector for Browser {
    fn open(&self, id: u64) -> Result<Box<dyn Socket>, String> {
        let host = web_sys::window()
            .ok_or("No window")?
            .location()
            .host()
            .map_err(|e| js_err(&e))?;
        let url = format!(
            "ws://{host}/ws?t={}",
            js_sys::encode_uri_component(&self.token)
        );
        let socket = WebSocket::new(&url).map_err(|e| js_err(&e))?;

        // Leaked on purpose, one set per socket: a socket may still fire after
        // the page has let go of it.
        let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
            if let Some(text) = event.data().as_string() {
                link::received(id, &text);
            }
        });
        socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        on_message.forget();
        let on_close = Closure::<dyn FnMut()>::new(move || link::closed(id));
        socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        on_close.forget();

        Ok(Box::new(BrowserSocket(socket)))
    }

    /// A browser hides the status of a refused upgrade, so ask over plain
    /// HTTP. An engine that doesn't answer at all is away, not refusing.
    fn refused(&self) -> Pin<Box<dyn Future<Output = bool>>> {
        let url = format!("/ws?t={}", js_sys::encode_uri_component(&self.token));
        Box::pin(async move {
            let Some(window) = web_sys::window() else {
                return false;
            };
            let Ok(response) = JsFuture::from(window.fetch_with_str(&url)).await else {
                return false;
            };
            response
                .dyn_into::<web_sys::Response>()
                .is_ok_and(|r| r.status() == 401)
        })
    }

    fn after(&self, ms: i32, then: Box<dyn FnOnce()>) {
        let then = Closure::once_into_js(then);
        if let Some(window) = web_sys::window() {
            let _ = window
                .set_timeout_with_callback_and_timeout_and_arguments_0(then.unchecked_ref(), ms);
        }
    }

    fn reload(&self, version: &str) -> bool {
        let Some(window) = web_sys::window() else {
            return false;
        };
        let session = window.session_storage().ok().flatten();
        if session
            .as_ref()
            .and_then(|s| s.get_item(RELOADED_KEY).ok().flatten())
            .as_deref()
            == Some(version)
        {
            return false;
        }
        // Without storage to note it in, reloading could loop; carry on.
        if session.is_none_or(|s| s.set_item(RELOADED_KEY, version).is_err()) {
            return false;
        }
        window.location().reload().is_ok()
    }
}

#[cfg(test)]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::{Link, split_link};

    fn link(token: &str, window: bool, rest: &str) -> Link {
        Link {
            token: token.to_owned(),
            window,
            rest: rest.to_owned(),
        }
    }

    #[wasm_bindgen_test]
    fn the_link_leaves_the_address_and_the_rest_stays() {
        assert_eq!(
            split_link("http://127.0.0.1:7878/commit?namespace=a%2Fb&t=abc#top"),
            Some(link(
                "abc",
                false,
                "http://127.0.0.1:7878/commit?namespace=a%2Fb#top"
            ))
        );
        assert_eq!(
            split_link("http://127.0.0.1:7878/?t=abc"),
            Some(link("abc", false, "http://127.0.0.1:7878/"))
        );
        assert_eq!(
            split_link("http://127.0.0.1:7878/?t=abc&window"),
            Some(link("abc", true, "http://127.0.0.1:7878/"))
        );
        assert_eq!(split_link("http://127.0.0.1:7878/?namespace=a"), None);
        assert_eq!(split_link("http://127.0.0.1:7878/?t="), None);
    }
}
