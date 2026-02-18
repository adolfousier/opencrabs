# Changelog

All notable changes to OpenCrab will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

[0.2.13]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.13
[0.2.12]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.12
[0.2.1]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.1
[0.2.0]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.0
[0.1.9]: https://github.com/adolfousier/opencrabs/releases/tag/v0.1.9
[0.1.8]: https://github.com/adolfousier/opencrabs/releases/tag/v0.1.8
[0.1.7]: https://github.com/adolfousier/opencrabs/releases/tag/v0.1.7
[0.1.6]: https://github.com/adolfousier/opencrabs/releases/tag/v0.1.6
[0.1.5]: https://github.com/adolfousier/opencrabs/releases/tag/v0.1.5
[0.1.4]: https://github.com/adolfousier/opencrabs/releases/tag/v0.1.4
[0.1.3]: https://github.com/adolfousier/opencrabs/releases/tag/v0.1.3

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

[0.1.0]: https://github.com/adolfousier/opencrab/releases/tag/v0.1.0
