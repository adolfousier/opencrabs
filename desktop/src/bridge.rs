use serde::Serialize;
use serde::de::DeserializeOwned;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::{JsCast, JsValue};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_futures::JsFuture;

#[cfg(target_arch = "wasm32")]
fn tauri_object() -> Result<JsValue, String> {
    let window = web_sys::window().ok_or("Desktop bridge could not access the browser window")?;
    let tauri = js_sys::Reflect::get(&window, &JsValue::from_str("__TAURI__"))
        .map_err(js_error_to_string)?;
    if tauri.is_undefined() || tauri.is_null() {
        return Err(
            "OpenCrabs is running in a browser preview, not the native Tauri app. Launch it with `cargo tauri dev`; browser previews cannot execute desktop actions."
                .to_string(),
        );
    }
    Ok(tauri)
}

#[cfg(target_arch = "wasm32")]
fn tauri_core_invoke(cmd: &str, args: JsValue) -> Result<js_sys::Promise, String> {
    let tauri = tauri_object()?;
    let core =
        js_sys::Reflect::get(&tauri, &JsValue::from_str("core")).map_err(js_error_to_string)?;
    let invoke = js_sys::Reflect::get(&core, &JsValue::from_str("invoke"))
        .map_err(js_error_to_string)?
        .dyn_into::<js_sys::Function>()
        .map_err(|_| "Tauri invoke API is unavailable".to_string())?;

    invoke
        .call2(&core, &JsValue::from_str(cmd), &args)
        .map_err(js_error_to_string)?
        .dyn_into::<js_sys::Promise>()
        .map_err(|_| "Tauri invoke did not return a Promise".to_string())
}

pub async fn invoke<R, A>(cmd: &str, args: A) -> Result<R, String>
where
    R: DeserializeOwned + 'static,
    A: Serialize,
{
    #[cfg(target_arch = "wasm32")]
    {
        let args = serde_wasm_bindgen::to_value(&args).map_err(|error| error.to_string())?;
        let value = JsFuture::from(tauri_core_invoke(cmd, args)?)
            .await
            .map_err(js_error_to_string)?;
        serde_wasm_bindgen::from_value(value).map_err(|error| error.to_string())
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (cmd, args);
        Err("Desktop bridge is only available in the wasm frontend".to_string())
    }
}

pub async fn invoke_unit<A>(cmd: &str, args: A) -> Result<(), String>
where
    A: Serialize,
{
    invoke::<serde_json::Value, _>(cmd, args).await.map(|_| ())
}

#[cfg(target_arch = "wasm32")]
fn js_error_to_string(err: JsValue) -> String {
    err.as_string().unwrap_or_else(|| format!("{err:?}"))
}
