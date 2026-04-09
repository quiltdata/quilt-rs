use serde::de::DeserializeOwned;
use serde::Serialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke)]
    fn tauri_invoke_raw(cmd: &str, args: JsValue) -> js_sys::Promise;
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
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{:?}", e)))?;
    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}

/// Call a Tauri command with no arguments.
pub async fn invoke_unit<R: DeserializeOwned>(cmd: &str) -> Result<R, String> {
    let args = js_sys::Object::new();
    let promise = tauri_invoke_raw(cmd, args.into());
    let result = JsFuture::from(promise)
        .await
        .map_err(|e| e.as_string().unwrap_or_else(|| format!("{:?}", e)))?;
    serde_wasm_bindgen::from_value(result).map_err(|e| e.to_string())
}
