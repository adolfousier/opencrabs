# Changelog

All notable changes to OpenCrab will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.39] - 2026-02-28

### Added
- **Status bar below input** (`02220e7`, `9dd4cab`) — Persistent one-line status bar replaces the old sticky overlay. Displays session name (orange), provider / model, working directory, and approval policy badge. Session and directory were moved from the header into the status bar; the full-width header bar was removed entirely
  - `src/tui/render/mod.rs`, `src/tui/render/input.rs`
- **Immediate thinking spinner in chat** (`57ffc40`) — A spinner and "OpenCrabs is thinking..." line appears in the chat area as soon as a request is submitted, before any streaming content arrives. Eliminates the blank gap while the provider is warming up
  - `src/tui/render/chat.rs`
- **Per-session context token cache** (`57ffc40`) — When switching between sessions or reloading, the last known input token count is restored from an in-memory cache instead of showing `–`. Accurate token counts are re-confirmed on the next API response
  - `src/tui/app/state.rs`, `src/tui/app/messaging.rs`

### Fixed
- **ctx shows accurate token count for providers that report zero usage** (`033043f`) — Providers like MiniMax always return `usage: {total_tokens: 0}` in streaming responses. The provider now uses its pre-computed `message_tokens + tool_schema_tokens` (serialised OpenAI JSON) as the fallback, so the ctx display (e.g. `29K/200K`) matches the debug log exactly instead of showing the lower raw-text estimate (~14K)
  - `src/brain/provider/custom_openai_compatible.rs`, `src/brain/agent/service/tool_loop.rs`
- **Compact app title in sessions/help screens** (`bc80a0f`) — Removed blank lines and border from the app title block in non-chat screens. Title now occupies exactly one row, reclaiming vertical space
  - `src/tui/render/mod.rs`
- **Extra blank space below chat history** (`d469f01`) — Scroll calculation used `reserved = 3` left over from removed borders/overlay. Changed to `reserved = 1` (top padding only), eliminating the gap at the bottom of the chat area
  - `src/tui/render/chat.rs`
- **Duplicate thinking indicators removed** (`aa08d68`, `57ffc40`) — "OpenCrabs is thinking" was appearing twice: once as an inline tool-group hint and once in the status bar. Removed both; the single spinner in the chat area is the sole indicator
  - `src/tui/render/chat.rs`, `src/tui/render/input.rs`
- **Muted orange replaces bright yellow** (`02220e7`) — `Color::Yellow` replaced with `Color::Rgb(215, 100, 20)` for ctx percentage, sessions spinner, and pending-approval badge. Intentional dark-golden `Rgb(184, 134, 11)` unchanged
  - `src/tui/render/input.rs`, `src/tui/render/sessions.rs`

## [0.2.38] - 2026-02-27

### Fixed
- **Splash screen shows actual custom provider name** — `resolve_provider_from_config()` was returning the hardcoded string `"Custom"` instead of the actual provider name (e.g. `"nvidia"`, `"moonshot"`). Now correctly returns the name key from `providers.active_custom()`
  - `src/config/types.rs`
- **Full request payload in debug logs** — Removed `.take(1000)` truncation from OpenAI-compatible request debug log. The API request itself was never truncated; only the log display was. Now logs the full payload for accurate debugging
  - `src/brain/provider/custom_openai_compatible.rs`
- **Standalone reasoning render during thinking-only phase** — Providers that emit reasoning before any response text (e.g. Kimi K2.5, DeepSeek) now render a visible `🦀 OpenCrabs is thinking...` block with live reasoning content while `streaming_response` is still empty. Previously the screen was blank until the first response chunk
  - `src/tui/render/chat.rs`
- **Streaming redraws per chunk** — Drain loop in runner now breaks immediately on `ResponseChunk` events, triggering a redraw after each text chunk. Previously `ReasoningChunk` events also broke the loop, preventing response text from rendering in real-time on some providers
  - `src/tui/runner.rs`
- **Approval dialog shows full tool parameters** — Tool approval dialog previously truncated parameter values at 60 characters. Now renders all parameters line-by-line without truncation so the full context is visible when deciding whether to approve
  - `src/tui/render/tools.rs`
- **Tool approval waits indefinitely** — Removed 120-second timeout on tool approval callbacks. The dialog now waits as long as needed for the user to approve or deny
  - `src/tui/app/state.rs`
- **Green dot pulse slowed** — Animated `●` dot in tool call groups now pulses on a ~1.6s cycle (`animation_frame / 8`) instead of the previous fast flicker (`animation_frame / 3`)
  - `src/tui/render/tools.rs`

### Removed
- **Plan Mode completely removed** (~1400 lines deleted) — All plan execution code, UI, keyboard shortcuts, and state removed. Includes `plan_exec.rs` module, `AppMode::Plan` variant, `PlanApprovalState`/`PlanApprovalData` structs, Ctrl+P/Ctrl+A/Ctrl+R/Ctrl+I shortcuts, plan approval intercept in input handler, plan help screen section, and plan re-exports. Plan Mode section removed from README
  - `src/tui/app/plan_exec.rs` (deleted), `src/tui/app/input.rs`, `src/tui/app/messaging.rs`, `src/tui/app/mod.rs`, `src/tui/app/state.rs`, `src/tui/events.rs`, `src/tui/mod.rs`, `src/tui/render/chat.rs`, `src/tui/render/help.rs`, `src/tui/render/mod.rs`, `src/tui/render/tools.rs`, `README.md`

## [0.2.37] - 2026-02-26

### Added
- **Per-session provider selection** (`5689cd9`) — Each session can now have its own LLM provider. Configure per-session via `/models` or in `config.toml` under `[session.*.provider]`. Parallel execution of multiple sessions with different providers supported
  - `src/brain/agent/service.rs`, `src/brain/mod.rs`, `src/tui/app/state.rs`, `src/tui/render.rs`, `config.toml.example`
- **Arrow key navigation in multiline input** (`9b544f9`) — Arrow Up/Down now navigate between lines in the multiline input field, not just recall history. Cursor moves within the multiline content as expected
  - `src/tui/app/input.rs`, `src/tui/render/input.rs`
- **Test units for multi-session and multi-model** (`cf7ff0d`) — Added unit tests covering session-aware approval policies, model switching within sessions, and provider key isolation
  - `src/brain/agent/service/tests/approval_policies.rs`, `src/brain/agent/service/tests/basic.rs`

### Fixed
- **Session-aware tool approvals** (`846f228`) — Tool approval policies now correctly apply per-session. Approval state is stored with session ID, not globally. Async model fetching improved with better error handling
  - `src/brain/agent/service.rs`, `src/brain/mod.rs`, `src/tui/app/state.rs`
- **Custom provider name field** (`c22a05a`) — Onboarding now pre-fills the custom provider name field. Model fetching uses existing key if available instead of requiring re-entry. Provider name displays correctly in `/models` dialog
  - `src/tui/onboarding.rs`, `src/tui/app/dialogs.rs`, `src/brain/provider/custom_openai_compatible.rs`

### Refactored
- **Split `agent/service.rs`** (`8f9c160`) — Extracted into module directory: `service/builder.rs`, `service/context.rs`, `service/helpers.rs`, `service/messaging.rs`, `service/mod.rs`. Improved code organization and testability
- **Split `render.rs`** (`6247666`) — Extracted 3312-line file into `render/` module directory with `render/mod.rs`, `render/input.rs`, `render/dialogs.rs`, `render/components.rs`
- **Cargo fmt pass** (`d02fcf7`) — Full codebase formatting enforcement

## [0.2.36] - 2026-02-26

### Fixed
- **Custom provider `/models` dialog** (`fc0626c`) — Model name is now a free-text input instead of a hardcoded list. Labels show the actual provider name (e.g. "Moonshot") instead of generic "Custom". Onboarding flow updated to match
  - `src/tui/app/dialogs.rs`, `src/tui/render.rs`, `src/tui/onboarding.rs`, `src/config/types.rs`, `src/tui/app/state.rs`, `src/brain/provider/anthropic.rs`, `src/brain/provider/custom_openai_compatible.rs`, `README.md`
- **Input UX improvements** (`7804ab3`) — Esc scrolls viewport to bottom of conversation. Arrow Up recalls previously cleared/stashed input text. Cursor renders as a block highlighting the current character instead of a thin line. Escape timer resets when processing completes so next Esc behaves correctly
  - `src/tui/app/input.rs`, `src/tui/app/messaging.rs`, `src/tui/app/state.rs`, `src/tui/render.rs`
- **Strip Kimi HTML comment markup** (`47b1d58`) — Kimi K2.5 embeds reasoning and hallucinated tool calls as HTML comments (`<!-- reasoning -->`, `<!-- tools-v2: -->`) in the content field. Extended `filter_think_tags` and `strip_think_blocks` to strip these alongside `<think>`. Fixed `extract_reasoning` to handle multiple reasoning blocks per message. Added Moonshot/Kimi pricing (K2.5, K2 Turbo, K2) to compiled-in defaults and `usage_pricing.toml.example`
  - `src/brain/provider/custom_openai_compatible.rs`, `src/pricing.rs`, `src/tui/app/messaging.rs`, `usage_pricing.toml.example`
- **`/models` provider switch: never overwrite user API keys** (`5120bf5`) — Killed sentinel string `"__EXISTING_KEY__"` from `/models` dialog entirely. Replaced with boolean flag `model_selector_has_existing_key`. Only writes to `keys.toml` when user actually types a new key. Disables all other providers on disk before enabling selected one. Added `is_real_key` guard in `merge_provider_keys` for all providers
  - `src/config/types.rs`, `src/tui/app/dialogs.rs`, `src/tui/app/state.rs`, `src/tui/render.rs`
- **Model change context hint for agent** (`ce8e422`) — When user switches model via `/models`, a `[Model changed to X (provider: Y)]` hint is prepended to the next user message via `pending_context` (same mechanism as `/cd`), so the LLM is aware of the switch. TUI status message also shown in chat. Custom provider uses user-configured name (e.g. "nvidia") instead of generic label. Fallback provider key changed from `providers.custom.default` to `providers.custom` to avoid stale config entries
  - `src/tui/app/dialogs.rs`, `src/tui/app/messaging.rs`

## [0.2.35] - 2026-02-26

### Added
- **Animated tool call dots** — Green `●` dot pulses (`●`/`○`) while tools are actively processing, stays solid when finished. Visually distinguishes active tool execution from completed groups
- **Inline thinking indicator during tool execution** — "OpenCrabs is thinking..." now renders inline above the active tool group instead of as a sticky overlay, preventing overlap with tool call content
- **`.github/CODEOWNERS`** — Auto-assigns `@adolfousier` as reviewer on all PRs

### Fixed
- **TUI spacing improvements** — Removed double blank lines between messages and tool groups. Added proper spacing before thinking sections and between thinking hint and expanded content
- **Inline code background removed** — `bg(Color::Black)` on backtick code spans in markdown renderer removed for cleaner look. Thinking hints use subtle `Rgb(90,90,90)` text with no background
- **Sudo prompt bleeding into TUI** — Added `-p ""` flag to `sudo -S` to suppress sudo's native "Password:" prompt from writing directly to the terminal
- **`cargo fmt` full codebase pass** — Enforced official Rust style guide across 92 files
- **Test fixes** — `stream_complete()` tests updated to destructure `(LLMResponse, Option<String>)` tuple return with reasoning assertions. `write_secret_key` doctest fixed (missing import + Result return type)

## [0.2.34] - 2026-02-26

### Added
- **Reasoning/thinking persistence** — MiniMax (and other providers that emit `reasoning_content`) now accumulate thinking content during streaming, persist it to DB with `<!-- reasoning -->` markers, and reconstruct it on session reload. Reasoning is rendered as a collapsible "Thinking" section on assistant messages
- **Real-time message persistence per step** — Assistant text is written to DB after each tool iteration, not just at the end. Crash or disconnect mid-task no longer loses intermediate text
- **Collapsible reasoning UI** — Ctrl+O now toggles both tool groups and reasoning sections. Collapsed by default, expandable inline with dimmed italic style matching the streaming "Thinking..." indicator

### Fixed
- **MiniMax intermediate text lost on reload** — Tool call indices from OpenAI-compatible providers collided with the text content block at index 0 in `stream_complete()`, overwriting accumulated text. Tool indices now offset by +1. Fixes [#10](https://github.com/adolfousier/opencrabs/issues/10)
- **TUI unresponsive after onboarding** — `rebuild_agent_service()` only attached the approval callback, dropping `progress_callback`, `message_queue_callback`, `sudo_callback`, and `working_directory`. All callbacks are now preserved from the existing agent service. Fixes [#10](https://github.com/adolfousier/opencrabs/issues/10)
- **Tool loop false positives eliminated** — Replaced 115-line per-tool signature matching with 7-line universal input hash. Different arguments = different hash = no false detection. Same args repeated 8 times = real loop
- **Chat history lost on mid-task exit** — Exiting while the agent was between tool iterations discarded the conversation. Now persists accumulated text before exit
- **Clippy warnings** — Collapsed nested `if` statements in `service.rs` and `input.rs`

## [0.2.33] - 2026-02-25

### Added
- **Streaming `/rebuild`** — Live compiler output streamed to chat during build. On success, binary is `exec()`-replaced automatically (no prompt, no restart). Auto-clones repo for binary-only users if no source tree found
- **Centralized `usage_pricing.toml`** — Runtime-editable pricing table for all providers (Anthropic, OpenAI, MiniMax, Google, DeepSeek, Meta). Edit live, changes take effect on next `/usage` open without restart. Written automatically on first run during onboarding
- **All-time `/usage` breakdown** — Shows cost grouped by model across all sessions. Historical sessions with stored tokens but zero cost get estimated costs (yellow `~$X.XX` prefix). Unknown models shown as `$0.00` instead of silently ignored
- **`/cd` context injection** — When user changes working directory via `/cd`, a context hint is queued and prepended to the next message so the LLM knows about the directory change without the user having to explain. Uses new `pending_context` vec on App state
- **Tool approval policy preservation across compaction** — Compaction summary prompt now includes `## Tool Approval Policy` section. All 4 continuation messages (pre-loop, mid-tool-loop, emergency, mid-loop) inject `CRITICAL: Tool approval is REQUIRED` when auto-approve is off. Agent can no longer "forget" approval policy after context resets
- **Dropped stream detection + retry** — Detects when provider streams end without `[DONE]`/`MessageStop` (stop_reason is None). Retries up to 2 times transparently, discarding partial responses. After 2 failures, proceeds gracefully with partial response

### Fixed
- **Context compaction streamed, not frozen** — `compact_context` uses `stream_complete` so the TUI event loop stays alive during compaction. Previously froze the UI for 2-5 minutes on large contexts
- **Compaction summary visible in chat** — Summary fires via `CompactionSummary` progress event after streaming, rendered in chat so user can see what was preserved
- **TUI state reset post-compaction** — Resets `streaming_response` + `active_tool_group` on compaction so the UI is clean for continuation
- **Compaction request budget cap** — Capped at 75% of context window with 16k token overhead (was 8k). Prevents the compaction request itself from exceeding the provider limit (was sending 359k tokens)
- **Real-time context counter** — Live token count updates in header during streaming
- **`/models` paste support** — API keys can be pasted into the model selection dialog
- **Pricing: $0 cost for all sessions** — `PricingConfig` struct used `HashMap<String, Vec<PricingEntry>>` but TOML has `entries = [...]` wrapper. Added `ProviderBlock` to match schema correctly
- **Pricing: MiniMax $0** — Stream chunks don't include model name. Falls back to request model
- **Pricing: legacy format migration** — Auto-migrates `[[usage.pricing.X]]` on-disk format to current schema
- **Clippy: collapsible_if** — Fixed in `rebuild.rs` and `pricing.rs`

## [0.2.32] - 2026-02-24

### Added
- **A2A Bearer token authentication** -- JSON-RPC endpoint (`/a2a/v1`) now supports `Authorization: Bearer <key>` when `api_key` is configured. Agent card and health endpoints remain public for discovery. Key can be set in `config.toml` or `keys.toml` under `[a2a]`
- **A2A task persistence** -- Tasks are persisted to SQLite (`a2a_tasks` table, auto-migration) on create, complete, fail, and cancel. Active tasks are restored from DB on server startup so in-flight work survives restarts
- **A2A SSE streaming (`message/stream`)** -- Real-time task updates via Server-Sent Events per A2A spec. Each SSE `data:` line is a JSON-RPC 2.0 response containing a `Task`, `TaskStatusUpdateEvent` (with `final: true` on completion), or `TaskArtifactUpdateEvent`. Agent card now advertises `streaming: true`

## [0.2.31] - 2026-02-24

### Fixed
- **Tool calls stacking into one giant group on reload** — Removed cross-iteration merge logic that collapsed all consecutive tool groups into a single "N tool calls" block, eating intermediate text between iterations. Each iteration's `<!-- tools-v2: -->` marker now produces its own collapsible group, matching live session behavior
- **Tool group ordering during live streaming** — IntermediateText handler flushed the previous iteration's tool group *after* pushing the new step's text, causing tools to appear below the wrong text. Now flushes tools first, matching DB order
- **Ctrl+O blocked during approval** — All non-approval keys were eaten when an approval dialog was pending, preventing users from collapsing expanded tool groups to see the approval. Ctrl+O now works during approval
- **Auto-collapse tool groups on approval** — When an approval request arrives, all tool groups are automatically collapsed so the approval dialog is immediately visible without manual intervention
- **EXA MCP fallback on empty API key** — Empty string API key (`""`) caused EXA to attempt direct API mode instead of free MCP. Now treats empty keys as absent, correctly falling back to MCP (aaefd3d)
- **Brave search registered without enabled flag** — `brave_search` tool registered whenever an API key existed, ignoring `enabled = false` in config.toml. Now requires both `enabled = true` and a valid API key

## [0.2.30] - 2026-02-24

### Added
- **Agent-to-Agent (A2A) Protocol** — HTTP gateway implementing A2A Protocol RC v1.0 for peer-to-peer agent communication via JSON-RPC 2.0. Supports `message/send`, `tasks/get`, `tasks/cancel`. Contributed by [@koatora20](https://github.com/koatora20) in [#9](https://github.com/adolfousier/opencrabs/pull/9)
- **Bee Colony Debate** — Multi-agent structured debate protocol based on ReConcile (ACL 2024) confidence-weighted voting. Configurable rounds with knowledge-enriched context from QMD memory search
- **Dynamic Agent Card** — `/.well-known/agent.json` endpoint with skills generated from the live tool registry
- **A2A Documentation** — Config example, README section with curl examples, TOOLS.md/SECURITY.md/BOOTSTRAP.md reference templates updated

### Fixed
- **Tool calls vanishing from TUI** — Tool call context (the collapsible bullet with tool names and output) disappeared from the chat after the agent responded. Tool group was being attached to a previous assistant message instead of rendered inline before the current response. Now matches the DB reload layout: tool calls appear above the response text, visible in both live and reloaded sessions
- **Tool loop false positives** — `web_search` and `http_request` calls with different arguments were treated as identical by the loop detector, killing legitimate multi-search flows. Signatures now include query/URL arguments. Thresholds raised (8 default, 4 for modification tools) with a 50-call history window
- **Tool call groups splitting on session reload** — Each tool-loop iteration wrote a separate DB marker, so "2 tool calls" became two "1 tool call" entries on reload. Fixed in v0.2.31
- **Brave search registered without enabled flag** — `brave_search` tool was available to the agent even when `enabled = false` in config.toml. Now requires both `enabled = true` and API key
- **EXA MCP fallback on empty API key** — Empty string API key (`""`) in keys.toml caused EXA to use direct API mode instead of free MCP mode. Now treats empty keys as absent, correctly falling back to MCP
- **A2A: Removed unused `rusqlite` dependency** — A2A handler no longer pulls in rusqlite; uses existing SQLite infrastructure
- **A2A: UTF-8 slicing safety** — Fixed potential panic on multi-byte characters in message truncation
- **A2A: Restrictive CORS by default** — No cross-origin requests allowed unless `allowed_origins` is explicitly configured
- **A2A: Handler module split** — Monolithic `handler.rs` split into `handler/mod.rs`, `handler/service.rs`, `handler/processing.rs` for maintainability

### Changed
- **A2A: Agent card uses tool registry** — Skills reflect actual available tools instead of hardcoded list
- **A2A: Server wiring** — Proper integration with AppState, config, and tool registry
- **Web search defaults in README** — Updated to reflect DuckDuckGo + EXA as default (no key needed), Brave as optional

## [0.2.29] - 2026-02-24

### Added
- **Tool Parameter Normalization** — Centralized alias map in tool registry corrects common LLM parameter name mistakes (`query`→`pattern`, `cmd`→`command`, `file`→`path`) before validation. Works across all tools
- **Brain Tool Reference** — System prompt lists exact required parameter names for each tool
- **TOOLS.md Parameter Table** — New user template includes tool parameter quick-reference table

### Fixed
- **Token Counting for OpenAI-Compatible Providers** — `stream_complete` now reads `input_tokens` from `MessageDelta` events. Previously always 0 for MiniMax and other OpenAI-compatible providers, causing incorrect session token totals and context percentage
- **Session Search UTF-8 Crash** — Fixed panic on multi-byte characters when truncating message content (`floor_char_boundary` instead of raw byte slice)
- **Session Search Deadlock** — Search uses `try_lock()` on embedding engine mutex with FTS-only fallback when backfill is running
- **Embedding Backfill Lock Contention** — Processes one document at a time, releasing engine lock between each
- **Tool Loop False Positive** — `session_search` loop detector signature includes `operation:query` to distinguish calls
- **Grep Traversal Performance** — Skips `target/`, `node_modules/`, `.git/` and other heavy directories; default limit of 200 matches
- **Thinking Indicator Overlap** — "OpenCrabs is thinking..." no longer overlaps chat content
- **App Exit Hang** — `process::exit()` prevents tokio runtime hanging on `spawn_blocking` threads
- **Ctrl+C Force Exit** — Cancel token + 1-second timeout fallback when tools are stuck

### Changed
- **App Module Split** — `app.rs` (4,960 lines) split into `state.rs`, `input.rs`, `messaging.rs`, `plan_exec.rs`, `dialogs.rs` with `mod.rs` declarations only
- **Doc Comments** — Converted `//` to `///` doc comments across codebase
- **7 Test Fixes** — Fixed `test_create_provider_no_credentials` (PlaceholderProvider) and 6 onboarding tests (config pollution, channel routing)

## [0.2.28] - 2026-02-23

### Added
- **Brain Setup Persistence** — BrainSetup step loads existing `USER.md`/`IDENTITY.md` from workspace as truncated preview on re-run. No extra files — brain files are the source of truth
- **Brain Setup Skip** — `Esc` to skip, unchanged inputs skip regeneration, empty inputs skip gracefully
- **Brain Regeneration Context** — On re-run, LLM receives current workspace brain files (not static templates), preserving manual edits as context. Generated content overwrites existing files
- **Splash Auto-Close** — Splash screen auto-closes after 3 seconds
- **Slack Debug Logging** — Added debug tracing for Slack message routing (user, channel, bot_id)

### Fixed
- **Model List Isolation** — Minimax and Custom provider model lists no longer mix. Each provider loads only its own models from `config.toml.example`. Previously `load_default_models()` dumped all providers into one shared list
- **Workspace Path Trim** — Workspace path is trimmed on confirm, preventing ghost directories from trailing spaces
- **HealthCheck Skipping BrainSetup** — HealthCheck step returned `WizardAction::Complete` immediately, skipping BrainSetup. Now returns `WizardAction::None` to advance to BrainSetup
- **Brain File Overwrite on Regeneration** — `apply_config()` skipped writing brain files if they already existed, even after regeneration. Now overwrites when AI-generated content is available

### Changed
- **Renamed `about_agent` → `about_opencrabs`** — Field and label renamed from "Your Agent" to "Your OpenCrabs" for clarity

## [0.2.27] - 2026-02-23

### Added
- **Named Custom Providers** — Define multiple named OpenAI-compatible providers via `[providers.custom.<name>]` (e.g. `lm_studio`, `ollama`). First enabled one is used. Legacy flat `[providers.custom]` format still supported

### Fixed
- **Stream Deduplication** — Fixed duplicated agent messages in chat when using LM Studio and other custom providers. Some providers send the full response in the final chunk's `message` field — falling back to `message` after receiving delta content duplicated everything
- **Database Path Tilde Expansion** — `~` in database path config was treated literally, creating a `~/` directory inside the repo. Added `expand_tilde()` to resolve to actual home directory
- **WhatsApp Onboarding** — Fixed WhatsApp channel setup to include QR code pairing step with auto-advance, skip and retry
- **Channel Onboarding Allowed Lists** — Fixed missing allowed users/channels/phones input fields on Telegram, Discord, WhatsApp and Slack setup screens

### Changed
- **README** — Provider examples updated to named custom provider format (`[providers.custom.lm_studio]`)
- **config.toml.example** — Database path uses smart default, custom providers use named format

## [0.2.26] - 2026-02-22

### Added
- **Streaming Tool Call Accumulation** — OpenRouter and Custom providers now correctly handle streaming tool calls. Added `StreamingToolCall`/`StreamingFunctionCall` structs with optional fields for incremental SSE deserialization, plus `ToolCallAccum` state machine that accumulates `id`, `name`, and `arguments` across chunks and emits on `finish_reason: "tool_calls"` or `[DONE]`
- **Input Sanitization** — Paste handler strips `\r\n`, takes first line only, trims whitespace. Storage layer (`write_secret_key`, `write_key`) also sanitizes before writing to TOML files
- **Auto-append `/chat/completions`** — Custom provider factory auto-appends `/chat/completions` to base URLs that don't include it, preventing silent 404s
- **Provider + Model in Completion** — Onboarding completion message now shows which provider and model were selected

### Fixed
- **Streaming Tool Calls Failing on OpenRouter/Custom** — Root cause: `StreamingToolCall` struct required `id` and `type` fields but SSE continuation chunks only send `index` + `function.arguments`. Made all fields optional except `index`. Removed unused `type` field
- **API Key Header Panic** — `headers()` used `.expect()` which panicked on invalid key characters (e.g. `\r` from paste). Now returns `Result<HeaderMap, ProviderError>` with descriptive error
- **Log Directory Path** — Logs were stored in `cwd/.opencrabs/logs/` (inside the repo) instead of `~/.opencrabs/logs/` (user workspace). Fixed `LogConfig`, `get_log_path()`, and `cleanup_old_logs()` to use home directory
- **Config/Keys Overwrite** — `Config::save()` was called in `app.rs` and `onboarding.rs`, destructively overwriting the entire TOML file. Replaced all instances with individual `write_key()`/`write_secret_key()` calls that read-modify-write without losing unrelated sections
- **Custom Provider Using Wrong Field** — Custom provider used `custom_api_key` while all other providers used `api_key_input`. Unified to `api_key_input` across all providers
- **Sentinel Prepended to Key** — `__EXISTING_KEY__` sentinel was prepended to actual API key on paste. Fixed `CustomApiKey` handlers to clear sentinel before appending new input
- **URL Appended to Key** — Pasting from clipboard could include `\r` and trailing URL text in API key field. Added paste sanitization at input handler and storage layer

### Changed
- **Renamed `openai.rs` → `custom_openai_compatible.rs`** — Reflects that this module handles all OpenAI-compatible APIs (OpenRouter, Minimax, Custom, LM Studio, Ollama), not just official OpenAI
- **Onboarding Simplified** — Removed ~300 lines of dead in-memory config construction from `apply_config()`; all config writes now use individual `write_key()`/`write_secret_key()` calls
- **keys.toml is Single Secret Source** — All API keys, bot tokens, and search keys are stored in `~/.opencrabs/keys.toml`. No more env vars or OS keyring for secrets. `config.toml` holds non-sensitive settings only

## [0.2.25] - 2026-02-21

### Added
- **Token Usage for MiniMax/OpenRouter** — Added `stream_options: {include_usage: true}` to streaming requests; extracts and logs token usage from final chunk
- **Shutdown Logo** — Shows ASCII logo with rolling goodbye message on terminal when exiting

### Fixed
- **Duplicate Messages** — Fixed duplicate assistant messages appearing when IntermediateText already added content
- **Tool Call Flow** — Tool calls now appear as separate messages after assistant text, flowing naturally between steps
- **Empty Content Rendering** — Fixed assistant messages showing empty during session (was showing correctly after restart)
- **Thinking Indicator** — Moved "OpenCrabs is thinking..." indicator to sticky position at bottom of chat (above input field), always visible to users

### Changed
- **Message Ordering** — Queued messages now appear at very bottom of conversation (after all assistant/tool messages), above input field
- **README** — Added GitHub stars call-to-action

## [0.2.24] - 2026-02-21

### Added
- **MiniMax Provider Support** — Added MiniMax as new LLM provider (OpenAI-compatible). Does not have /models endpoint, uses config_models for model list
- **Onboarding Wizard** — Full onboarding flow for first-time setup with provider selection
- **Model Selector** — Slash command `/models` to change provider and model with live fetching, search filter
- **Tool Call Expanded View** — Ctrl+O expands tool context with gray background; diff coloring (+ green, - red)
- **API Keys in keys.toml** — API keys now stored in separate `~/.opencrabs/keys.toml` (chmod 600)
- **STT/TTS Provider Config** — Added `providers.stt.groq` and `providers.tts.openai` config sections

### Fixed
- **MiniMax Tool Calls** — Fixed tool call parsing for MiniMax (empty arguments issue)
- **Context Compaction Crash** — Fixed orphaned tool_result crash after compaction
- **Onboarding Persistence** — Provider selection and settings now persist correctly
- **Model Selector Flow** — Multiple fixes for persistence, search, scrolling, Enter key behavior
- **Compaction Crash (400 — Orphaned tool_result)** — After any trim or compaction, a `user(tool_result)` message could be left at the front of history without its preceding `assistant(tool_use)`. The Anthropic API rejects this with a 400 error, crashing the next compaction attempt. Fixed at three layers: `trim_to_fit` and `trim_to_target` now call `drop_leading_orphan_tool_results()` after each removal; `compact_with_summary` advances `keep_start` past any leading orphaned tool_result messages; `compact_context` skips them before sending to the API as a safety net. Conversation continues normally after compaction with no tool call drops
- **Compaction Summary as Assistant Message** — Compaction summary was stored in a `details` field and hidden behind Ctrl+O. Now rendered as a real assistant chat message in the conversation flow. Tool calls that follow appear below it as normal tool groups with Ctrl+O expand/collapse
- **config.toml Model Priority over .env** — `ANTHROPIC_MAX_MODEL` env var was overwriting the model set in `config.toml`, reversing the intended priority. Now `config.toml` wins; `.env` is only a fallback when no model is configured in TOML
- **Stale Terminal on exec() Restart** — `/rebuild` hot-restart left stale rendered content from the previous process visible briefly. Terminal is now fully cleared immediately after the new process takes over

### Changed
- **Remove Qwen and Azure** — These providers are no longer supported
- **README Updated** — Added MiniMax documentation, keys.toml instructions

## [0.2.23] - 2026-02-20

### Added
- **session_search Tool** — Hybrid FTS5+vector search across all chat sessions (list/search operations)
- **History Paging** — Cap initial display at 200k tokens, Ctrl+O loads 100k more from DB
- **Onboarding Model Filter** — Type to search models, Esc clears filter

### Fixed
- **Onboard Centering** — Header/footer center independently, content block centers as uniform group
- **Onboard Scroll** — ProviderAuth tracks focused_line for proper scroll anchoring
- **Content Clipping** — Content no longer clips top border on overflow screens

### Changed
- **Compaction Display** — Now clears TUI display fully, shows summary as fresh start
- **Render history_marker** — Rendered as dim italic in chat view

## [0.2.22] - 2026-02-19

### Added
- **`/cd` Command** — Change working directory at runtime via slash command or agent NLP. Opens a directory picker (same UI as `@` file picker). Persists to `config.toml`. Agent can also call `config_manager` with `set_working_directory`
- **`slash_command` Tool** — Agent-callable tool to invoke any slash command programmatically: `/cd`, `/compact`, `/rebuild`, `/approve`, and all user-defined commands from `commands.toml`. Makes the agent aware of and able to trigger any slash command
- **Edit Diff Context** — Edit tool now includes a compact unified diff in its output. Renderer colors `+` lines green, `-` lines red, `@@` lines cyan — giving both user and agent clear visual context of changes

### Fixed
- **Stderr Bleeding into TUI** — Replaced all `unsafe` libc `dup2`/`/dev/null` hacks with `llama-cpp-2`'s proper `send_logs_to_tracing(LogOptions::default().with_logs_enabled(false))` API. Called once at engine init — kills all llama.cpp C-level stderr output permanently. Removed `libc` dependency entirely
- **Compaction Summary Never Visible** — System messages were rendered as a single `Span` on one `Line` — Ratatui clips at terminal width, so multi-paragraph summaries were silently swallowed. Fixed: newline-aware rendering with `⚡` yellow label. Compaction summary now goes into expandable `details` (Ctrl+O to read)
- **Tool Approval Disappearing** — Removed 4 `messages.retain()` calls that deleted approval messages immediately after denial, before the user could see or interact with them

### Changed
- **Install Instructions** — README now includes "Make It Available System-Wide" section with symlink/copy instructions
- **Brain Templates** — BOOT.md, TOOLS.md, AGENTS.md updated to document `/cd` and `config_manager` working directory control

## [0.2.21] - 2026-02-19

### Changed
- **Module Restructure** — Merged `src/llm/` (agent, provider, tools, tokenizer) into `src/brain/`. Brain is now the single intelligence layer — no split across two top-level modules
- **Channel Consolidation** — Moved `src/slack/`, `src/telegram/`, `src/whatsapp/`, `src/discord/`, and `src/voice/` into `src/channels/`. All messaging integrations + voice (STT/TTS) live under one module with feature-gated submodules
- **Ctrl+O Expands All** — Ctrl+O now toggles expand/collapse on ALL tool call groups in the session, not just the most recent one

### Fixed
- **Tool Approval Not Rendering** — Fixed approval prompts not appearing in long-context sessions when user had scrolled up. `auto_scroll` is now reset to `true` when an approval arrives, ensuring the viewport scrolls to show it
- **Tool Call Details Move** — Fixed `use of moved value` for tool call details field in ToolCallCompleted handler

## [0.2.20] - 2026-02-19

### Added
- **`/whisper` Command** — One-command setup for system-wide voice-to-text. Auto-downloads WhisperCrabs binary, launches floating mic button. Speak from any app, transcription auto-copies to clipboard
- **`SystemMessage` Event** — New TUI event variant for async tasks to push messages into chat

### Fixed
- **Embedding Stderr Bleed** — Suppressed llama.cpp C-level stderr during `embed_document()` and `embed_batch_with_progress()`, not just model load. Fixes garbled TUI output during memory indexing
- **Slash Autocomplete Dedup** — User-defined commands that shadow built-in names no longer show twice in autocomplete dropdown
- **Slash Autocomplete Width** — Dropdown auto-sizes to fit content instead of hardcoded 40 chars. Added inner padding on all sides
- **Help Screen** — Added missing `/rebuild` and `/whisper` to `/help` slash commands list
- **Cleartext Logging (CodeQL)** — Removed all `println!` calls from provider factory that wrote to stdout (corrupts TUI). Kept `tracing::info!` for structured logging
- **Stray Print Statements** — Removed debug `println!` from wacore encoder, replaced `eprintln!` in onboarding tests with silent returns

### Changed
- **Docker Files Relocated** — Moved `docker/` from project root to `src/docker/`, updated all references in README and compose.yml
- **Clippy Clean** — Fixed collapsible_if warnings in onboarding and app, `map_or` → `is_some_and`

## [0.2.19] - 2026-02-18

### Changed
- **Cleaner Chat UI** — Replaced role labels with visual indicators: `❯` for user messages, `●` for assistant messages. User messages get subtle dark background for visual separation. Removed horizontal dividers and input box title for a cleaner look
- **Alt+Arrow Word Navigation** — Added `Alt+Left` / `Alt+Right` as alternatives to `Ctrl+Left` / `Ctrl+Right` for word jumping (macOS compatibility)
- **Branding** — Thinking/streaming indicators now show `🦀 OpenCrabs` instead of model name

## [0.2.18] - 2026-02-18

### Added
- **OpenRouter Provider** -- First-class OpenRouter support in onboarding wizard. One API key, 400+ models including free and stealth models (DeepSeek, Llama, Mistral, Qwen, Gemma, and more). Live model list fetched from `openrouter.ai/api/v1/models`
- **Live Model Fetching** -- `/models` command and onboarding wizard now fetch available models live from provider APIs (Anthropic, OpenAI, OpenRouter). When a new model drops, it shows up immediately — no binary update needed. Falls back to hardcoded list if offline
- **`Provider::fetch_models()` Trait Method** -- All providers implement async model fetching with graceful fallback to static lists

### Changed
- **Onboarding Wizard** -- Provider step 2 now shows live model list fetched from API after entering key. Shows "(fetching...)" while loading. OpenRouter added as 5th provider option
- **Removed `cargo publish` from CI** -- Release workflow no longer attempts crates.io publish (was never configured, caused false failures)

## [0.2.17] - 2026-02-18

### Changed
- **QMD Vector Search + RRF** -- qmd's `EmbeddingEngine` (embeddinggemma-300M, 768-dim GGUF) wired up alongside FTS5 with Reciprocal Rank Fusion. Local model, no API key, zero cost, works offline. Auto-downloads ~300MB on first use, falls back to FTS-only when unavailable
- **Batch Embedding Backfill** -- On startup reindex, documents missing embeddings are batch-embedded via qmd. Single-file indexes (post-compaction) embed immediately when engine is warm
- **Discord Voice (STT + TTS)** -- Discord bot now transcribes audio attachments via Groq Whisper and replies with synthesized voice (OpenAI TTS) when enabled
- **WhatsApp Voice (STT)** -- WhatsApp bot now transcribes voice notes via Groq Whisper. Text replies only (media upload for TTS pending)
- **CI Release Workflow** -- Fixed nightly toolchain for all build targets, added ARM64 cross-linker config
- **AVX CPU Guard** -- Embedding engine checks for AVX support at init; gracefully falls back to FTS-only on older CPUs
- **Stderr Suppression** -- llama.cpp C-level stderr output redirected to /dev/null during model load to prevent TUI corruption

## [0.2.16] - 2026-02-18

### Changed
- **QMD Crate for Memory Search** -- Replaced homebrew FTS5 implementation with the `qmd` crate (BM25 search, SHA-256 content hashing, collection management). Upgraded `sqlx` to 0.9 (git main) to resolve `libsqlite3-sys` linking conflict
- **Brain Files Indexed** -- Memory search now indexes workspace brain files (`SOUL.md`, `IDENTITY.md`, `MEMORY.md`, etc.) alongside daily compaction logs for richer search context
- **Dynamic Welcome Messages** -- All channel connect tools (Telegram, Discord, Slack, WhatsApp) now instruct the agent to craft a creative, personality-driven welcome message on successful connection instead of hardcoded greetings
- **WhatsApp Welcome Removed** -- Replaced hardcoded WhatsApp welcome spawn with agent-generated message via `whatsapp_send` tool
- **Patches Relocated** -- Moved `wacore-binary` patch from `patches/` to `src/patches/`, stripped benchmarks and registry metadata

### Added
- **Discord `channel_id` Parameter** -- Optional `channel_id` input on `discord_connect` so the bot can send welcome messages immediately after connection
- **Slack `channel_id` Parameter** -- Optional `channel_id` input on `slack_connect` for the same purpose
- **Telegram Owner Chat ID** -- `telegram_connect` now sets the owner chat ID from the first allowed user at connection time
- **QMD Memory Benchmarks** -- Criterion benchmarks for qmd store operations: index file (203µs), hash skip (18µs), FTS5 search (381µs–2.4ms), bulk reindex 50 files (11.3ms), store open (1.7ms)

## [0.2.15] - 2026-02-17

### Changed
- **Built-in FTS5 Memory Search** -- Replaced external QMD CLI dependency with native SQLite FTS5 full-text search. Zero new dependencies (uses existing `sqlx`), always-on memory search with no separate binary to install. BM25-ranked results with porter stemming and snippet extraction
- **Memory Search Always Available** -- Sidebar now shows "Memory search" with a permanent green dot instead of conditional "QMD search" that required an external binary
- **Targeted Index After Compaction** -- After context compaction, only the updated daily memory file is indexed (via `index_file`) instead of triggering a full `qmd update` subprocess
- **Startup Background Reindex** -- On launch, existing memory files are indexed in the background so `memory_search` is immediately useful for returning users

### Added
- **FTS5 Memory Module** -- New async API: `get_pool()` (lazy singleton), `search()` (BM25 MATCH), `index_file()` (single file, hash-skip), `reindex()` (full walk + prune deleted). Schema: `memory_docs` content table + `memory_fts` FTS5 virtual table with sync triggers
- **Memory Search Tests** -- Unit tests for FTS5 init, index, search, hash-based skip, and content update re-indexing
- **Performance Benchmarks in README** -- Real release-build numbers: ~0.4ms/query, ~0.3ms/file index, 15ms full reindex of 50 files
- **Resource Footprint Table in README** -- Branded stats table with binary size, RAM, storage, and FTS5 search latency

### Removed
- **QMD CLI Dependency** -- Removed all `Command::new("qmd")` subprocess calls: `is_qmd_available()`, `ensure_collection()`, `search()` (sync), `reindex_background()`

## [0.2.14] - 2026-02-17

### Added
- **Discord Integration** -- Full Discord bot with message forwarding, per-user session routing, image attachment support, proactive messaging via `discord_send` tool, and dynamic connection via `discord_connect` tool
- **Slack Integration** -- Full Slack bot via Socket Mode (no public endpoint needed) with message forwarding, session sharing, proactive messaging via `slack_send` tool, and dynamic connection via `slack_connect` tool
- **Secure Bot Messaging: `respond_to` Mode** -- New `respond_to` config field for all platforms: `"mention"` (default, most secure), `"all"` (old behavior), or `"dm_only"`. DMs always get a response regardless of mode
- **Channel Allowlists** -- New `allowed_channels` config field restricts which group channels bots are active in. Empty = all channels. DMs always pass
- **Bot @Mention Detection** -- Discord checks `msg.mentions` for bot user ID, Telegram checks `@bot_username` or reply-to-bot, Slack checks `<@BOT_USER_ID>` in text. Bot mention text is stripped before sending to agent
- **Bot Identity Caching** -- Discord stores bot user ID from `ready` event, Telegram fetches `@username` via `get_me()` at startup, Slack fetches bot user ID via `auth.test` at startup
- **Troubleshooting Section in README** -- Documents the known session corruption issue where agent hallucinates tool calls, with workaround (start new session)

### Fixed
- **Pending Tool Approvals Hanging Agent** -- Approval callbacks were never resolved on cancel, error, supersede, or agent completion, causing the agent to hang indefinitely. All code paths now properly deny pending approvals with `response_tx.send()`
- **Stale Approval Cleanup** -- Cancel (Escape), error handler, new request, and agent completion all now send deny responses before marking approvals as denied
- **Rustls Crypto Provider for Slack** -- Install `ring` crypto provider at startup before any TLS connections, fixing Slack Socket Mode panics

### Changed
- **Proactive Message Branding Removed** -- `discord_send`, `slack_send`, `telegram_send` tools no longer prepend `MSG_HEADER` to outgoing messages
- **Agent Logging** -- Improved iteration logging: shows "completed after N tool iterations" or "responded with text only"
- **Auto-Approve Feedback** -- Selecting "Allow Always" now shows a system message confirming auto-approve is enabled for the session

## [0.2.13] - 2026-02-17

### Added
- **Proactive WhatsApp Messaging** -- New `whatsapp_send` agent tool lets the agent send messages to the user (or any allowed phone) at any time, not just in reply to incoming messages
- **WhatsApp Welcome Message** -- On successful QR pairing, the agent sends a fun random crab greeting to the owner's WhatsApp automatically
- **WhatsApp Message Branding** -- All outgoing WhatsApp messages are prefixed with `🦀 *OpenCrabs*` header so users can distinguish agent replies from their own messages
- **WhatsApp `device_sent_message` Unwrapping** -- Recursive `unwrap_message()` handles WhatsApp's nested message wrappers (`device_sent_message`, `ephemeral_message`, `view_once_message`, `document_with_caption_message`) to extract actual text content from linked-device messages
- **Fun Startup/Shutdown Messages** -- Random crab-themed greetings on launch and farewell messages on exit (10 variants each)

### Fixed
- **WhatsApp Self-Chat Messages Ignored** -- Messages from the user's own phone were dropped because `is_from_me: true`; now only skips messages with the agent's `MSG_HEADER` prefix to prevent echo loops while accepting user messages from linked devices
- **WhatsApp Phone Format Mismatch** -- Allowlist comparison failed because config stored `+351...` but JID user part was `351...`; `sender_phone()` now strips `@s.whatsapp.net` suffix, allowlist check strips `+` prefix
- **Model Name Missing from Thinking Spinner** -- "is thinking" showed without model name because `session.model` could be `Some("")`; added `.filter(|m| !m.is_empty())` fallback to `default_model_name`
- **WhatsApp SQLx Store Device Serialization** -- Device state now serialized via `rmp-serde` (MessagePack) instead of broken `bincode`; added `rmp-serde` dependency under whatsapp feature

### Changed
- **`wacore-binary` Direct Dependency** -- Added as direct optional dependency for `Jid` type access (needed by `whatsapp_send` and `whatsapp_connect` tools for JID parsing)

### Removed
- **`/model` Slash Command** -- Removed redundant `/model` command; `/models` already provides model switching with selected-model display

## [0.2.12] - 2026-02-17

### Added
- **WhatsApp Integration** -- Chat with your agent via WhatsApp Web. Connect dynamically at runtime ("connect my WhatsApp") or from the onboarding wizard. QR code pairing displayed in terminal using Unicode block characters, session persists across restarts via SQLite
- **WhatsApp Image Support** -- Send images to the agent via WhatsApp; they're downloaded, base64-encoded, and forwarded to the AI backend for multimodal analysis
- **WhatsApp Connect Tool** -- New `whatsapp_connect` agent tool: generates QR code, waits for scan (2 min timeout), spawns persistent listener, updates config automatically
- **Onboarding: Messaging Setup** -- New step in both QuickStart and Advanced onboarding modes to enable Telegram and/or WhatsApp channels right after provider auth
- **Channel Factory** -- Shared `ChannelFactory` for creating channel agent services at runtime, used by both static startup and dynamic connection tools
- **Custom SQLx WhatsApp Store** -- `wacore::store::Backend` implementation using the project's existing `sqlx` SQLite driver, avoiding the `libsqlite3-sys` version conflict with `whatsapp-rust-sqlite-storage` (Diesel-based). 15 tables, 33 trait methods, full test coverage
- **Nightly Rust Requirement** -- `wacore-binary` requires `#![feature(portable_simd)]`; added `rust-toolchain.toml` pinning to nightly. Local patch for `wacore-binary` fixes `std::simd::Select` API breakage on latest nightly

### Changed
- **Version Numbering** -- Corrected from 0.2.2 to 0.2.11 (following 0.2.1), this release is 0.2.12

## [0.2.11] - 2026-02-16

### Fixed
- **Context Token Display** -- TUI context indicator showed inflated values (e.g. `640K/200K`) because `input_tokens` was accumulated across all tool-loop iterations instead of using the last API call's actual context size; now `AgentResponse.context_tokens` tracks the last iteration's `input_tokens` for accurate display while `usage` still accumulates for correct billing
- **Per-Message Token Count** -- `DisplayMessage.token_count` now shows only output tokens (the actual generated content) instead of the inflated `input + output` sum which double-counted shared context
- **Clippy Warning** -- Fixed `redundant_closure` warning in `trim_messages_to_budget`

### Changed
- **Compaction Threshold** -- Lowered auto-compaction trigger from 80% to 70% of context window for earlier, safer compaction with more headroom
- **Token Counting** -- `trim_messages_to_budget` now uses tiktoken (`cl100k_base`) instead of `chars/3` heuristic; history budget targets 60% of context window (was 70%) to leave more room for tool results

### Added
- **2 New Tests** -- `test_context_tokens_is_last_iteration_not_accumulated` and `test_context_tokens_equals_input_tokens_without_tools` verifying correct context vs billing token separation (450 total)

### Removed
- **Dead Code** -- Removed unused `format_token_count` function and its 5 tests from `render.rs`

## [0.2.1] - 2026-02-16

### Added
- **Config Management Tool** -- New `config_manager` agent tool with 6 operations: `read_config`, `write_config`, `read_commands`, `add_command`, `remove_command`, `reload`; the agent can now read/write `config.toml` and `commands.toml` at runtime
- **Commands TOML Migration** -- User-defined slash commands now stored in `commands.toml` (`[[commands]]` array) instead of `commands.json`; existing `commands.json` files auto-migrate on first load
- **Settings TUI Screen** -- Press `S` for a real Settings screen showing: current provider/model, approval policy, user commands summary, QMD memory search status, and file paths (config, brain, working directory)
- **Approval Policy Persistence** -- `/approve` command now saves the selected policy to `[agent].approval_policy` in `config.toml`; policy is restored on startup instead of always defaulting to "ask"
- **AgentConfig Section** -- New `[agent]` config section with `approval_policy` ("ask" / "auto-session" / "auto-always") and `max_concurrent` (default: 4) fields
- **Live Config Reload** -- `Config::reload()` method and `TuiEvent::ConfigReloaded` event for refreshing cached config values after tool writes
- **Config Write Helper** -- `Config::write_key(section, key, value)` safely merges key-value pairs into `config.toml` without overwriting unrelated sections
- **Command Management Helpers** -- `CommandLoader::add_command()` and `CommandLoader::remove_command()` for atomic command CRUD
- **20 New Tests** -- 14 onboarding tests (key handlers, mode select, provider navigation, API key input, field flow, validation, model selection, workspace/health/brain defaults) + 6 config tests (AgentConfig defaults, TOML parsing, write_key merge, save round-trip) -- 443 total

### Changed
- **config.toml.example** -- Added `[agent]` and `[voice]` example sections with documentation
- **Commands Auto-Reload** -- After `ConfigReloaded` event, user commands are refreshed from `commands.toml`

## [0.2.0] - 2026-02-15

### Added
- **3-Tier Memory System** -- OpenCrabs now has a layered memory architecture: (1) **Brain MEMORY.md** -- user-curated durable memory loaded into system brain every turn, (2) **Daily Memory Logs** -- auto-compaction summaries saved to `~/.opencrabs/memory/YYYY-MM-DD.md` with multiple compactions per day stacking in the same file, (3) **Memory Search** -- `memory_search` tool backed by QMD for semantic search across all past daily logs
- **Memory Search Tool** -- New `memory_search` agent tool searches past conversation logs via QMD (`qmd query --json`); gracefully degrades if QMD is not installed, returning a hint to use `read_file` on daily logs directly
- **Compaction Summary Display** -- Auto-compaction at 80% context now shows the full summary in chat as a system message instead of running silently; users see exactly what the agent remembered
- **Scroll While Streaming** -- Users can scroll up during streaming without being yanked back to the bottom; `auto_scroll` flag disables on user scroll, re-enables when scrolled back to bottom or on message send
- **QMD Auto-Index** -- After each compaction, `qmd update` is triggered in the background to keep the memory search index current
- **Memory Module** -- New `src/memory/mod.rs` module with QMD wrapper: availability check, collection management, search, and background re-indexing
- **Path Consolidation** -- All data now lives under `~/.opencrabs/` (config, database, brain, memory, history, logs)
- **Context Budget Awareness** -- Tool definition overhead (~500 tokens per tool) now factored into context usage calculation, preventing "prompt too long" errors

### Changed
- **Compaction Target** -- Compaction summaries now write to daily logs (`~/.opencrabs/memory/YYYY-MM-DD.md`) instead of appending to brain workspace `MEMORY.md`; brain `MEMORY.md` remains user-curated and untouched by auto-compaction
- **Local Timestamps** -- Daily memory logs use `chrono::Local` instead of UTC for human-readable timestamps

## [0.1.9] - 2026-02-15

### Added
- **Cursor Navigation** -- Full cursor movement in input: Left/Right arrows, Ctrl+Left/Right word jump, Home/End, Delete key, Backspace at cursor position, word delete (Alt/Ctrl+Backspace), character and paste insertion at cursor position, cursor renders at correct position
- **Input History Persistence** -- Command history saved to `~/.config/opencrabs/history.txt` (one line per entry), loaded on startup, appended on each send, capped at 500 entries, survives restarts
- **Real-time Streaming** -- Added `stream_complete()` method that streams text chunks from the provider via `StreamingChunk` progress events, replacing the old blocking `provider.complete()` call
- **Streaming Spinner** -- Animated spinner shows `"claude-opus is responding..."` with streamed text below; `"thinking..."` spinner shows only before streaming begins
- **Inline Plan Approval** -- Plan approval now renders as an interactive inline selector with arrow keys (Approve / Reject / Request Changes / View Plan) instead of plain text Ctrl key instructions
- **Telegram Photo Support** -- Incoming photos download at largest resolution, saved to temp file, forwarded as `<<IMG:path>>` caption; image documents detected via `image/*` MIME type; temp files cleaned up after 30 seconds
- **Error Message Rendering** -- `app.error_message` is now rendered in the chat UI (was previously set but never displayed)
- **Default Model Name** -- New sessions show the actual provider model name (e.g. `claude-opus-4-6`) as placeholder instead of generic "AI"
- **Debug Logging** -- `DEBUG_LOGS_LOCATION` env var sets custom log directory; `--debug` CLI flag enables debug mode
- **8 New Tests** -- `stream_complete_text_only`, `stream_complete_with_tool_use`, `streaming_chunks_emitted`, `markdown_to_telegram_html_*`, `escape_html`, `img_marker_format` (412 total)

### Fixed
- **SSE Parser Cross-Chunk Buffering** -- TCP chunks splitting JSON events mid-string caused `EOF while parsing a string` errors and silent response drops; parser now buffers partial lines across chunks with `Arc<Mutex<String>>`, only parsing complete newline-terminated lines
- **Stale Approval Cleanup** -- Old `Pending` approval messages permanently hid streaming responses; now cleared on new message send, new approval request, and response completion
- **Approval Dialog Reset** -- `approval_auto_always` reset on session create/load; inline "Always" now sets `approval_auto_session` (resets on session change) instead of `approval_auto_always`
- **Brain File Path** -- Brain prompt builder used wrong path for workspace files
- **Abort During Streaming** -- Cancel token properly wired through streaming flow for Escape×2 abort

### Changed
- **README** -- Expanded self-sustaining section with `/rebuild` command, `SelfUpdater` module, session persistence, brain live-editing documentation

## [0.1.8] - 2026-02-15

### Added
- **Image Input Support** -- Paste image paths or URLs into the input; auto-detected and attached as vision content blocks for multimodal models (handles paths with spaces)
- **Attachment Indicator** -- Attached images show as `[IMG1:filename.png]` in the input box title bar; user messages display `[IMG: filename.png]`
- **Tool Context Persistence** -- Tool call groups are now saved to the database and reconstructed on session reload; no more vanishing tool history
- **Intermediate Text Display** -- Agent text between tool call batches now appears interleaved in the chat, matching Claude Code's behavior

### Fixed
- **Tool Descriptions Showing "?"** -- Approval dialog showed "Edit ?" instead of file paths; fixed parameter key mismatches (`path` not `file_path`, `operation` not `action`)
- **Raw Tool JSON in Chat** -- `[Tool: read_file]{json}` was dumped into assistant messages; now only text blocks are displayed, tool calls shown via the tool group UI
- **Loop Detection Wrong Keys** -- Tool loop detection used `file_path` for read/write/edit; fixed to `path`
- **Telegram Text+Voice Order** -- Text reply now always sent first, voice note follows (was skipping text on TTS success)

### Changed
- **base64 dependency** -- Re-added `base64 = "0.22.1"` for image encoding (was removed in dep cleanup but now needed)

## [0.1.7] - 2026-02-14

### Added
- **Voice Integration (STT)** -- Incoming Telegram voice notes are transcribed via Groq Whisper (`whisper-large-v3-turbo`) and processed as text by the agent
- **Voice Integration (TTS)** -- Agent replies to voice notes with audio via OpenAI TTS (`gpt-4o-mini-tts`, `ash` voice); falls back to text if TTS is disabled or fails
- **Onboarding: Telegram Setup** -- New wizard step with BotFather instructions, bot token input (masked), and user ID guidance; auto-detects existing env/keyring values
- **Onboarding: Voice Setup** -- New wizard step for Groq API key (STT) and TTS toggle with `ash` voice label; auto-detects `GROQ_API_KEY` from environment
- **Sessions Dialog: Context Info** -- `/sessions` now shows token count per session (`12.5K tok`, `2.1M tok`) and live context window percentage for the current session with color coding (green/yellow/red)
- **Tool Descriptions in Approval** -- Approval dialog now shows actual file paths and parameters (e.g. "Edit /src/tui/render.rs") instead of raw tool names ("edit_file")
- **Shared Telegram Session** -- Owner's Telegram messages now use the same session as the TUI terminal; no more separate sessions that could pick the wrong model

### Changed
- **Provider Priority** -- Factory order changed to Qwen → Anthropic → OpenAI; Anthropic is now always preferred over OpenAI for text generation
- **OPENAI_API_KEY Isolation** -- `OPENAI_API_KEY` no longer auto-creates an OpenAI text provider; it is only used for TTS (`gpt-4o-mini-tts`), never for text generation unless explicitly configured
- **Async Terminal Events** -- Replaced blocking `crossterm::event::poll()` with async `EventStream` + `tokio::select!` to prevent TUI freezes during I/O-heavy operations

### Fixed
- **Model Contamination** -- `OPENAI_API_KEY` in `.env` was causing GPT-4 to be used for text instead of Anthropic Claude; multi-layered fix across factory, env overrides, and TTS key sourcing
- **Navigation Slowdown** -- TUI became sluggish after losing terminal focus due to synchronous 100ms blocking poll in async context
- **Context Showing 0%** -- Loading an existing session showed 0% context; now estimates tokens from message content until real API usage arrives
- **Approval Spam** -- "edit_file -- approved" messages no longer clutter the chat; approved tool calls are silently removed since the tool group already shows execution progress
- **6 Clippy Warnings** -- Fixed collapsible_if (5) and manual_find (1) across onboarding and telegram modules

## [0.1.6] - 2026-02-14

### Added
- **Telegram Bot Integration** -- Chat with OpenCrabs via Telegram alongside the TUI; bot runs as a background task with full tool access (file ops, search, bash, etc.)
- **Telegram Allowlist** -- Only allowlisted Telegram user IDs can interact; `/start` command shows your ID for easy setup
- **Telegram Markdown→HTML** -- Agent responses are formatted as Telegram-safe HTML with code blocks, inline code, bold, and italic support
- **Telegram Message Splitting** -- Long responses automatically split at 4096-char Telegram limit, breaking at newlines
- **Grouped Tool Calls** -- Multiple tool calls in a single agent turn now display as a collapsible group with tree lines (├─ └─) instead of individual messages
- **Claude Code-Style Approval** -- Tool approval dialog rewritten as vertical selector with `❯ Yes / Always / No` matching Claude Code's UX
- **Emergency Compaction Retry** -- If the LLM provider returns "prompt too long", automatically compact context and retry instead of failing

### Changed
- **Token Estimation** -- Changed from `chars/4` to `chars/3` for more conservative estimation, preventing context overflows that the old estimate missed
- **Compaction Accounts for Tools** -- Auto-compaction threshold now reserves ~500 tokens per registered tool for schema overhead, preventing "prompt too long" errors
- **Telegram Feature Default** -- `telegram` feature now included in default features (no need for `--features telegram`)

### Fixed
- **Context % Showing 2369%** -- `context_usage_percent()` was summing all historical token counts; now uses only the latest response's `input_tokens`
- **TUI Lag After First Request** -- `active_tool_group` wasn't cleaned up on error/abort paths, causing UI to hang
- **Telegram Bot No Response** -- Bot was calling `send_message` (no tools) instead of `send_message_with_tools`; also needed `auto_approve_tools: true` since there's no TUI for approval

## [0.1.5] - 2026-02-14

### Added
- **Context Usage Indicator** -- Input box shows live `Context: X%` with color coding: green (<60%), yellow (60-80%), red (>80%) so you always know how close you are to the context limit
- **Auto-Compaction** -- When context usage exceeds 80%, automatically sends conversation to the LLM for a structured breakdown summary (Current Task, Key Decisions, Files Modified, Current State, Important Context, Errors & Solutions), saves to MEMORY.md, and trims context keeping the last 8 messages + summary for seamless continuation
- **`/compact` Command** -- Manually trigger context compaction at any time via slash command
- **Brave Search Tool** -- Real-time web search via Brave Search API (set `BRAVE_API_KEY`); great if you already have a Brave API key or want a free-tier option
- **EXA Search Tool** -- Neural-powered web search via EXA AI; works out of the box via free hosted MCP endpoint (no API key needed). Set `EXA_API_KEY` for direct API access with higher rate limits

### Changed
- **EXA Always Available** -- EXA search registers unconditionally via free MCP endpoint; Brave still requires `BRAVE_API_KEY`

## [0.1.4] - 2026-02-14

### Added
- **Inline Tool Progress** -- Tool executions now show inline in chat with human-readable descriptions (e.g. "Read src/main.rs", "bash: cargo check", "Edited src/app.rs") instead of invisible spinner
- **Expand/Collapse Tool Details** -- Press Ctrl+O to expand or collapse tool output details on completion messages, inspired by Claude Code's UX
- **Abort Processing** -- Press Escape twice within 3 seconds to cancel an in-progress agent request via CancellationToken
- **Active Input During Processing** -- Input box stays active with cursor visible while agent is processing; border remains steel blue
- **Processing Guard** -- Prevents sending a second message while one is already processing; shows "Please wait or press Esc x2 to abort"
- **Progress Callback System** -- New `ProgressCallback` / `ProgressEvent` architecture emitting `Thinking`, `ToolStarted`, and `ToolCompleted` events from agent service to TUI
- **LLM-Controlled Bash Timeout** -- Bash tool now accepts `timeout_secs` from the LLM (capped at 600s), default raised from 30s to 120s

### Changed
- **Silent Auto-Approved Tools** -- Auto-approved tool calls no longer spam the chat; only completion descriptions shown
- **Approval Never Times Out** -- Tool approval requests wait indefinitely until the user acts (no more 5-minute timeout)
- **Approval UI De-Emojified** -- All emojis removed from approval rendering; clean text-only UI
- **Yolo Mode Always Visible** -- All three approval tiers (Allow once, Allow all session, Yolo mode) always visible with color-coding (green/yellow/red) in inline approval

### Fixed
- **Race Condition on Double Send** -- Added `is_processing` guard in `send_message()` preventing overlapping agent requests

## [0.1.3] - 2026-02-14

### Added
- **Inline Tool Approval** — Tool permission requests now render inline in chat instead of a blocking overlay dialog, with three options: Allow once, Allow all for this task, Allow all moving forward
- **`/approve` Command** — Resets tool approval policy back to "always ask"
- **Word Deletion** — Ctrl+Backspace and Alt+Backspace delete the last word in input
- **Scroll Support** — Arrow keys and Page Up/Down now scroll Help, Sessions, and Settings screens
- **Tool Approval Docs** — README section documenting inline approval keybindings and options

### Changed
- **Ctrl+C Behavior** — First press clears input, second press within 3 seconds quits (was immediate quit)
- **Help Screen** — Redesigned as 2-column layout filling full terminal width instead of narrow single column
- **Status Bar Removed** — Bottom status bar eliminated for cleaner UI; mode info shown in header only
- **Ctrl+H Removed** — Help shortcut removed (use `/help` instead); fixes Ctrl+Backspace conflict where terminals send Ctrl+H for Ctrl+Backspace

### Removed
- **MCP Module** — Deleted empty placeholder `src/mcp/` directory (unused stubs, zero functionality)
- **Overlay Approval Dialog** — Replaced by inline approval in chat
- **Bottom Status Bar** — Removed entirely for more screen space

## [0.1.2] - 2026-02-14

### Added
- **Onboarding Wizard** — 8-step wizard with QuickStart/Advanced modes for first-time setup
- **AI Brain Personalization** — Generates all 6 workspace brain files (SOUL, IDENTITY, USER, AGENTS, TOOLS, MEMORY) from user input during onboarding
- **Session Management** — `/sessions` command, rename sessions (R), delete sessions (D) from session list
- **Mouse Scroll** — Mouse wheel scrolls chat history
- **Dynamic Input Height** — Input area grows with content, 1-line default
- **Screenshots** — Added UI screenshots to README (splash, onboarding, chat)

### Changed
- **Unified Anthropic Provider** — Auto-detects OAuth tokens vs API keys from env/keyring
- **Pre-wrapped Chat Lines** — Consistent left padding for all chat messages
- **Updated Model List** — Added `claude-opus-4-6`, `gpt-5.1-codex-mini`, `gemini-3-flash-preview`, `qwen3-coder-next`
- **Cleaner UI** — Removed emojis, reordered status bar
- **README** — Added screenshots, updated structure

[0.1.2]: https://github.com/adolfousier/opencrab/releases/tag/v0.1.2

## [0.1.1] - 2026-02-14

### Added
- **Dynamic Brain System** — Replace hardcoded system prompt with brain loader that reads workspace MD files (SOUL, IDENTITY, USER, AGENTS, TOOLS, MEMORY) per-turn from `~/opencrab/brain/workspace/`
- **CommandLoader** — User-defined slash commands via `commands.json`, auto-reloaded after each agent response
- **SelfUpdater** — Build/test/restart via Unix `exec()` for hot self-update (`/rebuild` command)
- **RestartPending Mode** — Confirmation dialog in TUI after successful rebuild
- **Onboarding Docs** — Scaffolding for onboarding documentation

### Changed
- **system_prompt → system_brain** — Renamed across entire codebase to reflect dynamic brain architecture
- **`/help` Fixed** — Opens Help dialog instead of pushing text message into chat

[0.1.1]: https://github.com/adolfousier/opencrab/releases/tag/v0.1.1

## [0.1.0] - 2026-02-14

### Added
- **Anthropic OAuth Support** — Claude Max / setup-token authentication via `ANTHROPIC_MAX_SETUP_TOKEN` with automatic `sk-ant-oat` prefix detection, `Authorization: Bearer` header, and `anthropic-beta: oauth-2025-04-20` header
- **Claude 4.x Models** — Support for `claude-opus-4-6`, `claude-sonnet-4-5-20250929`, `claude-haiku-4-5-20251001` with updated pricing and context windows
- **`.env` Auto-Loading** — `dotenvy` integration loads `.env` at startup automatically
- **CHANGELOG.md** — Project changelog following Keep a Changelog format
- **New Branding** — OpenCrab ASCII art, "Shell Yeah! AI Orchestration at Rust Speed." tagline, crab icon throughout

### Changed
- **Rust Edition 2024** — Upgraded from edition 2021 to 2024
- **All Dependencies Updated** — Every crate bumped to latest stable (ratatui 0.30, crossterm 0.29, pulldown-cmark 0.13, rand 0.9, dashmap 6.1, notify 8.2, git2 0.20, zip 6.0, tree-sitter 0.25, thiserror 2.0, and more)
- **Rebranded** — "OpenCrab AI Assistant" renamed to "OpenCrab AI Orchestration Agent" across all source files, splash screen, TUI header, system prompt, and documentation
- **Enter to Send** — Changed message submission from Ctrl+Enter (broken in many terminals) to plain Enter; Alt+Enter / Shift+Enter inserts newline for multi-line input
- **Escape Double-Press** — Escape now requires double-press within 3 seconds to clear input, preventing accidental loss of typed messages
- **TUI Header Model Display** — Header now shows the provider's default model immediately instead of "unknown" until first response
- **Splash Screen** — Updated with OpenCrab ASCII art, new tagline, and author attribution
- **Default Max Tokens** — Increased from 4096 to 16384 for modern Claude models
- **Default Model** — Changed from `claude-3-5-sonnet-20240620` to `claude-sonnet-4-5-20250929`
- **README.md** — Complete rewrite: badges, table of contents, OAuth documentation, updated providers/models, concise structure (764 lines vs 3,497)
- **Project Structure** — Moved `tests/`, `migrations/`, `benches/`, `docs/` inside `src/` and updated all references

### Fixed
- **pulldown-cmark 0.13 API** — `Tag::Heading` tuple to struct variant, `Event::End` wraps `TagEnd`, `Tag::BlockQuote` takes argument
- **ratatui 0.29+** — `f.size()` replaced with `f.area()`, `Backend::Error` bounds added (`Send + Sync + 'static`)
- **rand 0.9** — `thread_rng()` replaced with `rng()`, `gen_range()` replaced with `random_range()`
- **Edition 2024 Safety** — Removed unsafe `std::env::set_var`/`remove_var` from tests, replaced with TOML config parsing

### Removed
- Outdated "Claude Max OAuth is NOT supported" disclaimer (it now is)
- Sprint history and "coming soon" filler from README
- Old "Crusty" branding and attribution

[0.2.39]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.39
[0.2.38]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.38
[0.2.37]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.37
[0.2.36]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.36
[0.2.35]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.35
[0.2.34]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.34
[0.2.33]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.33
[0.2.32]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.32
[0.2.31]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.31
[0.2.30]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.30
[0.2.29]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.29
[0.2.28]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.28
[0.2.27]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.27
[0.2.26]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.26
[0.2.25]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.25
[0.2.24]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.24
[0.2.23]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.23
[0.2.22]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.22
[0.2.21]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.21
[0.2.20]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.20
[0.2.19]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.19
[0.2.18]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.18
[0.2.17]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.17
[0.2.16]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.16
[0.2.15]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.15
[0.2.14]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.14
[0.2.13]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.13
[0.2.12]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.12
[0.2.11]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.11
[0.2.1]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.1
[0.2.0]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.0
[0.1.9]: https://github.com/adolfousier/opencrabs/releases/tag/v0.1.9
[0.1.8]: https://github.com/adolfousier/opencrabs/releases/tag/v0.1.8
[0.1.7]: https://github.com/adolfousier/opencrabs/releases/tag/v0.1.7
[0.1.6]: https://github.com/adolfousier/opencrabs/releases/tag/v0.1.6
[0.1.5]: https://github.com/adolfousier/opencrabs/releases/tag/v0.1.5
[0.1.4]: https://github.com/adolfousier/opencrabs/releases/tag/v0.1.4
[0.1.3]: https://github.com/adolfousier/opencrabs/releases/tag/v0.1.3
[0.1.2]: https://github.com/adolfousier/opencrabs/releases/tag/v0.1.2
[0.1.1]: https://github.com/adolfousier/opencrabs/releases/tag/v0.1.1
[0.1.0]: https://github.com/adolfousier/opencrabs/releases/tag/v0.1.0
