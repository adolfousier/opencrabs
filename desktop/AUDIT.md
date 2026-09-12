> ⚠️ **SUPERSEDED (2026-07-26).** This audit captured the desktop app at **v0.3.74** — a broken build state with `src-ui/` paths that no longer exist, a missing core module, and unaddressed security gaps. It is retained below as a **historical record** of the issues found. The current state is in [`STABILITY-EVIDENCE.md`](STABILITY-EVIDENCE.md); the build/verify decisions are in [`docs/adr/`](docs/adr/). A resolution map follows; most findings below have since been addressed.

## Resolution status (as of the beta-stable build)

| Original finding | Status | Where |
|---|---|---|
| Build broken (16+ errors, missing `command_code_cli.rs`) | ✅ Fixed — both crates compile, all gates green | `release-verify.sh` |
| `src-ui/` type mismatches | ✅ N/A — frontend lives in `src/`; DTOs match the backend | `src/models.rs` |
| XSS via `dangerous_inner_html` | ✅ Not present — Dioxus renders escaped text nodes; no raw-HTML render of model output | `src/app.rs` |
| Overly permissive file access (no containment) | ✅ Fixed — workspace containment (`..` rejected, canonicalize + `starts_with`) | `src-tauri/src/commands/files.rs` |
| Overly permissive config/brain writes | ✅ Fixed — allowlisted config keys; brain filename + size + protected-header validation | `config_cmd.rs`, `brain.rs` |
| `'unsafe-inline'` script in CSP | ✅ Tightened — CSP allows only `'self'`, `'wasm-unsafe-eval'`, `'unsafe-eval'` (panic hook) | `src-tauri/tauri.conf.json` |
| Channel status fabricated (`alive: enabled`) | ✅ Truthful — UI labels it configuration/credential readiness, not connectivity | `src/app.rs` |
| No-op / stub commands | ✅ Removed or honestly marked — streaming/cancel removed; voice & update-install explicitly unsupported and labeled | `src-tauri/src/commands/` |
| Frontend never mounts | ✅ Fixed — built via the `dx` CLI; mounts natively | [ADR-0001](docs/adr/0001-build-dioxus-frontend-with-dx-cli.md) |
| No runtime verification | ✅ Added — reproducible ladder + native macOS smoke | [ADR-0002](docs/adr/0002-native-frontend-mount-verification.md) |

**Still open** (tracked, not regressions): macOS code signing / notarization / stapling (needs Apple Developer credentials); transitive dependency advisories inherited from the `opencrabs` core library; accessibility polish across panels.

---

# OpenCrabs Desktop — Audit Report (historical, v0.3.74)

**Date:** 2026-07-25
**Target:** `/Users/moe/Desktop/crabz/desktop`
**Version:** v0.3.74 (commit `71769cfb`)
**Type:** Tauri 2 desktop app (Rust backend + Dioxus WASM frontend)

---

## 1. Build Status — BROKEN

Both crates fail to compile.

### `opencrabs` (core library)
```
error[E0583]: file not found for module `command_code_cli`
  --> src/brain/provider/mod.rs:25:1
```
The file `src/brain/provider/command_code_cli.rs` is missing. It was deleted (git shows `D ../src/brain/provider/command_code_cli.rs` in the working tree status). The module declaration in `mod.rs` still references it.

### `opencrabs-desktop-ui` (Dioxus frontend)
15 compilation errors in `src-ui/src/components/`. Primary issues:
- `CronJobInfo` and `ChannelStatus` types in `src-ui/src/types.rs` are missing fields that the backend `commands/cron.rs` and `commands/channels.rs` return (e.g., `enabled`, `alive`, `id`, `name`, `cron_expr`, `display_name`)
- `bridge.rs` has a `providers_clone` reference that doesn't exist in the providers panel
- Stale type definitions that don't match the current backend API

### `opencrabs-desktop` (Tauri binary)
Cannot compile because it depends on the broken `opencrabs` core library.

---

## 2. Security Issues

### CRITICAL — XSS via `dangerous_inner_html` (chat.rs:58)
```rust
div { class: "msg-text", dangerous_inner_html: "{render_md(&m.content)}" }
```
AI-generated message content is rendered as raw HTML with zero sanitization. A malicious model response (or prompt injection from a compromised provider) can inject arbitrary JavaScript. The `render_md` function converts markdown to HTML but does not escape inline HTML or `<script>` tags. This is the primary attack surface — the model's output goes straight into the DOM.

### HIGH — Overly permissive file access (files.rs)
- `list_directory` accepts any path and reads the filesystem. No path validation or containment check — users can browse the entire filesystem.
- `read_file_content` reads any file the process has access to, including `.env`, `~/.ssh/`, `config.toml`, etc. No boundary prevents reading sensitive files.
- `get_workspace_root` returns `current_dir()` which can change. It should return a configured workspace path.

### HIGH — Overly permissive config write (config_cmd.rs / onboarding.rs)
- `update_config` takes arbitrary `section`, `key`, `value` strings and writes them to `config.toml`. No validation of the section/key namespace. A malicious frontend could write to any config key including `provider.*.api_key`.
- `write_brain_file` (brain.rs) allows writing any brain file content with no size limits or content validation. An empty `allowed` check passes but the content is written verbatim.
- `validate_api_key` (onboarding.rs) writes the API key to config immediately — no actual key validation is performed. It always returns `Ok(true)`.

### MEDIUM — CSP weakens security (tauri.conf.json:29)
```json
"csp": "default-src 'self'; style-src 'self' 'unsafe-inline' https://cdnjs.cloudflare.com; script-src 'self' 'unsafe-inline' https://cdnjs.cloudflare.com; ..."
```
- `'unsafe-inline'` for both `style-src` and `script-src` negates CSP protection against inline XSS and CSS injection.
- `script-src 'unsafe-inline'` is particularly dangerous given the `dangerous_inner_html` vector above.
- The `custom-protocol` feature is enabled by default but `connect-src` allows `https:` broadly.

### MEDIUM — Channel status is fabricated (channels.rs:35-38)
```rust
alive: enabled,  // just mirrors the enabled flag, doesn't check connectivity
error: if !enabled { Some("Not configured".to_string()) } else { None },
```
The `alive` field reports whether a channel is *enabled*, not whether it's actually connected or functioning. This gives users false confidence.

### MEDIUM — Stub no-op commands (no-op security implications)
| Command | File | Line | Issue |
|---|---|---|---|
| `stop_generation` | chat.rs | 93 | No-op — doesn't actually stop streaming |
| `approve_tool` | tools.rs | 57 | No-op — just logs, doesn't gate execution |
| `toggle_skill` | skills.rs | 58 | No-op — just logs |
| `toggle_channel` | channels.rs | 40 | Writes config but doesn't actually toggle the channel |
| `get_voice_config` | voice.rs | 15 | Returns hardcoded `off`/`false` — doesn't read config |
| `transcribe_audio` | voice.rs | 19 | Stub — returns placeholder string |
| `synthesize_speech` | voice.rs | 23 | Stub — returns placeholder string |
| `check_for_updates` | update.rs | 13 | Stub — always returns `None` |
| `install_update` | update.rs | 17 | Stub — always returns `Ok(())` |
| `run_health_check` | onboarding.rs | 60 | `provider_ok: true` hardcoded, `tools_count: 0` hardcoded |

---

## 3. Dependency Vulnerabilities

`cargo audit` found **5 vulnerabilities** and **25 warnings** in transitive dependencies:

| Severity | Crate | Issue | Advisory |
|---|---|---|---|
| **Vulnerability** | `protobuf` | Crash due to uncontrolled recursion | RUSTSEC-2024-0437 |
| **Vulnerability** | `quick-xml` | Unbounded namespace-declaration allocation (DoS) | RUSTSEC-2026-0195 |
| **Vulnerability** | `quick-xml` | Quadratic time on duplicate attribute names | RUSTSEC-2026-0194 |
| **Unmaintained** | `yaml-rust` | Unmaintained | RUSTSEC-2024-0320 |
| **Unsound** | `glib` | Iterator/DoubleEndedIterator unsoundness | RUSTSEC-2024-0429 |
| **Unsound** | `lru` | IterMut violates Stacked Borrows | RUSTSEC-2026-0002 |
| **Unmaintained** | `bincode` | Unmaintained | RUSTSEC-2025-0141 |
| **Unmaintained** | `atk`, `gdk`, `gdk-sys`, `gdkwayland-sys`, `gdkx11`, `atk-sys` | GTK3 bindings no longer maintained | RUSTSEC-2024-041x series |

Most of these are transitive dependencies from the core `opencrabs` library (which pulls in `tokio`, `rusqlite`, etc.). The direct `tauri` dependency chain is the main attack surface for the desktop binary.

---

## 4. Code Quality Issues

### Missing `sidebar.css`
`index.html` line 10 references `css/sidebar.css` but the file does not exist in `css/`. This causes a 404 on every page load. The sidebar styling is missing entirely (the `<aside>` element exists in the DOM but has no dedicated stylesheet rules beyond what's in `layout.css`).

### In-memory state only — no persistence
- `PaneLayout` in `panes.rs` uses a static `AtomicU32` counter. Pane state is lost on restart. No serialization to disk.
- `sidebar` and `nav_rail` state lives only in the Dioxus frontend — no persistence of which panel was last open.

### `send_message_streaming` cannot be cancelled
`stop_generation` (chat.rs:93) is a no-op. The streaming task spawned via `tauri::async_runtime::spawn` has no cancellation mechanism. Once started, it runs to completion even if the user clicks "stop."

### `tool_params_for` is hardcoded and incomplete
The `tools.rs` tool catalog is a hardcoded `Vec` in the Rust backend. It doesn't reflect dynamic tools loaded from `tools.toml`. The MCP dynamic tool commands (`list_dynamic_tools`, `add_dynamic_tool`, `remove_dynamic_tool`) manage `tools.toml` but the tool catalog shown in the UI doesn't include them.

### `get_channel_statuses` doesn't check actual connectivity
The `alive` field is set to `enabled`, not the result of an actual connectivity check. The function should probe the channel API to verify it's connected (e.g., send a test message or check bot token validity).

### `run_health_check` returns fake data
`provider_ok: true` is hardcoded (line 63 of onboarding.rs). `tools_count: 0` (line 64). These should actually validate provider connectivity and count available tools.

### No API key validation
`validate_api_key` (onboarding.rs:43-47) writes the key to config and immediately returns `true` without testing it against the provider's API. There's no actual verification that the key works.

### File size limits
`read_file_content` (files.rs) reads entire files into memory with no size limit. A 2GB binary file would crash the app.

---

## 5. Architecture Observations

### Two-process architecture
The app uses Tauri's multi-window/process model:
- **Backend**: Rust Tauri binary (`opencrabs-desktop`) — handles commands, file I/O, database, AI provider calls
- **Frontend**: Dioxus WASM (`opencrabs-desktop-ui`) — runs in a webview, communicates with backend via Tauri commands

### State management
- `AppState` in `lib.rs` holds `service_manager` (behind `Mutex<Option<...>>`) and `config` (behind `Arc<RwLock<...>>`)
- The `service_manager` starts as `None` and is set during setup — commands that require it will fail with "Service not initialized" if called before setup completes
- Config writes via `write_key` are file-based with full-file rewrite + merge strategy (not atomic writes)

### Frontend framework
Dioxus 0.6 with `web` feature — compiles to WASM, runs in Tauri's webview. The `use_signal`/`use_resource` hooks manage reactive state. The bridge layer (`bridge.rs`) calls Tauri commands via `invoke()`.

---

## 6. Prioritized Fix Suggestions

### Immediate (security)
1. **Sanitize AI output before rendering** — use a markdown sanitizer (e.g., `ammonia` or `mdfmt`) before inserting into `dangerous_inner_html`, or switch to a safe markdown renderer that emits Dioxus VNodes instead of raw HTML
2. **Add path containment** to `list_directory` and `read_file_content` — restrict to workspace root or home directory
3. **Validate API keys** in `validate_api_key` — actually test the key against the provider
4. **Remove `'unsafe-inline'` from `script-src`** in CSP — this is the #1 defense-in-depth layer for XSS

### High priority (correctness)
5. **Restore `command_code_cli.rs`** — fix the missing module to unblock the core library build
6. **Fix `types.rs` in `src-ui`** — align `CronJobInfo`, `ChannelStatus`, and other types with the backend API responses
7. **Implement `stop_generation`** — add a cancellation token or abort handle so streaming can actually be stopped
8. **Implement `approve_tool`** — the tool approval command should actually gate tool execution, not just log

### Medium priority (quality)
9. **Create `css/sidebar.css`** or remove the reference from `index.html`
10. **Implement `run_health_check` properly** — actually test providers and count tools
11. **Add file size limits** to `read_file_content`
12. **Implement `check_for_updates`** and `install_update` using GitHub API or `auto-update` crate
13. **Persist pane layout** to disk so it survives restarts
14. **Remove no-op commands** or implement them properly — stub commands erode trust in the system

### Low priority (maintenance)
15. **Update transitive dependencies** — run `cargo update` regularly and audit for resolved advisories
16. **Add integration tests** for the Tauri command handlers
17. **Replace hardcoded tool catalog** with dynamic loading from `tools.toml`
18. **Add actual channel connectivity checks** instead of mirroring the `enabled` flag

---

## 7. Summary

| Category | Count | Severity |
|---|---|---|
| Build errors | 16+ | 🔴 Blocked |
| Critical security (XSS) | 1 | 🔴 Critical |
| High security (path traversal, no-op gating) | 4 | 🟠 High |
| Medium security (fake status, stub health) | 5 | 🟡 Medium |
| Code quality (missing CSS, no persistence) | 4 | 🟡 Medium |
| Dependency vulnerabilities | 5 | 🟠 High |
| Architecture issues | 3 | 🟡 Medium |

The project is in a **broken build state** with significant security gaps, particularly around XSS rendering of AI content and overly broad filesystem/config access. The core library has a missing module that must be restored before any compilation or testing is possible.
