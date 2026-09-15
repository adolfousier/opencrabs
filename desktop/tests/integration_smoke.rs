#[test]
fn desktop_state_is_persisted_by_the_backend_contract() {
    let panes_rs = std::fs::read_to_string(backend_root().join("src/commands/panes.rs"))
        .expect("read panes.rs");
    assert!(panes_rs.contains("desktop-panes.toml"));
    assert!(panes_rs.contains("save_desktop_state"));
    assert!(panes_rs.contains("set_pane_session"));
    assert!(panes_rs.contains("remove_pane"));
}

#[test]
fn diagnostics_backend_redacts_and_bounds_log_output() {
    let diagnostics_rs =
        std::fs::read_to_string(backend_root().join("src/commands/diagnostics.rs"))
            .expect("read diagnostics.rs");
    assert!(diagnostics_rs.contains("MAX_LOG_BYTES"));
    assert!(diagnostics_rs.contains("sensitive_line"));
    assert!(diagnostics_rs.contains("api_key"));
}

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn backend_root() -> PathBuf {
    repo_root().join("src-tauri")
}

#[test]
fn desktop_contract_docs_exist() {
    let root = repo_root();
    assert!(root.join("desktop-command-contract.md").exists());
    assert!(root.join("docs/release-strategy.md").exists());
    assert!(root.join("docs/release-runbook.md").exists());
    assert!(root.join("docs/release-checklist.md").exists());
    assert!(root.join("docs/diagnostics.md").exists());
}

#[test]
fn frontend_metadata_matches_manual_update_policy() {
    let cargo_toml =
        std::fs::read_to_string(backend_root().join("Cargo.toml")).expect("read tauri cargo");
    assert!(cargo_toml.contains("auto_update_install = false"));
    assert!(cargo_toml.contains("distribution = \"signed-manual\""));
}

#[test]
fn updater_backend_is_honest_about_manual_install() {
    let update_rs = std::fs::read_to_string(backend_root().join("src/commands/update.rs"))
        .expect("read update.rs");
    assert!(update_rs.contains("intentionally unsupported"));
    assert!(update_rs.contains("signed release artifact"));
}

#[test]
fn bridge_has_non_wasm_guardrail() {
    let bridge_rs =
        std::fs::read_to_string(repo_root().join("src/bridge.rs")).expect("read bridge.rs");
    assert!(bridge_rs.contains("Desktop bridge is only available in the wasm frontend"));
}

#[test]
fn startup_promotes_chat_to_ready_before_secondary_panel_loads() {
    let app_rs = std::fs::read_to_string(repo_root().join("src/app.rs")).expect("read app.rs");
    let ready_at = app_rs
        .find("status.set(\"Ready\".to_string())")
        .expect("ready status transition");
    let secondary_load_at = app_rs
        .find("get_workspace_root")
        .expect("secondary workspace load");

    assert!(
        ready_at < secondary_load_at,
        "chat-critical readiness must precede secondary panel loading"
    );
    assert!(app_rs.contains("Some desktop panels are unavailable"));
    assert!(app_rs.contains("Files unavailable:"));
    assert!(app_rs.contains("Brain files unavailable:"));
}

#[test]
fn bridge_preserves_the_tauri_core_receiver_and_named_args() {
    let bridge_rs =
        std::fs::read_to_string(repo_root().join("src/bridge.rs")).expect("read bridge.rs");

    assert!(bridge_rs.contains(".call2(&core, &JsValue::from_str(cmd), &args)"));
    assert!(bridge_rs.contains("JsFuture::from(tauri_core_invoke(cmd, args)?)"));
    assert!(!bridge_rs.contains("tauri_invoke_without_args"));
    assert!(!bridge_rs.contains("tauri_invoke_with_args"));
}

#[test]
fn chat_uses_completed_request_response_without_wasm_event_closures() {
    let app_rs = std::fs::read_to_string(repo_root().join("src/app.rs")).expect("read app.rs");
    let bridge_rs =
        std::fs::read_to_string(repo_root().join("src/bridge.rs")).expect("read bridge.rs");
    let backend_lib =
        std::fs::read_to_string(backend_root().join("src/lib.rs")).expect("read backend lib");
    let backend_chat = std::fs::read_to_string(backend_root().join("src/commands/chat.rs"))
        .expect("read backend chat");

    assert!(app_rs.contains("\"send_message\""));
    assert!(app_rs.contains("Message sent, but the transcript could not refresh"));
    assert!(!app_rs.contains("send_message_streaming"));
    assert!(!app_rs.contains("stop_generation"));
    assert!(!bridge_rs.contains("Closure<"));
    assert!(!backend_lib.contains("chat::send_message_streaming"));
    assert!(!backend_chat.contains("send_message_streaming"));
    assert!(!backend_chat.contains("tauri::Emitter"));
}

#[test]
fn index_is_dioxus_native_with_mount_root_and_panic_hook() {
    // The frontend is built via the `dx` CLI (the Dioxus way). index.html is a
    // clean shell with the Dioxus mount root; dx injects its own wasm loader. It
    // must NOT carry Trunk markup — building via Trunk left `dioxus::launch` as a
    // silent no-op, so the frontend never mounted. A Rust panic during launch is
    // surfaced via the main.rs panic hook (console.error).
    let index = std::fs::read_to_string(repo_root().join("index.html")).expect("read index.html");
    let main_rs = std::fs::read_to_string(repo_root().join("src/main.rs")).expect("read main.rs");

    // Dioxus mounts into the #main root.
    assert!(
        index.contains(r#"<div id="main">"#),
        "index.html missing Dioxus mount root #main"
    );
    // No Trunk markup — we build via dx, not Trunk.
    assert!(
        !index.contains("data-trunk"),
        "index.html still carries Trunk markup (build via dx instead)"
    );
    assert!(
        !index.contains("TrunkApplicationStarted"),
        "index.html references the Trunk event (obsolete under dx)"
    );
    // main.rs launches Dioxus and surfaces panics.
    assert!(
        main_rs.contains("dioxus::launch"),
        "main.rs does not launch Dioxus"
    );
    assert!(main_rs.contains("set_hook"), "main.rs missing panic hook");
}

#[test]
fn frontend_and_native_register_first_invoke_commands() {
    let app_rs = std::fs::read_to_string(repo_root().join("src/app.rs")).expect("read app.rs");
    let backend_lib =
        std::fs::read_to_string(backend_root().join("src/lib.rs")).expect("read backend lib");

    for command in ["get_desktop_state", "list_sessions"] {
        assert!(app_rs.contains(&format!("\"{command}\"")));
        assert!(backend_lib.contains(command));
    }
}
