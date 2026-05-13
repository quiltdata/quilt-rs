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

/// Subscribe to a Tauri event from the WASM side.
///
/// `__TAURI__.event.listen(event, handler)` is an async IPC call — the JS
/// handler is wrapped, an ID is allocated, and the backend is asked to
/// route the named event to that ID. The subscription is not live until
/// the returned Promise resolves, so we await it inside a `spawn_local`
/// (callers are sync). The closure must outlive the registration *and*
/// every subsequent event delivery, so we `forget()` it — detaching
/// would mean awaiting the unlisten Promise, and the per-page leak from
/// one listener is small.
pub fn listen<T: DeserializeOwned + 'static>(event: &str, mut callback: impl FnMut(T) + 'static) {
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
    // Keep the JS-side callback registration alive for the app's lifetime.
    closure.forget();
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(err) = JsFuture::from(promise).await {
            web_sys::console::error_1(
                &format!("listen: failed to register {event_name}: {err:?}").into(),
            );
        }
    });
}
