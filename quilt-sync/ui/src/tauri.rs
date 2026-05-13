use std::cell::RefCell;
use std::rc::Rc;

use send_wrapper::SendWrapper;
use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke)]
    fn tauri_invoke_raw(cmd: &str, args: JsValue) -> js_sys::Promise;

    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], js_name = listen)]
    fn tauri_listen_raw(event: &str, handler: &js_sys::Function) -> js_sys::Promise;
}

/// Call a Tauri command with typed arguments and return type.
///
/// Tauri's `invoke` expects camelCase argument keys (the proc macro
/// converts snake_case Rust parameter names to camelCase on the JS
/// side). Use `#[serde(rename_all = "camelCase")]` on arg structs.
pub async fn invoke<A: Serialize, R: DeserializeOwned>(cmd: &str, args: &A) -> Result<R, String> {
    let args_js = serde_wasm_bindgen::to_value(args).map_err(|e| e.to_string())?;
    let promise = tauri_invoke_raw(cmd, args_js);
    let result = JsFuture::from(promise)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))?;
    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

/// Call a Tauri command with no arguments.
pub async fn invoke_unit<R: DeserializeOwned>(cmd: &str) -> Result<R, String> {
    let args = js_sys::Object::new();
    let promise = tauri_invoke_raw(cmd, args.into());
    let result = JsFuture::from(promise)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))?;
    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
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
}

/// Subscribe to a Tauri event. The returned `EventListener` calls the
/// JS-side `unlisten` on drop — pair with Leptos `on_cleanup`. The
/// underlying WASM closure is intentionally leaked (`Closure::forget`)
/// so Tauri can never dispatch into freed memory in the window between
/// `Drop` and the JS side actually detaching.
pub fn listen<T: DeserializeOwned + 'static>(
    event: &str,
    mut callback: impl FnMut(T) + 'static,
) -> EventListener {
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
    let promise = tauri_listen_raw(&event_name, closure.as_ref().unchecked_ref());
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
                    }
                    ListenerState::Resolved(_) | ListenerState::Done => {}
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
            ListenerState::Cancelled | ListenerState::Done => {}
        }
    }
}
