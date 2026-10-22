use std::cell::RefCell;
use std::rc::Rc;

use send_wrapper::SendWrapper;
use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

mod ws;

/// Whether this page talks to the backend over the WebSocket rather than Tauri.
///
/// Tauri wins whenever its bridge exists. Without it, the WebSocket is used only
/// when the page carries a session token (`?t=` on the first URL, saved in
/// `sessionStorage`), which only the `QuiltSync` server hands out. Anything else —
/// a test harness, the gallery, a page opened from disk — keeps the old
/// no-bridge behaviour: every call fails quietly.
pub fn is_web_mode() -> bool {
    !has_tauri_bridge() && ws::token().is_some()
}

fn has_tauri_bridge() -> bool {
    web_sys::window()
        .and_then(|w| js_sys::Reflect::has(&w, &"__TAURI__".into()).ok())
        .unwrap_or(false)
}

/// Copy `text` with the browser's clipboard API. For web mode, where there is no
/// native clipboard to call.
pub async fn copy_in_browser(text: &str) -> Result<(), String> {
    ws::write_clipboard(text).await
}

/// The path, query and fragment of `url`, whatever its scheme and host.
///
/// The backend builds navigation targets from the Tauri window's URL, which is
/// `tauri://localhost/…` or `http://tauri.localhost/…`; the browser needs them
/// relative to its own origin.
pub fn strip_origin(url: &str) -> String {
    let Some((_, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    match rest.find(['/', '?', '#']) {
        Some(i) if rest[i..].starts_with('/') => rest[i..].to_string(),
        Some(i) => format!("/{}", &rest[i..]),
        None => "/".to_string(),
    }
}

#[wasm_bindgen]
extern "C" {
    /// `listen`, bound so a throw comes back as a value. Same hazard as
    /// [`tauri_invoke_catching`] and a wider blast radius: a component that
    /// registers a listener on mount cannot be mounted in a test at all while a
    /// missing `window.__TAURI__` traps the module, which is why the pages that
    /// listen have historically had to keep their markup in a separate component
    /// to be testable.
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], js_name = listen, catch)]
    fn tauri_listen_raw(
        event: &str,
        handler: &js_sys::Function,
    ) -> Result<js_sys::Promise, JsValue>;

    /// `invoke`, bound so a throw comes back as a value rather than an uncaught JS
    /// exception. Without `catch`, a missing `window.__TAURI__` (a browser test
    /// harness, or a page loaded outside Tauri) traps the wasm module instead of
    /// returning `Err` — fatal in [`invoke_and_forget`]'s panic-hook context, and,
    /// discovered while building v2's heavy phase, capable of corrupting the
    /// wasm-bindgen-futures microtask queue for the rest of a test binary when hit
    /// from an ordinary `spawn_local` call site.
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke, catch)]
    fn tauri_invoke_catching(cmd: &str, args: JsValue) -> Result<js_sys::Promise, JsValue>;
}

/// Call a command and ignore the outcome — no `await`, no future, no unwinding.
///
/// For the one caller that cannot await: a **panic hook**. Awaiting means driving a
/// Rust future, and a panicking WASM module traps as soon as the hook returns, so a
/// queued future would never run. Calling the JS function posts the IPC message
/// synchronously; dropping the promise loses only the acknowledgement, which there is
/// nothing left to do with.
///
/// Every failure is swallowed deliberately. This runs while the module is already
/// dying, and the panic it is reporting is more informative than anything that could
/// go wrong here.
pub fn invoke_and_forget<A: Serialize>(cmd: &str, args: &A) {
    if is_web_mode() {
        if let Ok(args) = serde_json::to_value(args) {
            ws::send_and_forget(cmd, &args);
        }
        return;
    }
    if let Ok(args_js) = serde_wasm_bindgen::to_value(args) {
        let _ = tauri_invoke_catching(cmd, args_js);
    }
}

/// Call a Tauri command with typed arguments and return type.
///
/// Tauri's `invoke` expects camelCase argument keys (the proc macro
/// converts `snake_case` Rust parameter names to `camelCase` on the JS
/// side). Use `#[serde(rename_all = "camelCase")]` on arg structs.
///
/// Uses the `catch`-bound `tauri_invoke_catching`: a missing bridge (a browser
/// test harness, or a page loaded outside Tauri) must come back as an `Err`
/// here, not an uncaught JS exception — see that binding's doc comment.
pub async fn invoke<A: Serialize, R: DeserializeOwned>(cmd: &str, args: &A) -> Result<R, String> {
    if is_web_mode() {
        let args = serde_json::to_value(args).map_err(|e| e.to_string())?;
        return invoke_ws(cmd, args).await;
    }
    let args_js = serde_wasm_bindgen::to_value(args).map_err(|e| e.to_string())?;
    let promise = tauri_invoke_catching(cmd, args_js)
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))?;
    let result = JsFuture::from(promise)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))?;
    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

/// Call a Tauri command with no arguments. See [`invoke`] for why this catches.
pub async fn invoke_unit<R: DeserializeOwned>(cmd: &str) -> Result<R, String> {
    if is_web_mode() {
        return invoke_ws(cmd, serde_json::json!({})).await;
    }
    let args = js_sys::Object::new();
    let promise = tauri_invoke_catching(cmd, args.into())
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))?;
    let result = JsFuture::from(promise)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))?;
    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

async fn invoke_ws<R: DeserializeOwned>(cmd: &str, args: serde_json::Value) -> Result<R, String> {
    let text = ws::request(cmd, args).await?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// `__TAURI__.event` delivers payloads as `{ event, payload, id, ... }`.
/// Only the `payload` slot carries our typed data.
#[derive(serde::Deserialize)]
struct TauriEventEnvelope<P> {
    payload: P,
}

/// Shared between `Drop` and the registration future — the rendezvous
/// point that resolves the race between component unmount and async
/// listener registration.
enum ListenerState {
    /// Registration Promise is still pending.
    Pending,
    /// Promise resolved; the function will be called by `Drop`.
    Resolved(js_sys::Function),
    /// `Drop` ran first; the future calls `unlisten` when it resolves.
    Cancelled,
    /// Terminal: `unlisten` has been (or will be) called.
    Done,
    /// Registered with the WebSocket transport under this handle.
    Ws(u64),
}

/// Subscribe to a Tauri event. The returned `EventListener` calls the
/// JS-side `unlisten` on drop — pair with Leptos `on_cleanup`. The
/// underlying WASM closure is intentionally leaked (`Closure::forget`)
/// so Tauri can never dispatch into freed memory in the window between
/// `Drop` and the JS side actually detaching.
///
/// `tauri_listen_raw` registers asynchronously (it returns a Promise);
/// the internal task below awaits it to capture the `unlisten` function
/// and reconciles with `Drop` through [`ListenerState`], so unmounting
/// before registration completes still detaches the listener.
pub fn listen<T: DeserializeOwned + 'static>(
    event: &str,
    callback: impl FnMut(T) + 'static,
) -> EventListener {
    listen_then(event, callback, || ())
}

/// [`listen`], then `on_registered` once the listener is live — not if it was
/// dropped first, nor without a bridge.
///
/// Over the WebSocket, `on_registered` also runs again after each reconnect,
/// since events sent while the socket was down were lost.
///
/// For a caller that hydrates a state: a read taken before the listener is live
/// can miss a change made between the read and the registration.
pub fn listen_then<T: DeserializeOwned + 'static>(
    event: &str,
    mut callback: impl FnMut(T) + 'static,
    on_registered: impl Fn() + 'static,
) -> EventListener {
    if is_web_mode() {
        let event_name = event.to_string();
        let callback: ws::Callback =
            Rc::new(RefCell::new(
                move |payload: &serde_json::Value| match serde_json::from_value::<T>(
                    payload.clone(),
                ) {
                    Ok(payload) => callback(payload),
                    Err(err) => web_sys::console::error_1(
                        &format!("listen: failed to deserialize {event_name}: {err}").into(),
                    ),
                },
            ));
        let state = match ws::listen(event, callback, Some(Rc::new(on_registered))) {
            Ok(id) => ListenerState::Ws(id),
            Err(err) => {
                web_sys::console::error_1(
                    &format!("listen: no connection for {event}: {err}").into(),
                );
                ListenerState::Done
            }
        };
        return EventListener {
            state: SendWrapper::new(Rc::new(RefCell::new(state))),
        };
    }
    let event_name = event.to_string();
    let event_name_for_closure = event_name.clone();
    let closure: Closure<dyn FnMut(JsValue)> = Closure::new(move |raw: JsValue| {
        match serde_wasm_bindgen::from_value::<TauriEventEnvelope<T>>(raw) {
            Ok(envelope) => callback(envelope.payload),
            Err(err) => web_sys::console::error_1(
                &format!("listen: failed to deserialize {event_name_for_closure}: {err}").into(),
            ),
        }
    });
    let promise = match tauri_listen_raw(&event_name, closure.as_ref().unchecked_ref()) {
        Ok(promise) => promise,
        // No bridge: a page loaded outside Tauri, or a test harness. Nothing was
        // registered, so the handle has nothing to detach.
        Err(err) => {
            web_sys::console::error_1(
                &format!("listen: no Tauri bridge for {event_name}: {err:?}").into(),
            );
            closure.forget();
            return EventListener {
                state: SendWrapper::new(Rc::new(RefCell::new(ListenerState::Done))),
            };
        }
    };
    closure.forget();

    // `SendWrapper` lets the !Send JS handle satisfy `on_cleanup`'s
    // `Send + Sync` bound; WASM is single-threaded so the wrapper
    // never panics in practice.
    let state: Rc<RefCell<ListenerState>> = Rc::new(RefCell::new(ListenerState::Pending));
    let state_for_task = Rc::clone(&state);
    let event_name_for_task = event_name.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let result = JsFuture::from(promise).await;
        match result {
            Ok(val) => {
                let func = val.dyn_into::<js_sys::Function>().ok();
                let mut s = state_for_task.borrow_mut();
                match std::mem::replace(&mut *s, ListenerState::Done) {
                    ListenerState::Cancelled => {
                        drop(s);
                        if let Some(f) = func {
                            let _ = f.call0(&JsValue::NULL);
                        }
                    }
                    ListenerState::Pending => {
                        if let Some(f) = func {
                            *s = ListenerState::Resolved(f);
                        }
                        drop(s);
                        on_registered();
                    }
                    ListenerState::Resolved(_) | ListenerState::Done | ListenerState::Ws(_) => {}
                }
            }
            Err(err) => web_sys::console::error_1(
                &format!("listen: failed to register {event_name_for_task}: {err:?}").into(),
            ),
        }
    });

    EventListener {
        state: SendWrapper::new(state),
    }
}

pub struct EventListener {
    state: SendWrapper<Rc<RefCell<ListenerState>>>,
}

impl Drop for EventListener {
    fn drop(&mut self) {
        let mut s = self.state.borrow_mut();
        match std::mem::replace(&mut *s, ListenerState::Done) {
            ListenerState::Resolved(f) => {
                drop(s);
                let _ = f.call0(&JsValue::NULL);
            }
            ListenerState::Pending => {
                // Promise hasn't resolved yet — let the future unlisten
                // when it does.
                *s = ListenerState::Cancelled;
            }
            ListenerState::Ws(id) => ws::unlisten(id),
            ListenerState::Cancelled | ListenerState::Done => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test;

    #[test]
    fn strip_origin_keeps_path_query_and_fragment() {
        use super::strip_origin;
        assert_eq!(
            strip_origin("tauri://localhost/installed-package?namespace=a/b#x"),
            "/installed-package?namespace=a/b#x"
        );
        assert_eq!(
            strip_origin("http://tauri.localhost/login?host=h&back=%2F"),
            "/login?host=h&back=%2F"
        );
        assert_eq!(strip_origin("http://tauri.localhost"), "/");
        assert_eq!(strip_origin("http://tauri.localhost?x=1"), "/?x=1");
        assert_eq!(
            strip_origin("/commit?namespace=a/b"),
            "/commit?namespace=a/b"
        );
    }

    /// Firing a command where no Tauri bridge exists must be silent.
    ///
    /// This test environment *is* that case — a browser with no `window.__TAURI__` —
    /// which is why it can assert the property at all. It matters because the one
    /// caller is a panic hook: a throw here would replace the panic being reported
    /// with a failure to report it.
    #[wasm_bindgen_test]
    fn invoking_without_a_tauri_bridge_does_not_throw() {
        super::invoke_and_forget("report_ui_panic", &serde_json::json!({"message": "x"}));
    }

    /// Registering a listener where no bridge exists must be silent for the same
    /// reason, and it is a wider one: a page that registers a listener cannot be
    /// mounted in a test at all while this throws, because the trap takes the
    /// module down rather than arriving as an error the caller can log.
    #[wasm_bindgen_test]
    fn listening_without_a_tauri_bridge_does_not_throw() {
        drop(super::listen::<serde_json::Value>(
            "package-status-changed",
            |_| (),
        ));
    }

    /// Without a bridge nothing is registered, so `on_registered` must never
    /// run — a caller would read a state it then gets no events for — and the
    /// handle must still be safe to drop.
    #[wasm_bindgen_test]
    async fn listening_without_a_tauri_bridge_never_reports_registered() {
        let registered = std::rc::Rc::new(std::cell::Cell::new(false));
        let flag = std::rc::Rc::clone(&registered);
        let listener = super::listen_then::<serde_json::Value>(
            "package-status-changed",
            |_| (),
            move || flag.set(true),
        );
        // A timer, not `leptos::task::tick`: the registration runs on
        // `wasm_bindgen_futures`, and `tick` needs a Leptos executor, which only
        // exists if an earlier test in the run happened to mount something.
        crate::test_support::sleep_ms(0).await;
        drop(listener);
        assert!(!registered.get());
    }
}
