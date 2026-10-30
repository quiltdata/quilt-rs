//! The browser's side: the token the page was opened with, and the WebSocket
//! to the engine that served the page.
//!
//! The page is opened with `?t=<token>`. It keeps the token in its origin's
//! `localStorage`, which, unlike a cookie, no other port can read, and drops it
//! from the address. The socket is `/ws?t=<token>` on the page's own host.

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

/// The token this page has.
pub(super) enum Token {
    Kept(String),
    /// The address had one, but the page has no storage to keep it in.
    Unkept,
}

/// The token from the address, kept for later loads and dropped from the
/// address; or, with none there, the one kept before.
pub(super) fn read_token() -> Option<Token> {
    let window = web_sys::window()?;
    let storage = window.local_storage().ok().flatten();
    let href = window.location().href().ok()?;
    if let Some((token, rest)) = split_token(&href) {
        let kept = storage
            .as_ref()
            .is_some_and(|storage| storage.set_item(TOKEN_KEY, &token).is_ok());
        if let Ok(history) = window.history() {
            let _ = history.replace_state_with_url(&JsValue::NULL, "", Some(&rest));
        }
        return Some(if kept {
            Token::Kept(token)
        } else {
            Token::Unkept
        });
    }
    storage?.get_item(TOKEN_KEY).ok().flatten().map(Token::Kept)
}

/// The `t` of `href`, and `href` without it.
fn split_token(href: &str) -> Option<(String, String)> {
    let url = web_sys::Url::new(href).ok()?;
    let params = url.search_params();
    let token = params.get("t").filter(|t| !t.is_empty())?;
    params.delete("t");
    Some((token, url.href()))
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

    use super::split_token;

    #[wasm_bindgen_test]
    fn the_token_leaves_the_address_and_the_rest_stays() {
        assert_eq!(
            split_token("http://127.0.0.1:7878/commit?namespace=a%2Fb&t=abc#top"),
            Some((
                "abc".to_owned(),
                "http://127.0.0.1:7878/commit?namespace=a%2Fb#top".to_owned()
            ))
        );
        assert_eq!(
            split_token("http://127.0.0.1:7878/?t=abc"),
            Some(("abc".to_owned(), "http://127.0.0.1:7878/".to_owned()))
        );
        assert_eq!(split_token("http://127.0.0.1:7878/?namespace=a"), None);
        assert_eq!(split_token("http://127.0.0.1:7878/?t="), None);
    }
}
