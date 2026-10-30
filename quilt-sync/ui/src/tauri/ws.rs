//! The WebSocket transport, for the UI served to a plain browser.
//!
//! One shared socket at `/ws` on the page's own host, authorised by the
//! cookie the server set when the page was opened with `?t=<token>`. Requests are
//! `{"id", "cmd", "args"}`; replies are `{"id", "ok"}` or `{"id", "err"}`; events
//! are `{"event", "payload"}` and the server broadcasts all of them, so listeners
//! are only a registry here.
//!
//! After the socket drops, it reconnects with the same cookie; the token
//! survives a restart of the engine, and the listening pages read again. When the engine
//! refuses the token (it was reset), it stops trying and says so in a banner.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use leptos::prelude::{ArcRwSignal, Get, Update};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{MessageEvent, WebSocket};

const MAX_BACKOFF_MS: i32 = 10_000;

/// Why every request fails once the engine refused the token.
const REFUSED: &str = "QuiltSync was restarted with a new link. Open the link it printed.";

pub(super) type Callback = Rc<RefCell<dyn FnMut(&serde_json::Value)>>;
type Hydrate = Rc<dyn Fn()>;

struct Listener {
    event: String,
    callback: Callback,
    hydrate: Option<Hydrate>,
}

#[derive(Default)]
struct State {
    socket: Option<WebSocket>,
    open: bool,
    next_id: u64,
    /// `resolve` and `reject` of each request's promise.
    pending: HashMap<u64, (js_sys::Function, js_sys::Function)>,
    /// Messages sent before the socket opened.
    queue: Vec<String>,
    listeners: BTreeMap<u64, Listener>,
    next_listener: u64,
    retries: u32,
    /// Whether a socket has opened before, so the next open is a reconnect.
    opened_once: bool,
    /// The engine refused the token: no more sockets.
    refused: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
    /// Bumped on every reconnect, for reads that re-run then.
    static RECONNECTS: ArcRwSignal<u32> = ArcRwSignal::new(0);
}

/// How many times the socket reconnected. A tracked read: a resource or an
/// effect that calls it runs again after each reconnect.
pub(super) fn reconnects() -> u32 {
    RECONNECTS.with(Get::get)
}

/// Whether the `QuiltSync` server served this page, in a browser or in the
/// desktop window. The server sets this readable marker beside the token's
/// `HttpOnly` cookie; test harnesses and the gallery are served over HTTP too,
/// but without it.
pub(super) fn served() -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| js_sys::Reflect::get(&d, &JsValue::from_str("cookie")).ok())
        .and_then(|c| c.as_string())
        .is_some_and(|c| c.split(';').any(|pair| pair.trim() == "quiltsync-web=1"))
}

fn js_err(err: &JsValue) -> String {
    err.as_string().unwrap_or_else(|| format!("{err:?}"))
}

/// Open the shared socket unless one exists (open or still connecting).
fn connect() -> Result<(), String> {
    let (exists, refused) = STATE.with_borrow(|s| (s.socket.is_some(), s.refused));
    if refused {
        return Err(REFUSED.to_string());
    }
    if exists {
        return Ok(());
    }
    let window = web_sys::window().ok_or("No window")?;
    let location = window.location();
    let scheme = if location.protocol().map_err(|e| js_err(&e))? == "https:" {
        "wss"
    } else {
        "ws"
    };
    let host = location.host().map_err(|e| js_err(&e))?;
    let url = format!("{scheme}://{host}/ws");
    let socket = WebSocket::new(&url).map_err(|e| js_err(&e))?;

    // Leaked on purpose, one set per connection: a socket may fire after this
    // module stops referencing it.
    let on_open = Closure::<dyn FnMut()>::new(on_open);
    socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));
    on_open.forget();
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(on_message);
    socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    on_message.forget();
    let on_close = Closure::<dyn FnMut()>::new(on_close);
    socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));
    on_close.forget();

    STATE.with_borrow_mut(|s| s.socket = Some(socket));
    Ok(())
}

fn on_open() {
    let (reconnect, hydrates): (bool, Vec<Hydrate>) = STATE.with_borrow_mut(|s| {
        s.open = true;
        s.retries = 0;
        let reconnect = std::mem::replace(&mut s.opened_once, true);
        let queue = std::mem::take(&mut s.queue);
        if let Some(socket) = &s.socket {
            for message in queue {
                let _ = socket.send_with_str(&message);
            }
        }
        let hydrates = s
            .listeners
            .values()
            .filter_map(|l| l.hydrate.clone())
            .collect();
        (reconnect, hydrates)
    });
    // Every open, the first and each reconnect: events sent while the socket
    // was down were missed, so the listening pages read their state again.
    for hydrate in hydrates {
        hydrate();
    }
    if reconnect {
        RECONNECTS.with(|n| n.update(|n| *n += 1));
    }
}

#[allow(clippy::needless_pass_by_value)] // The closure signature.
fn on_message(event: MessageEvent) {
    let Some(text) = event.data().as_string() else {
        return;
    };
    let Ok(serde_json::Value::Object(mut message)) = serde_json::from_str(&text) else {
        web_sys::console::error_1(&format!("ws: unreadable message: {text}").into());
        return;
    };
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
            None => "Malformed reply: neither ok nor err".to_string(),
        };
        let _ = reject.call1(&JsValue::NULL, &JsValue::from_str(&err));
    }
}

fn on_close() {
    let (pending, delay, was_open) = STATE.with_borrow_mut(|s| {
        let was_open = s.open;
        s.open = false;
        s.socket = None;
        s.queue.clear();
        let delay = 500_i32
            .saturating_mul(1 << s.retries.min(5))
            .min(MAX_BACKOFF_MS);
        s.retries += 1;
        (std::mem::take(&mut s.pending), delay, was_open)
    });
    let closed = JsValue::from_str("Lost the connection to QuiltSync");
    for (_, (_, reject)) in pending {
        let _ = reject.call1(&JsValue::NULL, &closed);
    }
    if was_open {
        retry_after(delay);
        return;
    }
    // A socket that never opened: the engine is down, or it refused the token.
    // The browser hides the upgrade's status, so ask over plain HTTP.
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
        let _ = connect();
    });
    if let Some(window) = web_sys::window() {
        let _ = window
            .set_timeout_with_callback_and_timeout_and_arguments_0(retry.unchecked_ref(), delay);
    }
}

/// Whether the engine answers 401 to this page's token. An engine that does
/// not answer at all is down, not refusing.
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

/// Stop reconnecting, fail what is queued, and show one banner.
fn refuse() {
    let pending = STATE.with_borrow_mut(|s| {
        s.refused = true;
        s.queue.clear();
        std::mem::take(&mut s.pending)
    });
    let refused = JsValue::from_str(REFUSED);
    for (_, (_, reject)) in pending {
        let _ = reject.call1(&JsValue::NULL, &refused);
    }
    web_sys::console::error_1(&format!("ws: the engine refused the token. {REFUSED}").into());
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
        match (&s.socket, s.open) {
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
        .ok_or_else(|| "Reply was not text".to_string())
}

/// Send if the socket is open; otherwise drop the message.
pub(super) fn send_and_forget(cmd: &str, args: &serde_json::Value) {
    let _ = STATE.try_with(|state| {
        let Ok(mut s) = state.try_borrow_mut() else {
            return;
        };
        if !s.open {
            return;
        }
        s.next_id += 1;
        let id = s.next_id;
        let message = serde_json::json!({ "id": id, "cmd": cmd, "args": args }).to_string();
        if let Some(socket) = &s.socket {
            let _ = socket.send_with_str(&message);
        }
    });
}

/// Register a callback for `event` and return its handle for [`unlisten`].
///
/// `hydrate` runs once the socket is open — now, if it already is — and again
/// after every reconnect.
pub(super) fn listen(
    event: &str,
    callback: Callback,
    hydrate: Option<Hydrate>,
) -> Result<u64, String> {
    connect()?;
    let (id, open) = STATE.with_borrow_mut(|s| {
        s.next_listener += 1;
        let id = s.next_listener;
        s.listeners.insert(
            id,
            Listener {
                event: event.to_string(),
                callback,
                hydrate: hydrate.clone(),
            },
        );
        (id, s.open)
    });
    if let (true, Some(hydrate)) = (open, hydrate) {
        // Later, as with Tauri, and only if the handle was not dropped first.
        wasm_bindgen_futures::spawn_local(async move {
            if STATE.with_borrow(|s| s.listeners.contains_key(&id)) {
                hydrate();
            }
        });
    }
    Ok(id)
}

pub(super) fn unlisten(id: u64) {
    let _ = STATE.try_with(|state| {
        if let Ok(mut s) = state.try_borrow_mut() {
            s.listeners.remove(&id);
        }
    });
}

/// `navigator.clipboard.writeText(text)`.
pub(super) async fn write_clipboard(text: &str) -> Result<(), String> {
    let window = web_sys::window().ok_or("No window")?;
    let navigator = js_sys::Reflect::get(&window, &"navigator".into()).map_err(|e| js_err(&e))?;
    let clipboard =
        js_sys::Reflect::get(&navigator, &"clipboard".into()).map_err(|e| js_err(&e))?;
    let write = js_sys::Reflect::get(&clipboard, &"writeText".into())
        .map_err(|e| js_err(&e))?
        .dyn_into::<js_sys::Function>()
        .map_err(|_| "The clipboard is not available on this page".to_string())?;
    let promise = write
        .call1(&clipboard, &JsValue::from_str(text))
        .map_err(|e| js_err(&e))?
        .dyn_into::<js_sys::Promise>()
        .map_err(|_| "writeText did not return a promise".to_string())?;
    JsFuture::from(promise).await.map_err(|e| js_err(&e))?;
    Ok(())
}
