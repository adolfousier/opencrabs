# Testing Guide

Comprehensive test coverage for OpenCrabs. All tests run with:

```bash
cargo test --all-features
```

## Quick Reference

| Category | Tests | Location |
|----------|------:|----------|
| **Brain — Agent Service** | 42 | `src/brain/agent/service.rs` |
| **Brain — Prompt Builder** | 20 | `src/brain/prompt_builder.rs` |
| **Brain — Agent Context** | 12 | `src/brain/agent/context.rs` |
| **Brain — Provider (Anthropic)** | 9 | `src/brain/provider/anthropic.rs` |
| **Brain — Provider (Retry)** | 9 | `src/brain/provider/retry.rs` |
| **Brain — Provider (Custom OpenAI)** | 9 | `src/brain/provider/custom_openai_compatible.rs` |
| **Brain — Provider (Factory)** | 4 | `src/brain/provider/factory.rs` |
| **Brain — Provider (Copilot)** | 8 | `src/brain/provider/copilot.rs` |
| **Brain — Provider (Types/Error/Trait)** | 7 | `src/brain/provider/` |
| **Brain — Tokenizer** | 8 | `src/brain/tokenizer.rs` |
| **Brain — Commands** | 6 | `src/brain/commands.rs` |
| **Brain — Self-Update** | 1 | `src/brain/self_update.rs` |
| **Brain Tools — Plan Security** | 20 | `src/brain/tools/plan_tool.rs` |
| **Brain Tools — Exa Search** | 18 | `src/brain/tools/exa_search.rs` |
| **Brain Tools — Write File** | 17 | `src/brain/tools/write_opencrabs_file.rs` |
| **Brain Tools — A2A Send** | 16 | `src/brain/tools/a2a_send.rs` |
| **Brain Tools — Load Brain File** | 14 | `src/brain/tools/load_brain_file.rs` |
| **Brain Tools — Brave Search** | 12 | `src/brain/tools/brave_search.rs` |
| **Brain Tools — Doc Parser** | 10 | `src/brain/tools/doc_parser.rs` |
| **Brain Tools — Registry** | 7 | `src/brain/tools/registry.rs` |
| **Brain Tools — Slash Command** | 6 | `src/brain/tools/slash_command.rs` |
| **Brain Tools — Bash** | 21 | `src/brain/tools/bash.rs` |
| **Brain Tools — Write/Read/Config/Memory/Error** | 16 | `src/brain/tools/` |
| **Channels — Voice Service** | 14 | `src/channels/voice/service.rs` |
| **Channels — Voice Local TTS** | 14 | `src/channels/voice/local_tts.rs` |
| **Channels — Voice Local Whisper** | 25 | `src/channels/voice/local_whisper.rs` |
| **Channels — Commands** | 14 | `src/channels/commands.rs` |
| **Channels — WhatsApp Store** | 15 | `src/channels/whatsapp/store.rs` |
| **Channels — WhatsApp Handler** | 5 | `src/channels/whatsapp/handler.rs` |
| **Channels — Telegram Handler** | 8 | `src/channels/telegram/handler.rs` |
| **Channels — Slack Handler** | 2 | `src/channels/slack/handler.rs` |
| **Channels — Discord Handler** | 2 | `src/channels/discord/handler.rs` |
| **Channels — General** | 5 | `src/channels/` |
| **Config — Types** | 19 | `src/config/types.rs` |
| **Config — Secrets** | 5 | `src/config/secrets.rs` |
| **Config — Update** | 4 | `src/config/update.rs` |
| **Config — Crabrace** | 3 | `src/config/crabrace.rs` |
| **DB — Repository (Plan)** | 15 | `src/db/repository/plan.rs` |
| **DB — Retry** | 8 | `src/db/retry.rs` |
| **DB — Database** | 5 | `src/db/database.rs` |
| **DB — Models** | 4 | `src/db/models.rs` |
| **DB — Repository (Other)** | 9 | `src/db/repository/` |
| **Services — Plan** | 11 | `src/services/plan.rs` |
| **Services — File** | 11 | `src/services/file.rs` |
| **Services — Message** | 10 | `src/services/message.rs` |
| **Services — Session** | 9 | `src/services/session.rs` |
| **Services — Context** | 2 | `src/services/context.rs` |
| **A2A — Debate** | 8 | `src/a2a/debate.rs` |
| **A2A — Types** | 6 | `src/a2a/types.rs` |
| **A2A — Server/Handler/Agent Card** | 7 | `src/a2a/` |
| **Memory — Store** | 6 | `src/memory/store.rs` |
| **Memory — Search** | 3 | `src/memory/search.rs` |
| **Pricing** | 13 | `src/pricing.rs` |
| **Logging** | 4 | `src/logging/logger.rs` |
| **Utils — Install** | 6 | `src/utils/install.rs` |
| **Utils** | 1 | `src/utils/` |
| **CLI** | 1 | `src/cli.rs` |
| Tests — CLI Parsing | 28 | `src/tests/cli_test.rs` |
| Tests — Cron Jobs & Scheduling | 49 | `src/tests/cron_test.rs` |
| Tests — Channel Search | 24 | `src/tests/channel_search_test.rs` |
| Tests — Voice STT Dispatch | 11 | `src/tests/voice_stt_dispatch_test.rs` |
| Tests — Voice Onboarding | 62 | `src/tests/voice_onboarding_test.rs` |
| Tests — Candle Whisper | 6 | `src/tests/candle_whisper_test.rs` |
| Tests — Evolve (Self-Update) | 23 | `src/tests/evolve_test.rs` |
| Tests — Session & Working Dir | 15 | `src/tests/session_working_dir_test.rs` |
| Tests — Message Compaction | 24 | `src/tests/compaction_test.rs` |
| Tests — Fallback Vision | 35 | `src/tests/fallback_vision_test.rs` |
| Tests — GitHub Copilot Provider | 38 | `src/tests/github_provider_test.rs` |
| Tests — File Extract | 36 | `src/tests/file_extract_test.rs` |
| Tests — Image Utils | 9 | `src/tests/image_util_test.rs` |
| Tests — Onboarding Brain | 21 | `src/tests/onboarding_brain_test.rs` |
| Tests — Onboarding Navigation | 26 | `src/tests/onboarding_navigation_test.rs` |
| Tests — Onboarding Types | 16 | `src/tests/onboarding_types_test.rs` |
| Tests — Onboarding Keys | 4 | `src/tests/onboarding_keys_test.rs` |
| Tests — OpenAI Provider | 16 | `src/tests/openai_provider_test.rs` |
| Tests — Plan Document | 15 | `src/tests/plan_document_test.rs` |
| Tests — TUI Error | 16 | `src/tests/tui_error_test.rs` |
| Tests — Queued Messages | 15 | `src/tests/queued_message_test.rs` |
| Tests — Custom Provider | 27 | `src/tests/custom_provider_test.rs` |
| Tests — Context Window | 14 | `src/tests/context_window_test.rs` |
| Tests — Onboarding Field Nav | 46 | `src/tests/onboarding_field_nav_test.rs` |
| **Total** | **1,286** | |

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

## Disabled Test Modules

These modules exist but are commented out in `src/tests/mod.rs` (require network or external services):

| Module | Reason |
|--------|--------|
| `error_scenarios_test` | Requires mock API server |
| `integration_test` | End-to-end with LLM provider |
| `plan_mode_integration_test` | End-to-end plan workflow |
| `streaming_test` | Requires streaming API endpoint |
