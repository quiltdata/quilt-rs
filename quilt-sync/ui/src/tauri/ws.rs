//! The WebSocket transport, for the UI served to a plain browser.
//!
//! One shared socket at `/ws?t=<token>` on the page's own host. Requests are
//! `{"id", "cmd", "args"}`; replies are `{"id", "ok"}` or `{"id", "err"}`; events
//! are `{"event", "payload"}` and the server broadcasts all of them, so listeners
//! are only a registry here.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{MessageEvent, WebSocket};

/// The query parameter only reaches the first page: router navigation drops it.
const TOKEN_KEY: &str = "quiltsync-token";

const MAX_BACKOFF_MS: i32 = 10_000;

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
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
}

/// The session token from `?t=` on this page, or the one saved from an earlier
/// page. `None` means the page was not opened by the `QuiltSync` server.
pub(super) fn token() -> Option<String> {
    let window = web_sys::window()?;
    let storage = window.session_storage().ok().flatten();
    let from_url = window
        .location()
        .search()
        .ok()
        .and_then(|search| web_sys::UrlSearchParams::new_with_str(&search).ok())
        .and_then(|params| params.get("t"))
        .filter(|t| !t.is_empty());
    if let Some(token) = from_url {
        if let Some(storage) = &storage {
            let _ = storage.set_item(TOKEN_KEY, &token);
        }
        return Some(token);
    }
    storage?
        .get_item(TOKEN_KEY)
        .ok()
        .flatten()
        .filter(|t| !t.is_empty())
}

fn js_err(err: &JsValue) -> String {
    err.as_string().unwrap_or_else(|| format!("{err:?}"))
}

/// Open the shared socket unless one exists (open or still connecting).
fn connect() -> Result<(), String> {
    if STATE.with_borrow(|s| s.socket.is_some()) {
        return Ok(());
    }
    let token = token().ok_or("No QuiltSync session token")?;
    let window = web_sys::window().ok_or("No window")?;
    let location = window.location();
    let scheme = if location.protocol().map_err(|e| js_err(&e))? == "https:" {
        "wss"
    } else {
        "ws"
    };
    let host = location.host().map_err(|e| js_err(&e))?;
    let url = format!("{scheme}://{host}/ws?t={}", urlencoding::encode(&token));
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
    let hydrates: Vec<Hydrate> = STATE.with_borrow_mut(|s| {
        s.open = true;
        s.retries = 0;
        let queue = std::mem::take(&mut s.queue);
        if let Some(socket) = &s.socket {
            for message in queue {
                let _ = socket.send_with_str(&message);
            }
        }
        s.listeners
            .values()
            .filter_map(|l| l.hydrate.clone())
            .collect()
    });
    // Every open, the first and each reconnect: events sent while the socket
    // was down were missed, so the listening pages read their state again.
    for hydrate in hydrates {
        hydrate();
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
    let (pending, delay) = STATE.with_borrow_mut(|s| {
        s.open = false;
        s.socket = None;
        s.queue.clear();
        let delay = 500_i32
            .saturating_mul(1 << s.retries.min(5))
            .min(MAX_BACKOFF_MS);
        s.retries += 1;
        (std::mem::take(&mut s.pending), delay)
    });
    let closed = JsValue::from_str("Lost the connection to QuiltSync");
    for (_, (_, reject)) in pending {
        let _ = reject.call1(&JsValue::NULL, &closed);
    }
    let retry = Closure::once_into_js(|| {
        let _ = connect();
    });
    if let Some(window) = web_sys::window() {
        let _ = window
            .set_timeout_with_callback_and_timeout_and_arguments_0(retry.unchecked_ref(), delay);
    }
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
