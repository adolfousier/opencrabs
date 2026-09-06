# Desktop Command Contract

Status legend:
- **ready** — implemented and suitable for normal desktop use
- **partial** — works, but with clear limitations or truthfulness gaps
- **unsupported** — exposed today but should be hidden/gated until real

## Sessions / Chat

| Command | Status | Notes |
|---|---|---|
| `list_sessions` | ready | Lists sessions from the service manager. |
| `create_session` | ready | Creates a session and returns summary info. |
| `rename_session` | ready | Renames an existing session. |
| `delete_session` | partial | Deletes a session. Destructive; the UI confirms, but the native boundary does not yet require proof of confirmation. |
| `get_session_messages` | ready | Reads message history for a session. |
| `send_message` | ready | Non-streaming message send path exists and returns the completed response. |
| `send_message_streaming` | unsupported | Removed from the desktop command surface after its frontend listener caused a WASM closure-lifetime crash. Reintroducing streaming requires an owned subscription and tested unlisten lifecycle. |
| `stop_generation` | unsupported | Removed with the unusable event-stream path. |

## Config / Providers / Onboarding

| Command | Status | Notes |
|---|---|---|
| `get_config` | ready | Returns app config summary for desktop panels. |
| `get_providers` | ready | Provider list available for provider UI. |
| `select_model` | ready | Desktop model selection persists the change and refreshes the provider summary. |
| `update_config` | partial | Native allowlist and value validation exist; UI confirmation and field policy remain incomplete. |
| `is_first_time_setup` | ready | Simple setup detection. |
| `get_available_providers` | ready | Returns onboarding provider catalog. |
| `validate_api_key` | partial | No longer blindly writes keys, but validation is heuristic/shape-based rather than live provider verification. |
| `save_onboarding_config` | partial | Writes onboarding config, but deserves stronger UX review around secrets and provider validation feedback. |
| `run_health_check` | partial | More honest than before, but still not a full operational health model. |

## Brain / Files

| Command | Status | Notes |
|---|---|---|
| `list_brain_files` | ready | Lists allowlisted brain files. |
| `read_brain_file` | ready | Reads allowlisted brain files. |
| `write_brain_file` | partial | Enforces name, size, non-empty content, and ownership-header checks; confirmation and stale-write protection remain absent. |
| `list_directory` | partial | Contained to workspace root, but workspace-root model still needs explicit production semantics. |
| `read_file_content` | partial | Constrained, but still tied to current workspace-root assumptions. |
| `get_workspace_root` | partial | Returns current desktop workspace root, but root resolution policy should be made explicit and persistent. |

## Tools / Skills / Dynamic Tools

| Command | Status | Notes |
|---|---|---|
| `list_tools` | ready | Lists core + dynamic tools. |
| `get_tool_details` | ready | Returns detailed tool metadata. |
| `approve_tool` | partial | Desktop can persist an approval decision for the selected tool/session, but does not yet mirror the TUI's richer inline approval event workflow. |
| `list_skills` | ready | Lists skills with disabled-state awareness. |
| `get_skill_details` | ready | Returns skill details. |
| `toggle_skill` | ready | Desktop toggles the persisted state and refreshes both list and selected detail. |
| `list_dynamic_tools` | ready | Lists dynamic tools from `tools.toml`. |
| `add_dynamic_tool` | partial | Powerful mutation surface; production UI should gate and validate more aggressively. |
| `remove_dynamic_tool` | partial | Rewrites `tools.toml` without native confirmation proof or a not-found error. |

## Cron / Channels / Usage / Panes / Voice / Updates

| Command | Status | Notes |
|---|---|---|
| `list_cron_jobs` | ready | Lists configured cron jobs. |
| `create_cron_job` | partial | Native input validation and a complete creation UI are not yet present. |
| `delete_cron_job` | partial | UI confirms deletion, but the native boundary does not require confirmation proof. |
| `toggle_cron_job` | ready | Desktop exposes enable/disable and refreshes the job list. |
| `trigger_cron_job` | ready | Desktop exposes manual trigger. |
| `list_cron_runs` | ready | Lists cron run history; desktop exposes a per-job history view after manual refresh or run. |
| `get_channel_statuses` | partial | Reports enablement plus configuration/credential readiness. It does not claim a live connection. |
| `toggle_channel` | ready | Desktop persists the change and refreshes channel status; runtime reconnect/reload is outside this command. |
| `get_usage_data` | ready | Usage data is available for dashboard views. |
| `get_pane_layout` | ready | Persists and restores the full split tree, focus, and pane/session mappings. |
| `split_pane` | ready | Updates the persisted split tree. |
| `close_pane` | ready | Compacts the persisted split tree and clears the closed pane mapping. |
| `set_pane_session` | ready | Persists pane/session mapping across desktop restarts. |
| `get_desktop_state` | ready | Restores the last valid route and selected session, falling back safely. |
| `save_desktop_state` | ready | Validates and persists the active route and selected session. |
| `get_voice_config` | unsupported | Returns an explicit `unsupported` capability state; no voice controls should be shown. |
| `transcribe_audio` | unsupported | Fails explicitly until real STT integration is wired. |
| `synthesize_speech` | unsupported | Fails explicitly until real TTS integration is wired. |
| `check_for_updates` | partial | Honest check path exists, but desktop updater story is not complete. |
| `install_update` | unsupported | Explicitly not wired for desktop-native install yet. |

## Stable release acceptance contract

A build may be called **stable** only when every statement below has current evidence from the exact release commit and artifact.

1. **Startup:** a clean native launch replaces the static boot shell with Dioxus UI, reaches an explicit ready or actionable failure state, and emits no uncaught WASM/JavaScript exception.
2. **First invoke:** `get_desktop_state` and `list_sessions` execute through the packaged Tauri bridge with named arguments intact; browser-only rendering is not accepted as IPC evidence.
3. **Sessions and chat:** session selection loads canonical persisted messages. Sending either uses the completed request/response command or a streaming transport whose listener ownership, unlisten lifecycle, completion refresh, error path, and cancellation are exercised end to end. The UI must never remain permanently busy.
4. **Capability truthfulness:** channel state is labelled configuration/credential readiness rather than liveness. Voice and native update installation remain absent while unsupported. No UI claims a backend capability that the release artifact cannot execute.
5. **Protected mutations:** session deletion, cron creation/deletion, dynamic-tool removal, and brain/config writes validate at the native boundary. Destructive actions require explicit target-bound confirmation; protected writes reject invalid, stale, oversized, or out-of-policy input and surface actionable errors.
6. **Reproducible artifact:** the release gate runs from `desktop/`, starts from a clean generated-output state, produces a matching hashed JS/WASM pair, runs frontend and native format/lint/test checks, packages the native app, and records commit, platform, artifact path, SHA-256, signing, notarization, stapling, fresh-install, and upgrade evidence.

## Current demonstrated gaps

- `main` and `origin/main` were aligned at `c4a0ee13` before this contract update; the pre-existing untracked worktree addition was `desktop/release-notes-v0.1.0-desktop.md`.
- `Trunk.toml` requests hashed assets, but the current generated `dist/index.html` references unhashed `opencrabs-desktop-ui.js` and `opencrabs-desktop-ui_bg.wasm`. Generated output is untracked and cannot be treated as release evidence.
- The former frontend streaming path invoked a Tauri event command without a safe listener lifecycle and could remain busy indefinitely. The desktop now uses the completed request/response command and reloads the canonical persisted transcript; streaming and cancellation remain deliberately absent.
- The release script checks hashed assets after building, but does not first remove stale output, run Clippy, perform native launch/IPC smoke testing, or record artifact/signing/install evidence.
- GitHub Issues are disabled for this repository, so this milestone cannot use the normal issue-first tracking path; the approved plan and this contract are the durable tracking record.
