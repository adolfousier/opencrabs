# Testing Guide

Comprehensive test coverage for OpenCrabs. All tests run with:

```bash
cargo test --all-features
```

## Quick Reference

| Category | Tests | Location |
|----------|------:|----------|
| **Brain — Agent Service** | 52 | `src/brain/agent/service/` |
| **Brain — Prompt Builder** | 20 | `src/brain/prompt_builder.rs` |
| **Brain — Agent Context** | 12 | `src/brain/agent/context.rs` |
| **Brain — Provider (Anthropic)** | 9 | `src/brain/provider/anthropic.rs` |
| **Brain — Provider (Retry)** | 9 | `src/brain/provider/retry.rs` |
| **Brain — Provider (Custom OpenAI)** | 9 | `src/brain/provider/custom_openai_compatible.rs` |
| **Brain — Provider (Copilot)** | 8 | `src/brain/provider/copilot.rs` |
| **Brain — Provider (Factory)** | 4 | `src/brain/provider/factory.rs` |
| **Brain — Provider (Claude CLI)** | 4 | `src/brain/provider/claude_cli.rs` |
| **Brain — Provider (Types/Error/Trait)** | 7 | `src/brain/provider/` |
| **Brain — Tokenizer** | 8 | `src/brain/tokenizer.rs` |
| **Brain — Commands** | 6 | `src/brain/commands.rs` |
| **Brain — Self-Update** | 1 | `src/brain/self_update.rs` |
| **Brain Tools — Bash** | 21 | `src/brain/tools/bash.rs` |
| **Brain Tools — Plan Security** | 20 | `src/brain/tools/plan_tool.rs` |
| **Brain Tools — Exa Search** | 18 | `src/brain/tools/exa_search.rs` |
| **Brain Tools — Write File** | 17 | `src/brain/tools/write_opencrabs_file.rs` |
| **Brain Tools — A2A Send** | 16 | `src/brain/tools/a2a_send.rs` |
| **Brain Tools — Load Brain File** | 15 | `src/brain/tools/load_brain_file.rs` |
| **Brain Tools — Brave Search** | 12 | `src/brain/tools/brave_search.rs` |
| **Brain Tools — Browser Manager** | 12 | `src/brain/tools/browser/manager.rs` |
| **Brain Tools — Tool Manage** | 11 | `src/brain/tools/tool_manage.rs` |
| **Brain Tools — Dynamic Tools** | 17 | `src/brain/tools/dynamic/` |
| **Brain Tools — Doc Parser** | 10 | `src/brain/tools/doc_parser.rs` |
| **Brain Tools — Registry** | 7 | `src/brain/tools/registry.rs` |
| **Brain Tools — Slash Command** | 6 | `src/brain/tools/slash_command.rs` |
| **Brain Tools — Write/Read/Config/Memory/Error** | 20 | `src/brain/tools/` |
| **Channels — Voice Local Whisper** | 25 | `src/channels/voice/local_whisper.rs` |
| **Channels — Voice Service** | 14 | `src/channels/voice/service.rs` |
| **Channels — Voice Local TTS** | 14 | `src/channels/voice/local_tts.rs` |
| **Channels — Commands** | 15 | `src/channels/commands.rs` |
| **Channels — WhatsApp Store** | 15 | `src/channels/whatsapp/store.rs` |
| **Channels — Telegram Handler** | 8 | `src/channels/telegram/handler.rs` |
| **Channels — WhatsApp Handler** | 5 | `src/channels/whatsapp/handler.rs` |
| **Channels — General** | 5 | `src/channels/` |
| **Channels — Slack Handler** | 2 | `src/channels/slack/handler.rs` |
| **Channels — Discord Handler** | 2 | `src/channels/discord/handler.rs` |
| **Config — Types** | 19 | `src/config/types.rs` |
| **Config — Secrets** | 5 | `src/config/secrets.rs` |
| **Config — Update** | 4 | `src/config/update.rs` |
| **Config — Crabrace** | 3 | `src/config/crabrace.rs` |
| **DB — Repository (Plan)** | 15 | `src/db/repository/plan.rs` |
| **DB — Retry** | 8 | `src/db/retry.rs` |
| **DB — Repository (Other)** | 9 | `src/db/repository/` |
| **DB — Database** | 5 | `src/db/database.rs` |
| **DB — Models** | 4 | `src/db/models.rs` |
| **Services — Plan** | 11 | `src/services/plan.rs` |
| **Services — File** | 11 | `src/services/file.rs` |
| **Services — Message** | 10 | `src/services/message.rs` |
| **Services — Session** | 10 | `src/services/session.rs` |
| **Services — Context** | 2 | `src/services/context.rs` |
| **TUI — Onboarding** | 67 | `src/tui/onboarding/` |
| **TUI — Plan** | 25 | `src/tui/plan.rs` |
| **TUI — Render Utils** | 12 | `src/tui/render/utils.rs` |
| **TUI — Prompt Analyzer** | 8 | `src/tui/prompt_analyzer.rs` |
| **TUI — Highlight** | 8 | `src/tui/highlight.rs` |
| **TUI — Markdown** | 7 | `src/tui/markdown.rs` |
| **TUI — Error** | 5 | `src/tui/error.rs` |
| **TUI — Events** | 4 | `src/tui/events.rs` |
| **TUI — Components** | 2 | `src/tui/components/` |
| **TUI — App State** | 1 | `src/tui/app/state.rs` |
| **A2A — Debate** | 8 | `src/a2a/debate.rs` |
| **A2A — Types** | 6 | `src/a2a/types.rs` |
| **A2A — Server/Handler/Agent Card** | 7 | `src/a2a/` |
| **Memory — Store** | 6 | `src/memory/store.rs` |
| **Memory — Search** | 3 | `src/memory/search.rs` |
| **Pricing** | 17 | `src/pricing.rs` |
| **Utils — Sanitize** | 22 | `src/utils/sanitize.rs` |
| **Utils — Retry** | 8 | `src/utils/retry.rs` |
| **Utils — String** | 6 | `src/utils/string.rs` |
| **Utils — Install** | 6 | `src/utils/install.rs` |
| **Utils — Config Watcher** | 2 | `src/utils/config_watcher.rs` |
| **Logging** | 4 | `src/logging/logger.rs` |
| Tests — Voice STT Dispatch | 21 | `src/tests/voice_stt_dispatch_test.rs` |
| Tests — Voice Onboarding | 65 | `src/tests/voice_onboarding_test.rs` |
| Tests — Voice OpenAI Compatible | 12 | `src/tests/voice_openai_compatible_test.rs` |
| Tests — Voice Voicebox | 10 | `src/tests/voice_voicebox_test.rs` |
| Tests — Cron Jobs & Scheduling | 49 | `src/tests/cron_test.rs` |
| Tests — Onboarding Field Nav | 46 | `src/tests/onboarding_field_nav_test.rs` |
| Tests — GitHub Copilot Provider | 38 | `src/tests/github_provider_test.rs` |
| Tests — File Extract | 36 | `src/tests/file_extract_test.rs` |
| Tests — Fallback Vision | 35 | `src/tests/fallback_vision_test.rs` |
| Tests — CLI Parsing | 28 | `src/tests/cli_test.rs` |
| Tests — Custom Provider | 27 | `src/tests/custom_provider_test.rs` |
| Tests — Onboarding Navigation | 26 | `src/tests/onboarding_navigation_test.rs` |
| Tests — Message Compaction | 24 | `src/tests/compaction_test.rs` |
| Tests — Channel Search | 24 | `src/tests/channel_search_test.rs` |
| Tests — Evolve (Self-Update) | 23 | `src/tests/evolve_test.rs` |
| Tests — Slack Formatting | 21 | `src/tests/slack_fmt_test.rs` |
| Tests — Split Pane | 21 | `src/tests/split_pane_test.rs` |
| Tests — OpenCode CLI Provider | 21 | `src/tests/opencode_provider_test.rs` |
| Tests — Onboarding Brain | 21 | `src/tests/onboarding_brain_test.rs` |
| Tests — Onboarding Types | 16 | `src/tests/onboarding_types_test.rs` |
| Tests — OpenAI Provider | 16 | `src/tests/openai_provider_test.rs` |
| Tests — TUI Error | 16 | `src/tests/tui_error_test.rs` |
| Tests — Queued Messages | 15 | `src/tests/queued_message_test.rs` |
| Tests — Plan Document | 15 | `src/tests/plan_document_test.rs` |
| Tests — Session & Working Dir | 15 | `src/tests/session_working_dir_test.rs` |
| Tests — Stream Loop Detection | 15 | `src/tests/stream_loop_test.rs` |
| Tests — Context Window | 14 | `src/tests/context_window_test.rs` |
| Tests — HTML Comment Strip | 11 | `src/tests/html_comment_strip_test.rs` |
| Tests — Daemon Health & Config | 10 | `src/tests/daemon_health_test.rs` |
| Tests — XML Tool Fallback | 10 | `src/tests/xml_tool_fallback_test.rs` |
| Tests — Collapse Build Output | 9 | `src/tests/collapse_build_output_test.rs` |
| Tests — Image Utils | 9 | `src/tests/image_util_test.rs` |
| Tests — Brain Templates | 8 | `src/tests/brain_templates_test.rs` |
| Tests — AltGr Input | 8 | `src/tests/altgr_input_test.rs` |
| Tests — QR Render | 8 | `src/tests/qr_render_test.rs` |
| Tests — Provider Sync | 8 | `src/tests/provider_sync_test.rs` |
| Tests — WhatsApp State | 7 | `src/tests/whatsapp_state_test.rs` |
| Tests — Reasoning Lines | 6 | `src/tests/reasoning_lines_test.rs` |
| Tests — System Continuation | 6 | `src/tests/system_continuation_test.rs` |
| Tests — Candle Whisper | 6 | `src/tests/candle_whisper_test.rs` |
| Tests — Post-Evolve | 5 | `src/tests/post_evolve_test.rs` |
| Tests — Onboarding Keys | 4 | `src/tests/onboarding_keys_test.rs` |
| Tests — TUI Render Clear | 4 | `src/tests/tui_render_clear_test.rs` |
| Tests — Gemini Fetch | 3 | `src/tests/gemini_fetch_test.rs` |
| Tests — Profiles | 57 | `src/tests/profile_test.rs` |
| Tests — Subagent / Swarm | 84 | `src/tests/subagent_test.rs` |
| Tests — Telegram Resume & Helpers | 55 | `src/tests/telegram_resume_test.rs` |
| Tests — Token Tracking | 8 | `src/tests/token_tracking_test.rs` |
| Tests — wait_agent Resolver | 12 | `src/tests/wait_agent_resolver_test.rs` |
| Tests — Browser Default (macOS LSHandlers parser) | 8 | `src/tests/browser_default_test.rs` |
| Tests — exa_search MCP Handshake | 4 | `src/tests/exa_search_test.rs` |
| Tests — http_request User-Agent | 3 | `src/tests/http_request_test.rs` |
| **Usage — Categorizer** | 4 | `src/usage/categorizer.rs` |
| **Usage — Dashboard** | 6 | `src/usage/dashboard.rs` |
| **Usage — Data** | 7 | `src/usage/data.rs` |
| **Total** | **2,078** | |

---

## Feature-Gated Tests

Some tests only compile/run with specific feature flags:

| Feature | Tests |
|---------|-------|
| `local-stt` | Local whisper inline tests, candle whisper tests, STT dispatch local-mode tests, codec tests, availability cycling tests |
| `local-tts` | TTS voice cycling, Piper voice Up/Down |

All feature-gated tests use `#[cfg(feature = "...")]` and are automatically included when running with `--all-features`.

---

## Running Tests

```bash
# Run all tests (recommended)
cargo test --all-features

# Run a specific test module
cargo test --all-features -- voice_onboarding_test

# Run a single test
cargo test --all-features -- is_newer_major_bump

# Run with output (for debugging)
cargo test --all-features -- --nocapture

# Run only local-stt tests
cargo test --features local-stt -- local_whisper
```

---

## Profile Tests

Profile tests live in `src/tests/profile_test.rs` and cover multi-instance isolation:

| Area | What's tested |
|------|--------------|
| Name validation | Reserved names, length bounds, special characters |
| Token hashing | Determinism, uniqueness, fixed length, hex output |
| Registry (in-memory) | CRUD, serde roundtrip, touch timestamps |
| Path resolution | Base dir, env var override, default vs named profiles |
| Filesystem CRUD | Create/delete lifecycle, duplicate detection, registry sync |
| Export/Import | Roundtrip with config files, nested memory directories |
| Migration | Copy `.md`/`.toml` files, skip/force behavior, default source |
| Token locks | Acquire/release, stale PID cleanup, cross-profile conflict |
| Profile isolation | Separate directories, concurrent writes, default vs named |
| Concurrent writes | Tokio tasks creating 5 profiles simultaneously |

```bash
# Run profile tests only
cargo test --all-features -p opencrabs -- profile_test
```

**Note:** All filesystem-touching tests acquire a global `fs_lock()` mutex to prevent concurrent write corruption of `~/.opencrabs/profiles.toml`. The mutex uses `unwrap_or_else(|p| p.into_inner())` to recover from poison (a prior test panic won't cascade-fail every subsequent test). In-memory tests run in parallel without the lock. The `test_set_and_get_active_profile` test accounts for `OnceLock` semantics (can only be set once per process).

---

## Disabled Test Modules

These modules exist but are commented out in `src/tests/mod.rs` (require network or external services):

| Module | Reason |
|--------|--------|
| `error_scenarios_test` | Requires mock API server |
| `integration_test` | End-to-end with LLM provider |
| `plan_mode_integration_test` | End-to-end plan workflow |
| `streaming_test` | Requires streaming API endpoint |
