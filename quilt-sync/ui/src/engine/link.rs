//! The connection to the engine, apart from the socket that carries it, so a
//! test can drive it through a fake one.
//!
//! One shared socket. The engine speaks first, `{"hello": {"version", …}}`, and
//! nothing is sent before it. Then requests `{"id", "cmd", "args"}` get
//! `{"id", "ok"}` or `{"id", "err"}`, and every event arrives as
//! `{"event", "payload"}`.
//!
//! A socket that closes fails every request still waiting; none is sent again,
//! since it may have run. The page reconnects with back-off. A socket that
//! closes before its hello may have been refused, and a browser hides why, so
//! the connector asks: a refused token stops the retries for good.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use leptos::prelude::*;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;

use super::Connection;

const FIRST_RETRY_MS: i32 = 500;
const LAST_RETRY_MS: i32 = 5_000;

/// What every request answers once the engine refused the token.
pub(super) const REFUSED: &str =
    "QuiltSync has a new link. Open it from the tray menu: Open in browser.";

/// What a request answers when the page has no way to the engine at all: a
/// test harness, the gallery, or a page opened without its link.
pub(super) const NOT_LINKED: &str = "Not connected to QuiltSync";

/// What every request still waiting answers when its socket closes.
pub(super) const LOST: &str = "Lost the connection to QuiltSync";

/// The app version this page was built with.
pub(super) const VERSION: &str = env!("QUILTSYNC_VERSION");

/// One open socket to the engine.
pub(super) trait Socket {
    fn send(&self, text: &str) -> Result<(), String>;
}

/// How the page reaches the engine: the browser's WebSocket, or a test's fake.
pub(super) trait Connector {
    /// Open socket `id`. It reports through [`received`] and [`closed`] under
    /// that id, never before this returns.
    fn open(&self, id: u64) -> Result<Box<dyn Socket>, String>;

    /// Whether the engine refuses this page's token.
    fn refused(&self) -> Pin<Box<dyn Future<Output = bool>>>;

    /// Run `then` after `ms` milliseconds.
    fn after(&self, ms: i32, then: Box<dyn FnOnce()>);

    /// Load the page again, for an engine at `version`. False when the page
    /// already did once for that version, so a mismatch can't loop.
    fn reload(&self, version: &str) -> bool;
}

pub(super) type Callback = Rc<RefCell<dyn FnMut(&serde_json::Value)>>;
pub(super) type OnLive = Rc<RefCell<dyn FnMut()>>;

struct Listener {
    event: String,
    callback: Callback,
    /// Run each time events start to flow: at a hello.
    on_live: Option<OnLive>,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// No socket, and nothing under way.
    #[default]
    Idle,
    /// A socket is open or opening; the engine hasn't said hello on it yet.
    Opening,
    /// The engine said hello: requests go straight out.
    Ready,
    /// A retry, or the check of a refused token, is under way.
    Waiting,
    /// No more sockets, and every request answers this.
    Refused(&'static str),
}

struct State {
    connector: Option<Rc<dyn Connector>>,
    socket: Option<(u64, Box<dyn Socket>)>,
    sockets: u64,
    phase: Phase,
    hellos: u64,
    /// A socket closed since the last hello: requests failed and events were
    /// lost in the gap.
    gap: bool,
    next_id: u64,
    /// `resolve` and `reject` of each request's promise.
    pending: HashMap<u64, (js_sys::Function, js_sys::Function)>,
    /// Requests made before the hello.
    queue: Vec<String>,
    listeners: BTreeMap<u64, Listener>,
    next_listener: u64,
    retries: u32,
    connection: ArcRwSignal<Connection>,
    /// How many hellos followed a gap.
    reconnects: ArcRwSignal<u64>,
}

impl State {
    fn new(connector: Option<Rc<dyn Connector>>) -> Self {
        Self {
            connector,
            socket: None,
            sockets: 0,
            phase: Phase::Idle,
            hellos: 0,
            gap: false,
            next_id: 0,
            pending: HashMap::new(),
            queue: Vec::new(),
            listeners: BTreeMap::new(),
            next_listener: 0,
            retries: 0,
            connection: ArcRwSignal::new(Connection::Connecting),
            reconnects: ArcRwSignal::new(0),
        }
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::new(None));
}

/// Start over with `connector`: no socket, no listeners, nothing waiting.
pub(super) fn install(connector: Option<Rc<dyn Connector>>) {
    STATE.set(State::new(connector));
}

pub(super) fn connection() -> ArcRwSignal<Connection> {
    STATE.with_borrow(|s| s.connection.clone())
}

pub(super) fn reconnects() -> ArcRwSignal<u64> {
    STATE.with_borrow(|s| s.reconnects.clone())
}

fn js_err(err: &JsValue) -> String {
    err.as_string().unwrap_or_else(|| format!("{err:?}"))
}

/// Open the shared socket unless one exists or is on its way.
fn connect() -> Result<(), String> {
    let next = STATE.with_borrow_mut(|s| {
        match s.phase {
            Phase::Idle => {}
            Phase::Refused(message) => return Err(message.to_owned()),
            Phase::Opening | Phase::Ready | Phase::Waiting => return Ok(None),
        }
        let Some(connector) = s.connector.clone() else {
            return Err(NOT_LINKED.to_owned());
        };
        s.sockets += 1;
        Ok(Some((connector, s.sockets)))
    })?;
    let Some((connector, id)) = next else {
        return Ok(());
    };
    let socket = connector.open(id)?;
    STATE.with_borrow_mut(|s| {
        s.socket = Some((id, socket));
        s.phase = Phase::Opening;
    });
    Ok(())
}

/// A message on socket `id`.
pub(super) fn received(id: u64, text: &str) {
    if STATE.with_borrow(|s| s.socket.as_ref().map(|(current, _)| *current)) != Some(id) {
        return;
    }
    let Ok(serde_json::Value::Object(mut message)) = serde_json::from_str(text) else {
        web_sys::console::error_1(&format!("engine: unreadable message: {text}").into());
        return;
    };
    if let Some(hello) = message.remove("hello") {
        let version = hello.get("version").and_then(serde_json::Value::as_str);
        on_hello(version.unwrap_or_default());
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

fn on_hello(version: &str) {
    if version != VERSION {
        let connector = STATE.with_borrow(|s| s.connector.clone());
        if connector.is_some_and(|c| c.reload(version)) {
            return;
        }
        web_sys::console::warn_1(
            &format!("engine: QuiltSync {version} serves this page, built for {VERSION}").into(),
        );
    }
    let (lives, connection, reconnects, hellos, reconnected) = STATE.with_borrow_mut(|s| {
        s.phase = Phase::Ready;
        s.retries = 0;
        s.hellos += 1;
        let queue = std::mem::take(&mut s.queue);
        if let Some((_, socket)) = &s.socket {
            for message in queue {
                let _ = socket.send(&message);
            }
        }
        let lives: Vec<OnLive> = s
            .listeners
            .values()
            .filter_map(|l| l.on_live.clone())
            .collect();
        (
            lives,
            s.connection.clone(),
            s.reconnects.clone(),
            s.hellos,
            std::mem::take(&mut s.gap),
        )
    });
    connection.set(Connection::Open(hellos));
    if reconnected {
        reconnects.update(|n| *n += 1);
    }
    for live in lives {
        if let Ok(mut live) = live.try_borrow_mut() {
            live();
        }
    }
}

/// Socket `id` closed.
pub(super) fn closed(id: u64) {
    let Some((pending, delay, was_ready, connector, connection)) = STATE.with_borrow_mut(|s| {
        if s.socket.as_ref().map(|(current, _)| *current) != Some(id) {
            return None;
        }
        let was_ready = s.phase == Phase::Ready;
        s.socket = None;
        s.phase = Phase::Waiting;
        s.gap = true;
        // Their promises are rejected below; sent later, they would run
        // after the page was told they failed.
        s.queue.clear();
        let delay = FIRST_RETRY_MS
            .saturating_mul(1 << s.retries.min(4))
            .min(LAST_RETRY_MS);
        s.retries += 1;
        Some((
            std::mem::take(&mut s.pending),
            delay,
            was_ready,
            s.connector.clone()?,
            s.connection.clone(),
        ))
    }) else {
        return;
    };
    connection.set(Connection::Lost);
    let lost = JsValue::from_str(LOST);
    for (_, (_, reject)) in pending {
        let _ = reject.call1(&JsValue::NULL, &lost);
    }
    if was_ready {
        retry_after(&connector, delay);
        return;
    }
    // The socket never got going: the engine is away, or it refused the token.
    wasm_bindgen_futures::spawn_local(async move {
        if connector.refused().await {
            refuse(REFUSED);
        } else {
            retry_after(&connector, delay);
        }
    });
}

fn retry_after(connector: &Rc<dyn Connector>, delay: i32) {
    connector.after(
        delay,
        Box::new(|| {
            let waiting = STATE.with_borrow_mut(|s| {
                let waiting = s.phase == Phase::Waiting;
                if waiting {
                    s.phase = Phase::Idle;
                }
                waiting
            });
            if waiting {
                let _ = connect();
            }
        }),
    );
}

/// Stop for good, and fail what waits with `message`.
pub(super) fn refuse(message: &'static str) {
    let (pending, connection) = STATE.with_borrow_mut(|s| {
        s.phase = Phase::Refused(message);
        s.socket = None;
        s.queue.clear();
        (std::mem::take(&mut s.pending), s.connection.clone())
    });
    connection.set(Connection::Rejected(message));
    let refused = JsValue::from_str(message);
    for (_, (_, reject)) in pending {
        let _ = reject.call1(&JsValue::NULL, &refused);
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
        match (&s.socket, s.phase) {
            (Some((_, socket)), Phase::Ready) => {
                if let Err(err) = socket.send(&message) {
                    let _ = reject.call1(&JsValue::NULL, &JsValue::from_str(&err));
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
        if s.phase != Phase::Ready {
            return;
        }
        s.next_id += 1;
        let message = serde_json::json!({ "id": s.next_id, "cmd": cmd, "args": args }).to_string();
        if let Some((_, socket)) = &s.socket {
            let _ = socket.send(&message);
        }
    });
}

/// Register a callback for `event` and return its handle for [`unlisten`].
///
/// `on_live` runs each time events start to flow: at every hello, and soon if
/// the engine is ready already. Never once the handle is dropped.
pub(super) fn listen(
    event: &str,
    callback: Callback,
    on_live: Option<OnLive>,
) -> Result<u64, String> {
    connect()?;
    let (id, ready) = STATE.with_borrow_mut(|s| {
        s.next_listener += 1;
        let id = s.next_listener;
        s.listeners.insert(
            id,
            Listener {
                event: event.to_owned(),
                callback,
                on_live: on_live.clone(),
            },
        );
        (id, s.phase == Phase::Ready)
    });
    if let (true, Some(on_live)) = (ready, on_live) {
        // Later, as for a hello: the caller hasn't kept its handle yet.
        wasm_bindgen_futures::spawn_local(async move {
            if STATE.with_borrow(|s| s.listeners.contains_key(&id))
                && let Ok(mut live) = on_live.try_borrow_mut()
            {
                live();
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
