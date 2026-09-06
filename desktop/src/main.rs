mod app;

fn main() {
    // Surface a Rust panic during launch to the browser/Tauri console. Tauri
    // hides the webview console in release bundles, so a panic inside
    // `dioxus::launch` would otherwise be invisible. console.error survives the
    // CSP `unsafe-eval` we allowlist and works with or without a host helper.
    std::panic::set_hook(Box::new(|panic| {
        let msg = panic.to_string();
        let escaped = msg
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('\n', "\\n");
        let _ = js_sys::eval(&format!(
            "try{{console.error('DIOXUS STARTUP PANIC: {escaped}')}}catch(e){{}}"
        ));
    }));

    dioxus::launch(app::App);
}
