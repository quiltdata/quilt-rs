//! The WebSocket transport, for a page the engine served to a browser.
//!
//! One shared socket at `/ws` on the page's own host, authorised by the cookie
//! the engine set when the page was opened with `?t=<token>`. The engine
//! speaks first, `{"hello": {"version", …}}`; then requests `{"id", "cmd",
//! "args"}` get `{"id", "ok"}` or `{"id", "err"}`, and every event arrives as
//! `{"event", "payload"}`.
//!
//! Nothing is sent before the hello: an engine with another version than the
//! one that served this page means the page is stale, and it reloads instead.
//! A dropped socket reconnects with back-off. When the engine refuses the
//! token (it was reset), the page stops trying and says so once.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::MessageEvent;
use web_sys::WebSocket;

const FIRST_RETRY_MS: i32 = 500;
const LAST_RETRY_MS: i32 = 5_000;

/// What every request answers once the engine refused the token.
const REFUSED: &str = "QuiltSync was restarted with a new link. Open the link it printed.";

pub(super) type Callback = Rc<RefCell<dyn FnMut(&serde_json::Value)>>;
type OnReady = Box<dyn FnOnce()>;

struct Listener {
    event: String,
    callback: Callback,
}

#[derive(Default)]
struct State {
    socket: Option<WebSocket>,
    /// The engine said hello, and its version matches.
    ready: bool,
    next_id: u64,
    /// `resolve` and `reject` of each request's promise.
    pending: HashMap<u64, (js_sys::Function, js_sys::Function)>,
    /// Requests made before the hello.
    queue: Vec<String>,
    listeners: BTreeMap<u64, Listener>,
    next_listener: u64,
    /// Run once at the next hello, for listeners registered before it.
    on_ready: Vec<(u64, OnReady)>,
    retries: u32,
    /// A retry, or the check of a refused token, is under way: no new socket
    /// until it is done.
    waiting: bool,
    /// The version of the engine that served this page, from the first hello.
    version: Option<String>,
    refused: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
    static SERVED: bool = read_served();
}

/// Whether the engine served this page, so it talks over the socket. The
/// engine sets this readable marker beside the token's `HttpOnly` cookie; test
/// harnesses and the gallery are served over HTTP too, but without it.
pub(super) fn served() -> bool {
    SERVED.with(|served| *served)
}

fn read_served() -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| js_sys::Reflect::get(&d, &JsValue::from_str("cookie")).ok())
        .and_then(|c| c.as_string())
        .is_some_and(|c| c.split(';').any(|pair| pair.trim() == "quiltsync-web=1"))
}

fn js_err(err: &JsValue) -> String {
    err.as_string().unwrap_or_else(|| format!("{err:?}"))
}

/// Open the shared socket unless one exists.
fn connect() -> Result<(), String> {
    let (busy, refused) = STATE.with_borrow(|s| (s.socket.is_some() || s.waiting, s.refused));
    if refused {
        return Err(REFUSED.to_owned());
    }
    if busy {
        return Ok(());
    }
    let host = web_sys::window()
        .ok_or("No window")?
        .location()
        .host()
        .map_err(|e| js_err(&e))?;
    let socket = WebSocket::new(&format!("ws://{host}/ws")).map_err(|e| js_err(&e))?;

    // Leaked on purpose, one set per socket: a socket may still fire after
    // this module has let go of it.
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(on_message);
    socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    on_message.forget();
    let on_close = Closure::<dyn FnMut()>::new(on_close);
    socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    on_close.forget();

    STATE.with_borrow_mut(|s| s.socket = Some(socket));
    Ok(())
}

#[derive(serde::Deserialize)]
struct Hello {
    version: String,
}

fn on_hello(hello: &Hello) {
    let stale = STATE.with_borrow_mut(|s| {
        let first = s.version.get_or_insert_with(|| hello.version.clone());
        *first != hello.version
    });
    if stale {
        // The engine was upgraded while this page stayed open.
        if let Some(window) = web_sys::window() {
            let _ = window.location().reload();
        }
        return;
    }
    let on_ready = STATE.with_borrow_mut(|s| {
        s.ready = true;
        s.retries = 0;
        let queue = std::mem::take(&mut s.queue);
        if let Some(socket) = &s.socket {
            for message in queue {
                let _ = socket.send_with_str(&message);
            }
        }
        std::mem::take(&mut s.on_ready)
    });
    for (_, on_ready) in on_ready {
        on_ready();
    }
}

#[allow(clippy::needless_pass_by_value, reason = "the closure's signature")]
fn on_message(event: MessageEvent) {
    let Some(text) = event.data().as_string() else {
        return;
    };
    let Ok(serde_json::Value::Object(mut message)) = serde_json::from_str(&text) else {
        web_sys::console::error_1(&format!("ws: unreadable message: {text}").into());
        return;
    };
    if let Some(hello) = message.remove("hello") {
        match serde_json::from_value(hello) {
            Ok(hello) => on_hello(&hello),
            Err(err) => web_sys::console::error_1(&format!("ws: unreadable hello: {err}").into()),
        }
        return;
    }
    if let Some(serde_json::Value::String(name)) = message.remove("event") {
        let payload = message.remove("payload").unwrap_or(serde_json::Value::Null);
        let callbacks: Vec<Callback> = STATE.with_borrow(|s| {
            s.listeners
                .values()
                .filter(|l| l.event == name)
                .map(|l| Rc::clone(&l.callback))
                .collect()
        });
        for callback in callbacks {
            if let Ok(mut callback) = callback.try_borrow_mut() {
                callback(&payload);
            }
        }
        return;
    }
    let Some(id) = message.get("id").and_then(serde_json::Value::as_u64) else {
        return;
    };
    let Some((resolve, reject)) = STATE.with_borrow_mut(|s| s.pending.remove(&id)) else {
        return;
    };
    if let Some(ok) = message.get("ok") {
        let _ = resolve.call1(&JsValue::NULL, &JsValue::from_str(&ok.to_string()));
    } else {
        let err = match message.remove("err") {
            Some(serde_json::Value::String(err)) => err,
            Some(err) => err.to_string(),
            None => "Malformed reply: neither ok nor err".to_owned(),
        };
        let _ = reject.call1(&JsValue::NULL, &JsValue::from_str(&err));
    }
}

fn on_close() {
    let (pending, delay, was_ready) = STATE.with_borrow_mut(|s| {
        let was_ready = std::mem::take(&mut s.ready);
        s.socket = None;
        s.waiting = true;
        let delay = FIRST_RETRY_MS
            .saturating_mul(1 << s.retries.min(4))
            .min(LAST_RETRY_MS);
        s.retries += 1;
        (std::mem::take(&mut s.pending), delay, was_ready)
    });
    let lost = JsValue::from_str("Lost the connection to QuiltSync");
    for (_, (_, reject)) in pending {
        let _ = reject.call1(&JsValue::NULL, &lost);
    }
    if was_ready {
        retry_after(delay);
        return;
    }
    // The socket never got going: the engine is away, or it refused the
    // token. A browser hides the status of a refused upgrade, so ask over
    // plain HTTP.
    wasm_bindgen_futures::spawn_local(async move {
        if token_refused().await {
            refuse();
        } else {
            retry_after(delay);
        }
    });
}

fn retry_after(delay: i32) {
    let retry = Closure::once_into_js(|| {
        STATE.with_borrow_mut(|s| s.waiting = false);
        let _ = connect();
    });
    if let Some(window) = web_sys::window() {
        let _ = window
            .set_timeout_with_callback_and_timeout_and_arguments_0(retry.unchecked_ref(), delay);
    }
}

/// Whether the engine answers 401 to this page's cookie. An engine that
/// doesn't answer at all is away, not refusing.
async fn token_refused() -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let Ok(response) = JsFuture::from(window.fetch_with_str("/ws")).await else {
        return false;
    };
    response
        .dyn_into::<web_sys::Response>()
        .is_ok_and(|r| r.status() == 401)
}

/// Stop reconnecting, fail what waits, and say so once.
fn refuse() {
    let pending = STATE.with_borrow_mut(|s| {
        s.refused = true;
        s.waiting = false;
        s.queue.clear();
        s.on_ready.clear();
        std::mem::take(&mut s.pending)
    });
    let refused = JsValue::from_str(REFUSED);
    for (_, (_, reject)) in pending {
        let _ = reject.call1(&JsValue::NULL, &refused);
    }
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let (Ok(banner), Some(body)) = (document.create_element("div"), document.body()) else {
        return;
    };
    let _ = banner.set_attribute("role", "alert");
    let _ = banner.set_attribute(
        "style",
        "position:fixed;top:0;left:0;right:0;z-index:2147483647;padding:12px 16px;\
         background:#b3261e;color:#fff;font:600 14px/1.4 system-ui,sans-serif;text-align:center",
    );
    banner.set_text_content(Some(REFUSED));
    let _ = body.append_child(&banner);
}

/// Send one command and resolve with its `ok` value as JSON text.
pub(super) async fn request(cmd: &str, args: serde_json::Value) -> Result<String, String> {
    connect()?;
    let mut slot = None;
    let promise = js_sys::Promise::new(&mut |resolve, reject| slot = Some((resolve, reject)));
    let (resolve, reject) = slot.ok_or("Promise executor did not run")?;
    STATE.with_borrow_mut(|s| {
        s.next_id += 1;
        let id = s.next_id;
        let message = serde_json::json!({ "id": id, "cmd": cmd, "args": args }).to_string();
        match (&s.socket, s.ready) {
            (Some(socket), true) => {
                if let Err(err) = socket.send_with_str(&message) {
                    let _ = reject.call1(&JsValue::NULL, &err);
                    return;
                }
            }
            _ => s.queue.push(message),
        }
        s.pending.insert(id, (resolve, reject));
    });
    let value = JsFuture::from(promise).await.map_err(|e| js_err(&e))?;
    value
        .as_string()
        .ok_or_else(|| "Reply was not text".to_owned())
}

/// Send if the engine is ready; otherwise drop the message.
pub(super) fn send_and_forget(cmd: &str, args: &serde_json::Value) {
    let _ = STATE.try_with(|state| {
        let Ok(mut s) = state.try_borrow_mut() else {
            return;
        };
        if !s.ready {
            return;
        }
        s.next_id += 1;
        let message = serde_json::json!({ "id": s.next_id, "cmd": cmd, "args": args }).to_string();
        if let Some(socket) = &s.socket {
            let _ = socket.send_with_str(&message);
        }
    });
}

/// Register a callback for `event` and return its handle for [`unlisten`].
///
/// `on_ready` runs once events flow: at the next hello, or soon if the engine
/// is ready already, and not if the handle was dropped first.
pub(super) fn listen(event: &str, callback: Callback, on_ready: OnReady) -> Result<u64, String> {
    connect()?;
    let (id, ready) = STATE.with_borrow_mut(|s| {
        s.next_listener += 1;
        let id = s.next_listener;
        s.listeners.insert(
            id,
            Listener {
                event: event.to_owned(),
                callback,
            },
        );
        (id, s.ready)
    });
    if ready {
        // Later, as with Tauri.
        wasm_bindgen_futures::spawn_local(async move {
            if STATE.with_borrow(|s| s.listeners.contains_key(&id)) {
                on_ready();
            }
        });
    } else {
        STATE.with_borrow_mut(|s| s.on_ready.push((id, on_ready)));
    }
    Ok(id)
}

pub(super) fn unlisten(id: u64) {
    let _ = STATE.try_with(|state| {
        if let Ok(mut s) = state.try_borrow_mut() {
            s.listeners.remove(&id);
            s.on_ready.retain(|(listener, _)| *listener != id);
        }
    });
}
