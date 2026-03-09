# Testing Guide

Comprehensive test coverage for OpenCrabs. All tests run with:

```bash
cargo test --all-features
```

## Quick Reference

| Category | Tests | Location |
|----------|------:|----------|
| CLI Parsing | 28 | `src/tests/cli_test.rs` |
| Cron Jobs & Scheduling | 45 | `src/tests/cron_test.rs` |
| Channel Search | 23 | `src/tests/channel_search_test.rs` |
| Voice STT Dispatch | 22 | `src/tests/voice_stt_dispatch_test.rs` |
| Voice Onboarding | 48 | `src/tests/voice_onboarding_test.rs` |
| Local Whisper (inline) | 25 | `src/channels/voice/local_whisper.rs` |
| Candle Whisper | 6 | `src/tests/candle_whisper_test.rs` |
| Evolve (Self-Update) | 12 | `src/tests/evolve_test.rs` |
| Session & Working Dir | 12 | `src/tests/session_working_dir_test.rs` |
| Message Compaction | 14 | `src/tests/compaction_test.rs` |
| Fallback Vision | 17 | `src/tests/fallback_vision_test.rs` |
| Onboarding Keys | 4 | `src/tests/onboarding_keys_test.rs` |
| **Total** | **256+** | |

---

## Test Modules

### CLI Parsing (`src/tests/cli_test.rs`)

Validates argument parsing for all CLI commands and flags.

- `chat`, `run`, `init`, `config`, `db`, `daemon` subcommands
- Flag combinations: `--debug`, `--config`, `--format`, `--auto-approve`
- Error cases: invalid format, missing prompt, invalid subcommand

### Cron Jobs & Scheduling (`src/tests/cron_test.rs`)

Full coverage of the cron job system across 4 test modules.

**CLI parsing** (15 tests) — `/cron add`, `/cron list`, `/cron remove`, `/cron enable/disable`, missing/invalid args

**Database** (9 async tests) — insert, find by ID/name, list all/enabled, delete, set enabled, update last run, full field round-trip

**Cron expressions** (3 tests) — valid/invalid expression validation, next-run calculation

**Service** (15 async tests) — create/list, missing fields, invalid cron, duplicate names, enable/disable, approval requirements, deliver-to routing, due-time calculation

**Session tracking** (4 tests) — follows user to current session, fallback to initial session, shared session ID updates

### Channel Search (`src/tests/channel_search_test.rs`)

Tests the channel message search repository and tool.

**Repository** (11 async tests) — insert/recent, limit, channel filter, content search, cross-chat search, list chats, duplicate handling, field round-trip

**Tool** (12 async tests) — list chats (empty/with data/filtered), recent messages (requires chat_id, returns messages, empty, n-limit), search (requires query, finds messages, channel filter, no match, unknown operation)

### Voice STT Dispatch (`src/tests/voice_stt_dispatch_test.rs`)

Tests STT routing logic, audio decoding, and codec support.

**Dispatch routing** (5 async tests)
- API mode: requires key, empty key fails, provider-no-key fails
- Local mode: unknown model fails, model-not-downloaded fails (feature-gated)

**VoiceConfig** (3 tests) — default is API mode, local STT from providers TOML, empty config defaults

**Audio decoding** (4 tests, `local-stt` feature) — empty bytes fails, WAV magic detection, WAV sine generation + decode, resampler identity

**Codec support** (3 tests, `local-stt` feature) — Opus decoder registration, symphonia OGG probe, model preset validation

**API mock** (3 async tests) — Groq dispatch, mode selection routing, local whisper dispatch

**Quick-jump** (3 tests) — quick-jump done triggers flag, Esc returns cancel, non-quick-jump advances step

### Voice Onboarding (`src/tests/voice_onboarding_test.rs`)

Tests the voice setup wizard step: STT/TTS mode selection, key input, model picker, navigation, config persistence, availability-gated cycling.

**STT mode selection** (6 tests) — starts on SttModeSelect, cycles Up/Down, Off/API/Local Tab targets, Enter same as Tab

**Groq API key** (4 tests) — typing appends, backspace removes, Tab/BackTab navigation

**Local model selection** (5 tests) — Tab/BackTab navigation, Enter triggers download or advances, Enter during download is no-op

**TTS mode selection** (7 tests) — cycles Up/Down, tts_enabled flag, Off/API enter advances, Local enter goes to voice picker, BackTab targets

**TTS local voice** (5 tests) — BackTab to TTS mode, Tab advances step, Enter triggers download, Enter during download is no-op, Up/Down cycles voices

**Capability detection** (3 tests)
- `local_stt_available` matches compile-time feature flag
- `local_tts_available` matches feature + python3 probe
- `local_tts_available` is cached via OnceLock

**Availability-gated cycling** (5 tests)
- STT cycles to Local when `local-stt` feature enabled
- STT Up from Off goes to Local when available
- TTS cycles to Local when `local-tts` + python3 available
- TTS skips Local when unavailable
- STT skips Local when unavailable

**Wizard reset** (2 tests)
- `from_config` resets saved Local STT to Off when unavailable
- `from_config` resets saved Local TTS to Off when unavailable

**Config persistence** (4 tests) — SttMode/TtsMode serde round-trip, TuiEvent variants

**Navigation flow** (4 tests) — full API flow, Channels->Voice->Image step transitions

**Rendering smoke tests** (5 tests) — produces lines, API mode shows Groq field, Local mode shows model selector, TTS section, voice list

### Local Whisper Inline Tests (`src/channels/voice/local_whisper.rs`)

Unit tests co-located with the local STT engine (gated behind `local-stt` feature).

**Transcript cleaning** (3 tests) — whitespace collapse, newlines/tabs, empty input

**PcmSource (rodio::Source)** (4 tests) — iterates all samples, empty source, channels/sample_rate/duration metadata, frame_len decreases

**Whisper source parsing** (2 tests) — all presets parse successfully, unknown source fails

**Model management** (3 tests) — `is_model_downloaded` always true (rwhisper auto-downloads), unique preset IDs, model path under opencrabs dir

**Audio decoding** (4 tests) — empty bytes fails, garbage bytes fails, WAV sine decode, stereo-to-mono mixdown

**Resampling** (2 tests) — 48kHz to 16kHz ratio, resampled audio is non-silent

**Audio sanitization** (4 tests) — NaN/Inf scrubbing, short audio padded to 16000 samples, at-minimum not padded, above-minimum not padded

**Download progress** (2 tests) — done state, error state

**Preset validation** (1 test) — default preset is QuantizedTiny (multilingual)

### Candle Whisper (`src/tests/candle_whisper_test.rs`)

Validates mel filterbank computation and model presets for the rwhisper integration.

- Mel filters: correct shape (80 and 128 bins), non-zero values, no NaN/Inf/negative
- Model presets: required fields, find by ID

### Evolve / Self-Update (`src/tests/evolve_test.rs`)

Tests version comparison and asset naming for the self-update system.

**Version comparison** (7 tests) — major/minor/patch bumps, equal returns false, older returns false, different semver lengths, non-numeric segments

**Asset naming** (3 tests) — single binary format (`opencrabs-v{tag}-{platform}.tar.gz`), Windows `.zip`, legacy fallback without version

**Binary identity** (1 test) — binary name is always `opencrabs`

**Platform support** (1 test) — current platform has a recognized suffix

### Session & Working Directory (`src/tests/session_working_dir_test.rs`)

**Working directory** (4 async tests) — new session has no dir, update persists, default has no dir, multiple sessions have independent dirs

**Update checker** (8 tests) — newer patch/minor/major, same version, older version, two-segment versions

### Message Compaction (`src/tests/compaction_test.rs`)

**Snapshot formatting** (14 tests) — empty messages, user/assistant text, system messages, tool use/result blocks, image blocks, long text truncation at 500 chars

### Fallback Vision (`src/tests/fallback_vision_test.rs`)

**Fallback chain** (9+ tests) — empty config, legacy single provider, providers array, array+legacy dedup, TOML deserialization variants

### Onboarding Keys (`src/tests/onboarding_keys_test.rs`)

- Provider count matches constants
- Custom provider detection
- All providers use api_key_input
- keys.toml has all provider sections

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
