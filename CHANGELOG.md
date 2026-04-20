# Changelog

All notable changes to OpenCrabs will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.13] - 2026-04-20

### Self-healing — phantom detector expansion

| Type | Area | Change | Why / Impact |
|------|------|--------|--------------|
| add | self-healing | **14 investigative intent phrases** (`let me hunt/trace/track/look into/check into/find out/dig into` + `i'll` variants) | Catches "Let me hunt down where..." drops where agent announces investigation but never calls tools |
| add | self-healing | **14 "Now + gerund" patterns** (`now cherry-picking/updating/fixing/committing/pushing/merging/rebasing/deploying/building/testing/checking/applying/restarting`) | Catches status-report-then-action drops like "Now cherry-picking to main..." followed by silence |
| add | self-healing | **Regex-based gerund detection** with sentence boundary requirement | Prevents false positives like "Are you now checking the logs?" |
| refactor | self-healing | **Extracted `has_investigative_intent()`** as public helper + re-exported in service mod | Enables test coverage for phantom detection logic |

### Channel — duplicate response elimination

| Type | Area | Change | Why / Impact |
|------|------|--------|--------------|
| fix | slack | **Pre-send intermediate dedup** — checks `sent_intermediates` before sending | Same intermediate emitted twice during streaming no longer sends twice |
| fix | slack | **Sanitized intermediate storage** — `strip_llm_artifacts` + `redact_secrets` applied before storing | Final response `.replace()` dedup now matches correctly (raw vs sanitized mismatch) |
| fix | slack | **Image download fallback** — tries `url_private` when `url_private_download` returns HTML | Slack sometimes returns HTML preview page instead of raw bytes; now falls back instead of saving HTML as `.png` |
| fix | whatsapp | **Pre-send intermediate dedup** with `sent_intermediates` tracking + sanitized storage | Same pattern as Slack/Telegram — prevents duplicate intermediates during streaming |

### Browser tools — new tool + 10 reliability fixes

| Type | Area | Change | Why / Impact |
|------|------|--------|--------------|
| add | browser | **`browser_find` tool** — enumerate page elements with stable selectors (CSS/XPath/text/aria modes) | Returns selectors passable directly to `browser_click` / `browser_type` without ambiguity |
| fix | browser | **`browser_click` waits for network-idle** instead of fixed 500ms sleep | Pages with async loads stabilize before screenshot |
| fix | browser | **Screenshot failure surfaced** in tool result instead of swallowed | Users see actual error instead of silent failure |
| fix | browser | **`browser_eval` caps output at 50 KB** | Prevents massive DOM dumps from blowing up context |
| fix | browser | **Waits up to 10s for user's Chrome profile** to unlock before fallback | Reduces SingletonLock failures when user's Chrome is running |
| fix | browser | **Per-session tab isolation** — no more cross-session DOM stomping | Multiple concurrent sessions no longer interfere |
| fix | browser | **Drop impl aborts CDP handler** + releases Browser properly | Prevents zombie Chrome processes on session end |
| fix | browser | **Stealth JS registered** via `addScriptToEvaluateOnNewDocument` | Anti-bot detection runs before any page script |
| fix | browser | **Detects dead CDP handler** and relaunches instead of zombie | Recovers from crashed Chrome automatically |
| fix | browser | **Sweeps stale SingletonLock** before launch | Fixes launch failures from crashed Chrome leaving lock files |
| fix | browser | **Detects user's default browser** correctly on macOS | Uses `LSHandlers` from `com.apple.LaunchServices` plist |

### Other tool fixes

| Type | Area | Change | Why / Impact |
|------|------|--------|--------------|
| fix | tools | **`wait_agent` resolves by prefix or label**, lists actives on miss | No more "No sub-agent found" when using short IDs or labels |
| fix | tools | **`exa_search` supports stateless MCP servers** (missing session header) | Fixes "MCP server did not return session ID" errors |
| fix | tools | **`http_request` sets default User-Agent** | Stops GitHub API 403 Forbidden errors |

### Build / test

| Type | Area | Change | Why / Impact |
|------|------|--------|--------------|
| test | self-healing | **5 investigative intent tests** in `src/tests/self_healing_test.rs` | Coverage for 14 new phrases + false positive prevention |
| test | self-healing | **2 gerund pattern tests** with 17 assertions | Coverage for "Now + gerund" detection + false positive prevention |
| test | tools | **12 tests** for `wait_agent` id resolver and miss-message | |
| test | browser | **8 tests** for macOS LSHandlers default-browser parser | |
| test | tools | **3 tests** for `http_request` User-Agent default fix | |
| test | tools | **4 tests** for `exa_search` stateless-MCP fallback | |
| test | browser | **Linux xdg + Windows reg** default-browser tests | |

**2,131 tests passing** (+75 since v0.3.12).

[0.3.13]: https://github.com/adolfousier/opencrabs/compare/v0.3.12...v0.3.13

## [0.3.12] - 2026-04-18

### Voice — new STT/TTS providers

| Type | Area | Change | Why / Impact |
|------|------|--------|--------------|
| add | voice | **OpenAI-compatible STT** via `stt_base_url` + `stt_model` + `providers.stt.openai_compatible.api_key` | Any Whisper-compatible `/v1/audio/transcriptions` endpoint (self-hosted Whisper, Deepgram-compatible proxies) |
| add | voice | **OpenAI-compatible TTS** via `tts_base_url` + `tts_model` + `tts_voice` + `providers.tts.openai_compatible.api_key` | Any `/v1/audio/speech` endpoint (self-hosted Coqui/Bark, ElevenLabs-compatible proxies) |
| add | voice | **Voicebox STT** via `voicebox_stt_enabled=true` + `voicebox_stt_base_url` | Self-hosted open-source voice stack, no API key |
| add | voice | **Voicebox TTS** via `voicebox_tts_enabled=true` + `voicebox_tts_base_url` + `voicebox_tts_profile_id` | Async `POST /generate` → poll `/generate/{id}/status` → fetch audio from returned path (HTTP(S) URL / server-relative path / local filesystem) |
| add | voice | **Unified provider dispatch** in `voice::transcribe` / `voice::synthesize` | Priority chain: Voicebox → OpenAI-compatible → (Groq STT / OpenAI TTS) → Local. First match wins. Every TTS output normalised to OGG/Opus via `ensure_opus` before return |
| add | voice | **`SttProvider` / `TtsProvider` named enums** replace string discriminators | Typed variants (`Off` / `Groq` / `OpenAICompatible` / `Voicebox` / `Local`) prevent mismatched-string bugs at compile time |

### Voice — channel parity

| Type | Area | Change | Why / Impact |
|------|------|--------|--------------|
| add | slack | **TTS voice reply** via `files.upload` (OGG/Opus, inline waveform UI) | Was missing entirely — Slack voice-input users got text-only replies |
| add | channels | **Full STT/TTS parity** across Telegram / WhatsApp / Discord / Slack | Single code path via `crate::channels::voice::{transcribe, synthesize}` — no channel reinvents voice handling |
| fix | telegram | **`voice_msg_ids: Vec<MessageId>`** tracked in `StreamingState` | Doc-comment-enforced invariant: cleanup paths must never iterate this list. TTS voice notes are the most expensive artefact the bot produces (real synthesis) — losing one to a well-intentioned sweep is a regression we've made hard to introduce |
| fix | telegram | **Diagnosable voice drop on mid-stream cancellation** | When a new user message cancels a voice-input turn before the TTS block runs, `tracing::warn!` names the session. Previously silent — looked identical to `send_voice` failing |
| fix | telegram | **`send_voice` error no longer swallowed** | Logs real error + Debug repr instead of a generic "TTS failed" placeholder |
| fix | channels | **Bot replies recorded in `channel_messages`** table (Telegram/Discord/WhatsApp/Slack) | `channel_msg_repo.recent()` builds group/channel conversation context on every turn. Previously stored only user messages → bot saw a one-sided transcript and responded blind to its own prior replies. Now sender=`bot:opencrabs` / name=`OpenCrabs` (or bot's real ID on Discord) |

### Voice — Voicebox + format pipeline

| Type | Area | Change | Why / Impact |
|------|------|--------|--------------|
| fix | voicebox | **Audio fetch supports HTTP(S) + server-relative + filesystem paths** | Fixes silent "audio never arrived" when server returns a URL instead of a local path |
| fix | voicebox | **Async polling** — `POST /generate` → poll `/generate/{id}/status` every 500 ms up to 120 s | Fixes hang-or-fail on servers that don't synthesise synchronously |
| fix | voice | **ffmpeg uses `pipe:0`/`pipe:1`** instead of `/dev/stdin`/`/dev/stdout` | The named device paths don't exist on macOS; the pipe syntax works on Linux and macOS |
| fix | voice | **ffmpeg output uses `-f ogg`** explicitly | Raw Opus without Ogg container was being emitted — Telegram rejects it; every channel now gets a proper Ogg/Opus file |

### Onboarding

| Type | Area | Change | Why / Impact |
|------|------|--------|--------------|
| add | onboarding | **Voice screen rewrite** with 5-provider radio selectors per mode | Off / Groq / OpenAI-compatible / Voicebox / Local for STT; Off / OpenAI / OpenAI-compatible / Voicebox / Local for TTS. Fields shown/hidden based on selected provider |
| add | onboarding | **Voice API keys wired to `keys.toml`** | Adds `providers.stt.openai_compatible.api_key` + `providers.tts.openai_compatible.api_key`; previously only `config.toml` received the field |
| fix | onboarding | **Paste routes to the focused voice field** | Old handler keyed on field indices that shifted when providers were added — pasting into TTS base URL could land in STT model name |
| add | voice | **TTS dispatch logs** (`tracing::info!`) name the provider + its params | Failures now point at the right config; previously silent on the happy path |

### Telegram delivery / dedup

| Type | Area | Change | Why / Impact |
|------|------|--------|--------------|
| fix | telegram | **Kill duplicate responses at the source** (`ends_with_url` helper in `looks_truncated_mid_sentence` + pre-send dedup) | URL-terminated replies ("Done. Uploaded to Drive: https://…/view") were flagged truncated → model re-stated the whole answer → duplicate intermediates. Fixed the heuristic + added defensive pre-send dedup in both `handle_message` and `resume_session` |
| fix | telegram | **Keep prior intermediates + tool-call history on follow-up cancel** | Cancel path used to delete every prior intermediate + tool bubble. Now only deletes the typing placeholder; history stays visible |
| fix | telegram | **`redact_secrets` applied to intermediates** | Consistency with final-response redaction — otherwise a redacted final wouldn't match an un-redacted intermediate and dedup failed |
| fix | sanitize | **URL path segments no longer redacted as secrets** | Paths like `/api/v1/users/123` were treated as tokens and stripped. Redaction only fires on actual-secret patterns now |

### Provider / session resilience

| Type | Area | Change | Why / Impact |
|------|------|--------|--------------|
| fix | models | **Construct session provider by name on custom pick** — `/models` save calls `create_provider_by_name(&cfg, chosen)` instead of cloning `agent_service.provider()` | Fixes "session says opencode2 but routes to opencode" regression (per-session isolation lagged one rebuild cycle) |
| fix | models | **Verify `enabled=true` actually lands on disk**, retry via `try_write` on drift | No more silent divergence between dialog state and config.toml |
| fix | agent | **Sticky fallback on 429 / 401 / 403** — session provider swap holds, `ProviderSwitched` persists to DB, alerts read `'provider/model'` | Otherwise every turn hit the same rate-limit, walked the chain, bounced back. Users with opencode / opencode2 / opencode3 can now tell which subscription got limited |
| fix | fallback | **Cancellation-safe swap** via `FallbackProviderGuard` Drop — restores on error + future-drop | Fixes "400 Unknown Model" from a mid-fallback cancel leaving `session_providers` on fallback while `session.model` still pointed at primary |
| fix | provider | **Hard invariant against mismatched `{provider, model}` pair** in `stream_complete` | Remaps to provider default + warns if model not in `supported_models()`. Catches every stale-pin / upstream-bug / cancelled-fallback path |
| fix | provider | **Custom provider wins name collisions** against built-in ids (`opencode`, `anthropic`, …) | Prevents custom entry with a built-in name from silently spawning a CLI subprocess |
| fix | models | **Custom edit = update in place; rename = table-key move** with api_key preserved | Fixes rename-duplicate that left `opencodeiolo` keyless on 2026-04-18 |
| fix | agent | **401 / 403 trigger fallback chain** instead of terminal error | Retry-in-place is pointless when the key is bad; next provider has its own key |
| fix | session | **`session.provider_name` not silently overwritten on swap failure** | Old code overwrote with fallback's name in DB → next restart loaded the wrong provider |

### TUI — reasoning renderer + /models + /usage

| Type | Area | Change | Why / Impact |
|------|------|--------|--------------|
| fix | tui | **Reasoning renderer double-wrap + double-padding eliminated** (3 sites in `chat.rs`) | Each paragraph was wrapped twice → inconsistent 2-vs-4 space continuation indent + awkward word breaks like `combined like.this.and.this`. Renderer reserves outer indent in wrap budget, skips second pass |
| fix | tui | **Flush-left reasoning wraps** — continuation padding switched from `"  "` to `""` | Continuations now align with paragraph starts at a consistent 2-space column |
| fix | tui | **Faster `/models` navigation** — mid-field navigation skips `rebuild_agent_service` | Old flow froze 5–10 s per field move (HTTP health checks + subprocess spawns). Enabled flag / base_url / default_model still persist; full rebuild runs once on final Enter |
| fix | channels | **Full `/usage` dashboard on Telegram/Discord/Slack/WhatsApp** | Was showing only current-session line + top 5 models; now renders 5 period-grouped cards (Daily / Projects / Models / Tools / Activity) per period (Today + All-Time), within the 4096-char Telegram cap |

### Brain

| Type | Area | Change | Why / Impact |
|------|------|--------|--------------|
| add | brain | **`TOOLS.md` always injected** into core brain | On-demand loading caused the model to guess CLI syntax from training data (duplicate `gdrive` uploads on 2026-04-18). Actual tool syntax now always available |

## [0.3.11] - 2026-04-17

Major refactor: Qwen OAuth rotation replaced with DashScope API-key provider,
per-session provider isolation, local model tool-call extraction, and 40+ TUI
and self-heal fixes.

The headline change is the DashScope migration: Qwen OAuth rotation (10+ files,
device flow, credential persistence, rotation logic) was replaced with a simple
API-key provider (`qwen3.6-plus` default). The CLI provider was also removed.
This deletes ~2,500 lines of complexity.

Local models (Qwen, Unsloth) now have their tool-call hallucinations
auto-extracted from text content: bare JSON `{"tool_calls":[...]}` envelopes,
Claude-style XML `<TOOLNAME><PARAM>value</PARAM></TOOLNAME>`, and
Qwen-specific formats are all recovered into real executable tool calls.

Per-session provider isolation means switching models in one session no longer
affects Telegram, Discord, or other sessions. Each session carries its own
provider and context budget.

### Added

#### Qwen/DashScope Migration
- **DashScope API-key provider** — replaces OAuth rotation with a single
  API key. `qwen3.6-plus` as default model.
- **API-key onboarding with DashScope defaults** — `/models` and `/onboard`
  walk through key entry; no more device flow or multi-account management.
- **`qwen3.6-plus` default model** — added as the DashScope default.
- **`chat_template_kwargs` injection** — local thinking models receive proper
  `enable_thinking` flags via DashScope-compatible body transforms.
- **Qwen-style tool call recovery from text content** — detects Qwen's
  `<!-- tool_calls -->` or raw JSON in `delta.content` and converts into
  real tool calls before the tool loop sees them.

#### TUI
- **Stack queued messages instead of replacing** — multiple messages queued
  while the agent processes now stack visibly instead of overwriting each
  other.
- **Shell-mode visual cue** — input starting with `!` shows a visual `shell`
  indicator matching the original design.
- **Collapse `$HOME` → `~` and middle-truncate tool summaries** — channel
  delivery paths and long tool descriptions are shortened for readability.

#### Brain
- **Anti-code-block nudge for local models** — brain instructions explicitly
  tell the model to use `tool_calls`, not markdown code blocks.

### Changed

#### Qwen/DashScope Migration
- **Rewrote `qwen.rs` as thin DashScope header/body helper** — the module is
  now ~200 lines (was ~1,500 with rotation, device flow, and credential
  management).
- **Removed Qwen CLI provider entirely** — `qwen-code-cli` no longer appears
  in the provider list; all routing goes through DashScope API.
- **Dropped `qwen_accounts` rotation field** — config.toml no longer has
  `[[providers.qwen_accounts]]`; replaced with single `api_key` in keys.toml.
- **Stripped Qwen OAuth device flow and rotation wizard** — onboarding no
  longer shows the multi-account setup screen.
- **Route provider-specific branches by id, not index** — TUI dialogs match
  on provider identifier strings instead of hardcoded numeric indices, so
  removing the CLI provider doesn't shift everything.

#### Agent
- **Per-session provider isolation** — each session carries its own provider
  instance; no more global swap that affected all sessions simultaneously.
- **Per-session ctx budget isolation** — context budget calculated per session
  with last-iteration prompt size tracking.

### Fixed

#### Provider / Stream
- **Extract bare OpenAI `tool_call` envelopes from text content** — local
  models sometimes emit `{"tool_calls":[...]}` as plain text alongside (or
  instead of) proper SSE tool_call chunks. These are now detected and
  converted into executable tool calls.
- **Extract Claude-style XML tool calls** — `<TOOLNAME><PARAM>value</PARAM>
  </TOOLNAME>` patterns recovered into real tool calls.
- **Handle singular `tool_call` envelope + malformed JSON** — some models
  send `"tool_call"` (singular) or truncated JSON; both are handled.
- **Suppress local tool-call markers from stream display** — `<!-- tool_calls
  -->` and similar markers no longer appear as visible text in the TUI.
- **Surface real 422 body + helpful hint for Unsloth Studio** — local
  endpoints that return 422 now show the actual error body instead of a
  generic connection error.
- **Bound initial stream handshake + funnel timeouts into retry** — the POST
  → response-headers phase is now time-limited (90s) so a local server that
  accepts TCP but never replies surfaces a `Timeout` error into the existing
  retry/fallback chain instead of sitting on reqwest's 300s default.
- **1h stream idle for local endpoints** — handshake stays tight (90s) as
  the "is the server alive?" detector, but once headers arrive the body
  stream gets a full hour. Covers legitimate large-model prefill on
  Unsloth / llama.cpp / LM Studio / Ollama / MLX where prompt processing
  on a 70B at full context can run tens of minutes before the first token.
  Earlier 30s / 90s / 15min tries all killed genuine turns.
- **Name fallback target in TUI alert** — the stream-error arm now emits
  `Trying fallback 'zhipu/glm-5.1'...` per attempt so users see which
  provider the session is now on after retries exhaust.
- **Sub-agent `AwaitingInput` state + wait_agent polling** — sub-agents now
  transition to `AwaitingInput` at round boundaries instead of sitting in
  `Running` forever. `wait_agent` polls state (250ms) and returns the
  round's output immediately on pause, terminal state, or a partial
  progress preview on timeout — eliminates the deadlock where `wait_agent`
  blocked on `handle.await` for a task that only terminates on input or
  cancel. LLMs no longer give up the parent turn after repeated empty
  "still running" responses.

#### Self-Heal / Phantom Detection
- **Narrow phantom gate, recover empty-reasoning turns** — models that
  produce only reasoning content (no text, no tools) are now recovered
  instead of flagged as phantom.
- **Split thinking per iteration** — reasoning state is cleared between tool
  loop iterations so stale thinking doesn't carry over.
- **Broaden phantom-retry gate for local providers** — local models trigger
  the retry-and-nudge path more aggressively since they're more prone to
  hallucinating tool calls.
- **Adopt Unsloth's blunt anti-code-block nudge** — when a local model emits
  code blocks instead of tool calls, the retry prompt is direct and explicit.
- **Tighten phantom scope** — fewer false positives on legitimate responses
  that happen to mention tools.
- **Detect phantom intent with backtick code references** — models that say
  "let me run `grep -r pattern`" without actually calling the tool are caught.
- **Add investigative intent phrases to phantom detector** — "Let me check…",
  "Let me see…", and similar phrases now trigger the detector.
- **Phantom detector lets loops slide after one retry** — prevents infinite
  retry loops when the model consistently fails to produce tool calls.
- **Preserve tool group on Escape-twice** — cancelling no longer loses the
  visible tool call history.
- **Catch phantom "Let me see:" mid-turn** — the detector catches this
  specific pattern that was slipping through.

#### TUI
- **Per-session message queue with isolated display** — queue messages no
  longer live in the shared input buffer; each session has its own queue.
- **Queue preview was invisible** — height was 1 but the border consumed the
  only row, leaving zero rows for text. Fixed to height 2.
- **Wrap system messages to terminal width** — long RSI summaries and
  compaction notices no longer clip at the right edge.
- **Reduce truncation warning false positives** — responses ending with `:`
  that contain multiple sentences no longer trigger the warning.
- **Persist `last_session` on create_new_session** — restarting now resumes
  the correct session instead of jumping to a different one.
- **Drop ".." from file picker when searching** — Enter picks the top match
  instead of navigating to the parent directory.
- **Keep streaming text + reasoning visible after Escape-twice cancel** —
  cancelled content is preserved in the display instead of vanishing.
- **Mid-sentence truncation retry** — responses that appear truncated
  mid-sentence are retried automatically.
- **Stop reload-on-cancel wiping TUI push** — cancelling no longer triggers
  a full session reload that wipes queued messages.
- **Drop redundant closure around `strip_llm_artifacts`** — minor cleanup
  that was allocating unnecessarily.
- **Persist api_key on first save of custom provider** — custom provider
  keys are now written immediately on first entry.
- **Capture panic location in render `catch_unwind`** — TUI render panics
  now log the exact file and line for easier debugging.
- **Attach opencrabs-frame backtrace to render panic log** — panic traces
  include the custom frame for better error reports.
- **Dialogs.rs panicked indexing PROVIDERS** — fixed crash after Qwen CLI
  provider removal shifted the provider list.
- **Merge consecutive reasoning-only messages in IntermediateText** —
  multiple reasoning blocks between tool calls collapse into one display.
- **Stop tools-v2 JSON bleeding into chat** — `<!-- tools-v2: -->` markers
  with rustc arrow formatting no longer appear as visible text.
- **Route ```think...``` content to ReasoningDelta** — backtick-wrapped
  thinking content renders in the thinking section and persists correctly.

#### Context / Token Counting
- **Persist server `input_tokens` on messages** — server-reported token
  counts are stored on each message instead of relying on in-memory cache.
- **tiktoken fallback was missing system prompt** — counted only messages +
  tools, underestimating context by ~2-4K tokens.

#### Channels
- **Stop silently wiping final response via bad dedup** — the dedup logic
  was matching too aggressively and removing the agent's actual response.
- **Revert 'never strip to empty' guard** — it caused duplicate messages
  by preventing legitimate dedup.
- **Delete stale intermediates on cancelled in-flight call** — Telegram
  no longer leaves orphaned edit messages after cancellation.

#### Qwen / DashScope (migration fixes)
- **401/403 now trigger account rotation** — auth errors cause advancement
  to the next credential instead of immediate failure (applies to remaining
  rotation paths during migration).
- **Invalidate dead OAuth accounts** — when both refresh and retry fail,
  credentials are cleared and persisted so re-authentication is triggered.
- **Tighten auth-invalidate trigger** — single-slot rotation writes are
  skipped to avoid wiping the only valid credential.
- **Handle 401/403 auth errors in stream rotation** — mid-stream auth
  errors now trigger rotation and retry.
- **Raise think-tag safety valve** — long Qwen reasoning blocks no longer
  get partially stripped by the tag filter.

[0.3.12]: https://github.com/adolfousier/opencrabs/compare/v0.3.11...v0.3.12
[0.3.11]: https://github.com/adolfousier/opencrabs/compare/v0.3.10...v0.3.11

## [0.3.10] - 2026-04-15

RSI reliability hardening — cycle summaries no longer truncated, phantom
detection reduced to a two-signal requirement, and compaction label
simplified in the README diagram.

### Fixed

#### RSI
- **Stop truncating cycle summary sent to TUI** — cycle summaries now
  display the full text instead of being cut off mid-sentence.
- **Reduce phantom detection false positives** — now requires two signals
  (intent keyphrase + zero tool calls) before flagging a phantom,
  eliminating spurious self-heal triggers.

#### Style
- **Cargo fmt pass** — `rsi.rs` and `helpers.rs` formatting.
- **Simplify compaction label in README diagram** — cleaner visual in
  the architecture reference.

## [0.3.9] - 2026-04-15

Usage Dashboard, config stability overhaul, and TUI polish. The headline
feature is an interactive `/usage` overlay showing cost, tokens, and
sessions broken down by project, model, activity, and tool. Config writes
were rewritten from scratch to stop the recurring config-wipe bug.

### Added

#### Usage Dashboard
- **Interactive usage overlay** — press `/usage` to open a centered TUI
  panel with five cards: Daily Activity, By Project, By Model, Core Tools,
  and By Activity. Tab navigates cards, T/W/M/A filters by time period.
- **Tool execution recording** — every tool call is now logged to a
  `tool_executions` table for per-tool usage analytics in the dashboard.
- **Session auto-categorization** — heuristic classifier tags sessions as
  Development, CI/Deploy, Testing, etc. based on title and tool patterns.
  Categories feed the By Activity card with cost/turns/1-shot% breakdown.
- **Project & model breakdowns** — By Project shows cost, tokens, and
  session count per workspace. By Model shows cost and tokens with display
  labels and estimated-cost markers for free-tier models.

#### TUI
- **Mouse drag-select text copy** — click and drag in the input area to
  select text, automatically copied to clipboard on release.

### Fixed

#### Config Stability
- **Stopped `migrate_if_needed` from wiping `config.toml`** — the migration
  function was overwriting the file on every startup with default values.
- **Replaced toml round-trip with `toml_edit`** — all config writes now use
  lossless parsing so comments, ordering, and unknown keys are preserved.
- **Eliminated all `unwrap_or(empty_table)` write paths** — these were
  silently replacing corrupt or partially-loaded config with blank tables.
- **Mutex lock on config writes** — prevents concurrent writes from racing
  and producing empty files.
- **API key recovery from last-good snapshot** — when `keys.toml` is
  corrupt, recovers credentials from the most recent valid backup.
- **Qwen OAuth credentials passed directly** — OAuth tokens flow through
  the event chain instead of being re-read from a potentially-wiped config.

#### Qwen
- **Enforce `gitCoAuthor=false` before every CLI spawn** — patches
  `~/.qwen/settings.json` programmatically so `Co-authored-by` trailers
  never appear, even after config resets. System prompt rule stays as
  belt-and-suspenders.
- **Detect expired rotation accounts** — Alt+Backspace wipes stale OAuth
  credentials for the current account.

#### Usage Dashboard Polish
- **Right-aligned data columns** — cost, tokens, sessions, and 1-shot%
  columns are pushed flush to the right card edge across all cards.
- **Adaptive card widths** — columns auto-size to actual data width instead
  of using fixed-width assumptions.
- **Real 1-shot% calculation** — replaced fake binary approximation with
  actual per-session turn counting.
- **Half-block bar charts** — visual separation between bars and data.

#### Self-Heal
- **Phantom detection on single intent keyphrase** — catches when the model
  produces a single tool-intent phrase with zero actual tool calls.

#### TUI
- **Prevent impossible provider/model combos** — status bar no longer shows
  model names that don't belong to the active provider.
- **Brain generation background mode** — enters chat immediately while brain
  files generate asynchronously. 120s timeout prevents infinite hangs.
- **Stricter brain prompt** — forbids preamble, closing remarks, and code
  fences so parsing succeeds reliably.
- **Voice step Continue button** — Tab cycles between fields instead of
  advancing screens. Added Continue button matching Channels step.
- **Brain textarea wrapping** — first backspace on untouched template clears
  the field. Large pastes render cleanly with line-wrap.

#### Database
- **Soft-delete sessions** — preserves metadata for usage tracking instead
  of hard-deleting.
- **Strip ANSI codes from tool output** — prevents escape sequences from
  polluting DB persistence.
- **Recreated `tool_executions` schema** — migration fixes column types for
  correct recording.

#### CI
- **Windows CRT static linking** — `LLAMA_STATIC_CRT=1`, `RUSTFLAGS`,
  and `CFLAGS=/MT` ensure all objects use static CRT.
- **Removed Windows CRT overrides** — cleaned up after the fix landed.

### Docs
- **Terminal permissions setup** — macOS, Windows, Linux instructions.
- **Session auto-categorization** documented in README.
- **Python runtime dependency for local TTS** added to install docs.
- **System commands section** — documented OS-specific terminal capabilities.

## [0.3.8] - 2026-04-14

25+ hardening fixes across context tracking, self-heal, provider rotation,
and cross-platform support. RSI moves to fully autonomous mode with restart
resilience and skip-on-unchanged.
Existing users should ask their crab to compare brain files with the
latest templates and apply any diffs.

> **RSI is experimental.** Autonomous self-improvement runs without human
> approval and writes improvements to `~/.opencrabs/rsi/improvements.md`.
> Monitor the file during early testing.

### Changed

- **RSI runs autonomously** — removed human-in-the-loop approval. The
  `self_improve` tool now applies improvements directly and logs them
  to `~/.opencrabs/rsi/improvements.md`. The `propose` action was removed;
  only `apply`, `update`, and `read` remain.
- **RSI uses active provider** — no longer hardcoded to Anthropic; respects
  the current provider and model configuration.
- **RSI reuses persistent session** — one session per cycle instead of
  creating a new one on every improvement run.
- **RSI survives app restarts** — persists `last_cycle` timestamp to disk;
  calculates remaining delay on startup instead of resetting the 1h timer.
- **RSI skips unchanged feedback** — if feedback count hasn't changed since
  last cycle, skips analysis entirely to avoid wasted LLM calls.
- **RSI enriched opportunities** — opportunity descriptions now include
  session ID, model name, provider, and timestamps so the agent knows
  which model/session produced failures.

### Fixed

#### Context & Token Accuracy
- **Real token counts from DB** — replaced estimation with actual values
  stored per request.
- **Reverted cumulative DB token_count** — discovered values were API
  lifetime totals, not per-request; stopped using them for context budget.
- **Silent emergency truncation** — when context exceeds budget, truncates
  to 80% and triggers auto-compaction instead of crashing.
- **Accurate token display after compaction** — context token counter now
  reads from the compaction point, not the full session history. Fixed
  calibration drift that made the counter diverge over time.
- **TUI shows compaction-aware token count** — context display reflects
  tokens since last compaction, not the cumulative total.

#### Self-Heal & Phantom Detection
- **Phantom tool call detection without file paths** — catches when the
  model narrates file changes in prose (e.g. "I updated the config") without
  actually executing any tool calls.
- **Structural phantom detection via imperative statement count** — detects
  hallucinated tool usage by analyzing imperative verb density in responses.
- **Past-tense standalone detection** — catches "Amended.\nCommitted.\n..."
  style phantom narration where the model claims completed actions.
- **Expanded completion claim vocabulary** — git-specific patterns like
  "amended the commit", "bumped the version", and "I've made the changes"
  now trigger phantom retry.
- **Rotation continuation prompt** — when Qwen OAuth rotation happens
  mid-task and the new account returns 0 tool calls, injects a continuation
  prompt so the agent resumes where it left off.

#### Provider & Rotation
- **Fallback walks entire provider chain** — on failure, iterates all
  configured providers while skipping the one that just failed.
- **Rotation token persistence** — saved between sessions so model rotation
  resumes correctly after restart. Fixed model remap and resume contamination.
- **Custom provider API key written before config reload** — prevents the
  key from being lost on `/models` switch or config refresh.

#### Qwen
- **Auto-retry device flow on failure** — Qwen OAuth device flow now retries
  automatically; pressing Enter restarts with a fresh auth code.
- **Normalized Qwen 3.6 Plus variants** — `:free`, `thinking`, and bare
  variants all resolve correctly.

#### TUI
- **Fixed panic on multi-byte characters** — mouse sequence detection no
  longer crashes on Unicode input. Fixed path truncation for wide chars.
- **Voice step Continue button** — Tab now cycles between fields instead
  of advancing to the next onboarding screen. Added Continue button
  matching the Channels step pattern.
- **Brain textarea wrapping + template wipe** — first backspace on untouched
  template clears the entire field. Large markdown pastes render cleanly
  with line-wrap and overflow indicator.
- **Brain generation runs in background** — no longer blocks on "Cooking
  up brain files..." screen. Enters chat immediately, generates in the
  background, writes files directly to workspace. 120s timeout prevents
  infinite hangs.
- **Stricter brain prompt** — explicitly forbids preamble, closing remarks,
  and code fences so parsing succeeds reliably.
- **Rebuild provider before brain gen on fresh install** — prevents
  PlaceholderProvider error when brain step fires before provider is wired.

#### Usage
- **Normalized model names and display labels in `/usage`** — consistent
  naming across providers with human-readable labels.

#### Windows
- **`where.exe` for CLI binary detection** — uses Windows-native `where.exe`
  instead of `which` when running on Windows.
- **Restored `--all-features` for Windows CI builds** — full feature set
  tested in CI again.
- **Fixed MSVC CRT mismatch** — triple-layer fix: `LLAMA_STATIC_CRT=1`
  (cmake path), `RUSTFLAGS=-Ctarget-feature=+crt-static` (Rust linker),
  and `CFLAGS=/MT CXXFLAGS=/MT` (direct MSVC compiler flags). Ensures
  all C/C++ objects use static CRT matching esaxx-rs.

### Docs

- **Python runtime dependency for local TTS** — added install commands
  for all platforms (apt, dnf, brew, winget) in README.

### Contributors

- **Teo Gonzalez Collazo** — PR #71 (Exa search integration)
- **Swoorup** — Issue #72 (readline keyboard shortcuts)

## [0.3.7] - 2026-04-13

OpenRouter streaming fully fixed — all models now use the standard OpenAI
SSE parser with full reasoning, tool calls, and think-tag support.
Qwen rotation hardened against wipe storms and expired accounts.
RSI agent now reads before writing and can surgically update brain files.

### Added

#### Streaming
- **Non-streaming compatibility module** — dedicated `nonstream_compat.rs`
  synthesizes full stream events (reasoning, tool calls, usage with cache)
  from non-streaming JSON responses. Handles OpenRouter upstreams like
  Trinity and Venice that return `chat.completion` blobs instead of SSE.
- **Stream drop exhaustion fallback** — when a provider stream drops
  mid-response, triggers fallback to the next provider in the chain.

#### RSI (Recursive Self-Improvement)
- **`read` action** — RSI agent must read brain files before modifying,
  preventing blind appends and redundancy.
- **`update` action** — surgical find-and-replace on specific sections
  of brain files instead of always appending.
- **Mandatory read-before-write workflow** — RSI prompt enforces:
  read → decide (apply new / update existing / skip redundant) → act.
- **Structured awareness** — RSI routes improvements to the correct
  brain file based on event type taxonomy.
- **Phantom tool call detection** — self-heal layer detects when the
  model narrates file changes in prose without executing tools, and
  auto-retries with a corrective prompt.
- **Proactive MEMORY.md hints** — system prompt nudges the agent to
  persist important context to memory.

### Changed

- **OpenRouter uses standard OpenAI SSE parser** — removed the
  Anthropic-format bypass that broke streaming, reasoning, and tool
  calls for all OpenRouter models. Every model now gets full feature
  support (reasoning_content, tool_call accumulation, think-tag
  filtering, leak detection).
- **Context budget** — stopped subtracting tool overhead from context
  budget, which was causing premature compaction.

### Fixed

- **OpenRouter 429 retry** — exponential backoff before falling to
  next provider, instead of immediately failing.
- **Stream content ordering** — emit content delta before finish_reason
  to prevent truncated final tokens.
- **Qwen rotation wipe storm** — concurrent refresh failures no longer
  cascade-wipe all accounts from keys.toml.
- **Qwen expired rotation accounts** — preserved for re-auth instead
  of being silently dropped.
- **Qwen account split** — metadata in config.toml, secrets in
  keys.toml, fixing #74 and #75.
- **Qwen rotation retry** — disabled retry-on-rate-limit inside
  RotatingQwenProvider to prevent double-retry with outer fallback.
- **TUI footer model** — shows actual provider/model after fallback
  swap instead of stale primary (#73).
- **Runtime deps docs** — added libgomp + libasound to pre-built
  binary requirements.

## [0.3.6] - 2026-04-13

The self-improving release. OpenCrabs now records every tool execution,
provider error, and user correction to a persistent feedback ledger,
analyzes patterns at session start, and can autonomously rewrite its own
brain files — no human approval needed. Plus Qwen OAuth hardening,
readline keyboard shortcuts, responsive table rendering, and Exa search.

Contributors:
- Teo Gonzalez Collazo (PR #71 — Exa search tool update)
- Swoorup (Issue #72 — readline shortcuts)

### Added

#### Recursive Self-Improvement (RSI) — Experimental
- **Feedback ledger** — persistent SQLite table recording every tool
  success/failure, provider error, user correction, and context compaction
  event. Zero overhead — fires and forgets inside the tool loop.
- **Startup digest** — on session open, the last 50 feedback events are
  injected into the system prompt as "Performance History", surfacing
  tool failure rates, recent errors, and user correction counts.
- **`feedback_record` tool** — agent can manually log observations
  (patterns, strategies, user corrections).
- **`feedback_analyze` tool** — query aggregated stats: per-tool success
  rates, recent failures, failure patterns.
- **`self_improve` tool** — autonomously append improvements to brain
  files (SOUL.md, AGENTS.md, TOOLS.md, etc.) and log to
  `~/.opencrabs/rsi/improvements.md`. No human approval required —
  the agent identifies patterns and applies fixes directly.
- **User correction detection** — short negative messages ("wrong",
  "no", "try again", "you broke", "fix it") are auto-recorded as
  `user_correction` events, training the ledger without explicit action.
- **Proactive compaction recording** — auto-compaction events at 65%
  threshold are logged to the ledger.

#### TUI
- **Readline keyboard shortcuts** — Ctrl+P (history up), Ctrl+N
  (history down), Ctrl+A (beginning of line), Ctrl+E (end of line).
  Thanks to Swoorup (Issue #72).
- **Responsive table rendering** — markdown tables now adapt to chat
  width instead of clipping at the terminal edge.
- **Onboarding/dialogs responsive** — dialogs scale to terminal size,
  file picker search added, session directory navigation improved.
- **Eliminated window blinking** on terminal resize.
- **Expired Qwen rotation accounts** show as red in the provider
  selector instead of green.

#### Tools
- **Exa search updated** — new search types (fast, deep-lite, deep,
  deep-reasoning, instant) aligned with latest Exa API. Added
  `x-exa-integration: opencrabs` header. Thanks to Teo Gonzalez Collazo
  (PR #71).

### Changed

- **RSI is experimental** — enable via `~/.opencrabs/config.toml`
  under `[agent]` section. The system self-improves autonomously by
  default, but users should monitor `~/.opencrabs/rsi/improvements.md`
  for changes. Existing users should ask their crab to compare
  `~/.opencrabs/TOOLS.md` with the template and update any changes.
- **Qwen OAuth retry backoff** — changed from 500ms hammer to 3s→6s→
  12s→24s exponential backoff, letting Qwen's free tier recover instead
  of burning quota on instant retries.

### Fixed

- **Qwen OAuth token validation** — validates tokens at startup, falls
  back to next provider on 401/403.
- **Async factory** — `create_provider()` is now fully async, removing
  `block_in_place` calls and fixing potential deadlocks.
- **Ghost custom providers** — cleaned up stale provider entries from
  config, and fixed API key paste handling in the provider selector.
- **Window blinking on resize** — eliminated the flash that occurred
  when the terminal was resized.

## [0.3.5] - 2026-04-12

### Fixed

- **Windows CLI providers restored** — v0.3.4 erroneously hid Claude CLI,
  OpenCode CLI, and Qwen CLI from the provider selector on Windows. These
  providers work correctly on Windows and are now visible again on all
  platforms.

## [0.3.4] - 2026-04-11

Major feature release: **Qwen multi-account OAuth rotation** — configure 2–10
Qwen accounts that rotate automatically on rate-limit, giving effectively N×
the free-tier quota. Also ships critical TUI stability and performance fixes,
a visual redesign of the provider selector, and cross-platform improvements.

### Added

- **Qwen multi-account rotation** — new `RotatingQwenProvider` round-robins
  across N OAuth accounts, auto-advancing on 429 / rate-limit errors. Setup
  via `/models` or `/onboard`: toggle rotation with `Space`, pick 2–10
  accounts (`1` = 10), authenticate each via device flow. Credentials persist
  in `keys.toml` under `[[providers.qwen_accounts]]`. With 3 accounts you get
  **180 req/min** and **3,000 req/day** before fallback kicks in (`e1b2417`,
  `a104290`).
- **Incremental rotation setup** — adding accounts later (e.g. 3→5) only
  authenticates the new ones, preserving existing credentials (`5200146`).
- **Provider credential indicators** — providers with existing API keys, OAuth
  tokens, or CLI binaries in PATH now display in green with a **✓** checkmark
  in both `/models` and `/onboard` provider lists, so you immediately see
  what's already configured (`471ae0a`).

### Fixed

- **TUI garbled display from channel sessions** — two root causes fixed:
  (1) `suppress_stdio()` in the embedding engine did process-wide
  `dup2(devnull, stdout)` which raced with ratatui's terminal writes on fd 1,
  producing partial escape sequences → garbled display. Now skips stdout
  redirection when TUI is active (stderr-only suppression). (2)
  `ChannelProcessingFinished` called `load_session().await` directly, blocking
  the event loop with a DB query and eating queued terminal events. Now
  schedules a debounced refresh instead. Earlier fix (`33f9ab6`) added the
  `ChannelSessionEvent` lifecycle with 500ms debounce for `SessionUpdated`
  events (`33f9ab6`, `5c349d4`).
- **TUI streaming lag** — every `ResponseChunk` forced a full re-render with
  O(n²) markdown re-parsing on the entire growing buffer. Now batches chunks
  within a 30ms time budget and caches parsed markdown between frames. Typing,
  scrolling, and Ctrl+C work normally during streaming (`9098e02`).
- **Context counter not updating after compaction** — `TokenCount` event now
  emitted after both LLM tier-1 compaction and tier-2 hard truncation so the
  footer percentage reflects post-compaction values in real time (`98dc401`).
- **Qwen rotation auth wipe** — opening `/models` with existing rotation
  accounts no longer re-triggers the full OAuth flow and destroys credentials.
  Guard checks persisted account count matches desired count (`8e8bb04`,
  `5200146`).
- **Duplicate messages and truncated response self-heal** — dedup logic
  prevents double-send and rotation state now persists across sessions
  (`de5a040`).
- **Custom provider duplicate entries** — entering a name with capital letters
  (e.g. "Dialagram") no longer creates two entries (one normalized, one empty).
  Names are normalized via `normalize_toml_key()` on Enter in both `/models`
  and `/onboard` (`5200146`).
- **Onboarding missing defaults** — `approval_policy = "auto-always"` and
  `enable_thinking = true` (Qwen) are now persisted to `config.toml` during
  onboarding so they're visible and editable on fresh installs (`c5c326c`).
- **CLI health check** — `qwen-code-cli` binary detection now works correctly
  in the onboarding health check (was falling through to "opencode") (`c5c326c`).

### Changed

- **Shared provider module** — all Qwen rotation logic (state loading, auth
  guards, incremental flow, count-change handling), custom name normalization,
  and credential detection consolidated into `ProviderSelectorState` methods.
  `/models` and `/onboard` now share one code path instead of diverging
  independently (`5200146`).
- **Windows provider list** — CLI providers (claude-cli, opencode-cli,
  qwen-code-cli) are hidden from the provider selector on Windows where they
  cannot work (`5200146`).
- **Windows release build** — now uses `--no-default-features` to exclude
  local-stt/local-tts, matching the CI test matrix.

## [0.3.3] - 2026-04-09

Focused fix release: **Qwen OAuth time-to-first-token drops from 60-70s to
8-15s** by matching the exact retry parameters the Qwen Code CLI uses
internally. Also ships browser auto-screenshot and a couple of UX fixes.

### Fixed

- **Qwen OAuth retry timing** — `qwen_cli_match()` now mirrors the actual
  OpenAI Node SDK defaults used by qwen-code-cli v0.14: 500ms initial delay,
  8s max delay, 3 retries, 25% jitter (was 1.5s/30s/7/30%). Unparseable
  Retry-After headers no longer default to 60s — exponential backoff decides
  instead. Combined effect: TTFB **60-70s → 8-15s** (`e42d46a`).
- **TUI execute_code collapsed card** — now shows the actual code content
  instead of generic "Execute bash" text (`34c4b39`).

### Added

- **Browser auto-screenshot** — `navigate`, `click`, and `type_text` browser
  tools now automatically capture a PNG screenshot after each action and
  attach it as a vision image alongside the tool result. The model can "see"
  the page without a separate `screenshot` call (`1ac8f91`).

### Changed

- **CI release workflow** — release jobs now poll for CI to pass before
  starting builds, preventing releases from broken commits (`182a005`).

## [0.3.2] - 2026-04-08

Big release focused on **making `providers.custom.*` a first-class citizen** —
dozens of OpenAI-compatible backends (dialagram, nvidia, z.ai, lm-studio,
ollama, vllm, any self-hosted endpoint) now stream thinking tokens, tool
calls, and intermediate text to the TUI exactly like the native providers.
Also ships the **native Qwen OAuth provider** with opt-in Qwen3 hybrid
thinking mode and a full round of prompt caching across Anthropic /
OpenRouter / Gemini / Qwen DashScope.

Post-release work adds a **brand new TUI startup experience** (splash screen
replaced by an animated header card overlaying the chat), drag-to-select
text, panic recovery, gaslighting refusal stripping, and a dozen input/render
fixes.

> ⚠️ **Known limitations**
>
> - **Qwen OAuth** still hits upstream rate limits intermittently even with
>   the global singleton limiter and per-model fingerprint headers. The
>   free 1k-req/day tier is fragile — expect occasional 429s that the
>   sticky-fallback chain will swap out of. **Qwen Code CLI** works great
>   today as an alternative, but inherits the CLI's own tool surface and
>   memory model (no native OpenCrabs tool bridge, separate scratch state).
> - **Custom OpenAI-compatible routers that rewrite the stream** (e.g.
>   dialagram's `qwen-3.6-plus-thinking`) can inject sanitized boilerplate
>   reasoning that doesn't reflect the real model's thinking. Not an
>   OpenCrabs bug — prefer native Qwen OAuth, OpenRouter, or Alibaba
>   DashScope for Qwen3.

### Added

#### Native Qwen OAuth provider
- **OAuth device-code flow** — Sign in with a Qwen account directly from
  onboarding (`/onboard`) or the runtime model switcher (`/models`); the
  wizard and model dialog now share a single device-flow state machine
  (`d16a88d`).
- **Qwen3 hybrid thinking toggle** — Qwen3 is a single-weights hybrid
  model that runs in thinking or non-thinking mode depending on a runtime
  flag. Set `enable_thinking = true` in `[providers.qwen]` to opt in when
  you want reasoning tokens; leave unset for fast output. Only the native
  Qwen provider honours the flag (`cf76e49`).
- **Full token lifecycle** — mtime-based credential reload, 401 → refresh
  retry, dynamic `resource_url` → base URL switching, background refresh
  well before expiry (`a306e22`).
- **Prompt caching** via `cache_control` on the last tool definition and
  session-stable metadata headers — keeps the free tier viable for longer
  conversations (`0aa65dc`).
- **Request body parity with `qwen-cli`** — body transform rewrites user /
  system messages to array form, tags only the last tool with cache
  control, adds the required VL flag and metadata, preserves existing
  `max_tokens`, and strips disallowed fields. Tested against a real
  `qwen-cli` HTTP capture (`b2e2c50`, `ee9b1c7`).
- **Gateway fingerprint headers** — `x-stainless-*` SDK identity, stable
  session id, 13-hex prompt id, and `Accept` header matching the official
  client so Qwen's gateway accepts requests as first-party SDK traffic
  (`11f314f`, `a7b3c35`, `c09a8bf`).
- **Global singleton rate limiter** (`QWEN_OAUTH_LIMITER`) — a single
  `LazyLock<Arc<RateLimiter>>` shared across every request regardless of
  provider rebuild / session swap, so the second request no longer
  instantly trips the per-second limit (`488480b`).
- **Sticky fallback swap UI** — the footer and session header now show the
  *active* sub-provider name + model when a rate limit forces a swap, so
  users can see which backend is actually serving them (`c736856`).

#### Prompt caching across providers
- **Anthropic native** — `cache_control` on system prompt and tool
  definitions (`a0e5f68`).
- **OpenRouter** — cache_control forwarded for Anthropic-family models
  routed through OpenRouter (`4752abf`).
- **Gemini** — full `cachedContent` API integration (`ee24e68`).
- **Qwen DashScope** — cache_control on last tool (`0aa65dc`).
- **Cache token accounting** — provider responses now parse OpenAI /
  DashScope cache + reasoning token fields, so the context window display
  reflects real usage across cache-aware providers (`d973808`).

#### TUI — header card replacing splash screen
- **Splash mode removed** — app now starts directly in chat with the last
  session loaded, so user input is available immediately (`e637e4a`).
- **Header card overlay** — animated card replaces the old splash screen,
  overlaying the top of the chat history, then vanishes like the splash
  did. Responsive layout, padding, and wrapping (`76791af`, `8368f97`).
- **Card sizing** — scales as 70% of chat area, capped and centered to
  avoid overwhelming small terminals (`1e7eb5a`, `1094832`, `5f4bc72`).
- **Full startup banner** — `opencrabs` prints the complete banner to the
  terminal on startup and exit, visible in scrollback even when the card
  vanishes (`08470bc`).
- **Help screen shows version** — `/help` now displays the current version,
  provider, and model (`fe3a49a`).
- **App title shortened** to "OpenCrabs AI Agent" for cleaner card display
  (`0e116dd`).
- **Async session load** — session messages load asynchronously on startup
  for instant first paint instead of blocking render (`66512ee`).

#### TUI — input, rendering, and interaction
- **Drag-to-select text with auto-copy** — native mouse selection in the
  TUI, auto-copies the selection on release (`40e9279`).
- **O(N) input render + scroll-to-cursor** — tall pastes no longer cause
  quadratic render cost; cursor position preserved on long inputs
  (`c05f809`).
- **Line navigation inside recalled multiline** — pressing Up/Down inside
  a multi-line history-recalled input navigates lines instead of wiping
  the whole buffer (`5f97d90`, `ee718e5`).
- **Emoji cursor rendering** — grapheme cluster extraction for the cursor
  so multi-byte emoji are highlighted correctly (`6df5774`).
- **Panic recovery** — TUI recovers from render panics and clamps title
  splits instead of crashing (`3460d1b`).
- **SoftBreak as space** — markdown soft breaks now render as a single
  space instead of a hard line break (`e81c62c`).

#### Self-healing gaslighting strip
- **Mid-turn refusal preamble strip** — reasoning models (dialagram,
  qwen-3.6-plus-thinking, etc.) that inject "I can't use tools right now"
  or similar refusal text mid-stream now have those paragraphs stripped
  before the reply reaches the user (`531f172`).
- **Per-paragraph strip + new refusal phrases** — extended phrase list
  covering tool-registry family denials, applied per-paragraph so partial
  refusals don't nuke the entire response (`bebd951`, `df51c3c`).
- **Dialagram refusal detection mid-stream** — detects `<think>`-adjacent
  refusal patterns that fire alongside tool calls and strips them inline
  (`2d80119`).
- **Cross-chunk `tool_calls` leak detection** — detects and surgically
  strips leaked `tool_calls` JSON that some routers inject across SSE chunk
  boundaries (`ff1ff22`, `b9adfb4`).

### Changed

- **Custom providers now feel like native providers.** This was the
  headline investigation of the release — see *Fixed* below for the
  `ContentBlockStop` / reasoning fixes that make tool cards and thinking
  tokens actually reach the TUI on every OpenAI-compatible backend.
- **Per-model rate limiters** — rate limit state is now keyed by the exact
  model id instead of the provider name, so swapping models on the same
  provider no longer drags a stale limiter between them (`1238573`).
- **Zero retries on hard rate limits** — a 429 goes straight to the
  sticky-fallback swap instead of spinning retry backoffs that would burn
  quota on every sub-provider in the chain (`8d8607f`).
- **OpenRouter app-identity headers on every request** — `HTTP-Referer`
  and `X-Title` are attached for every call so OpenRouter's dashboard
  attributes usage correctly (`580a225`).
- **Anthropic OAuth removed** — Anthropic banned OAuth-based third-party
  clients; all dead OAuth code paths and docs are gone (`777d87e`).
- **CLI context management split** — `cli_handles_tools` is now separate
  from `cli_manages_context`, so providers that delegate tool execution
  to an external CLI (Qwen Code, OpenCode, Claude CLI) can still have
  OpenCrabs manage compaction and the context budget (`34b59f2`).
- **Tool path resolution centralized** — every filesystem tool now goes
  through `resolve_tool_path()`, normalising tilde expansion,
  relative-to-working-dir, and symlink handling in one place instead of
  eight copies (`9898aee`).
- **Provider base_url() trait method** — added to the provider trait and
  forwarded through `FallbackProvider`, so rate-limit toasts and swap
  notices can report the actual serving backend (`0d97ea8`).
- **Qwen DashScope wire format parity** — request body matches `qwen-cli`
  capture exactly to avoid upstream rate-limit triggers (`5d8a483`).
- **Qwen 429 retry in-place** — rate limits are retried in the same way
  `qwen-cli` does it instead of bailing to fallback immediately (`994cb6a`).
- **OpenRouter `:free` 429 retry in-place** — free-tier models on OpenRouter
  retry rate limits in-place before falling back (`c46cb81`).

### Fixed

#### Custom providers (the headline)
- **Tool cards no longer get stuck "in-flight" forever** — the OpenAI-compatible
  SSE parser never emitted `ContentBlockStop` events. `helpers.rs` fires
  `ToolStarted` / `ToolCompleted` progress events *exclusively* inside
  `ContentBlockStop`, so without them tool cards stayed visually pending
  even though `tool_loop` had already executed the tool cleanly. Every
  other provider (Anthropic, Gemini, Claude CLI, OpenCode CLI, Qwen Code)
  emits Start/Stop pairs — custom was the only outlier (`f86c0e6`).
- **Thinking / reasoning tokens now reach the TUI** — providers like
  dialagram's `qwen-3.6-plus-thinking` stream `delta.reasoning_content`
  *before* any regular text content. The parser emitted `ReasoningDelta`
  at index 0 without opening a block at that index, and `helpers.rs`'s
  range check silently dropped every reasoning chunk. The parser now
  opens a zero-length text block at index 0 on the first reasoning
  delta so `ReasoningChunk` progress events forward through to the UI
  (`f86c0e6`).
- **Intermediate text no longer vanishes mid-response** — same root
  cause; the text block now receives a matching `ContentBlockStop` before
  tool flushes and before the final `MessageDelta`, so helpers finalizes
  the block and emits `IntermediateText` events in the correct order
  (`f86c0e6`).
- **Duplicate tool lifecycle events on non-CLI providers** — `helpers.rs`
  was firing `ToolStarted` / `ToolCompleted` inside `ContentBlockStop` for
  every provider, not just CLIs. `tool_loop` already owns the full
  lifecycle for non-CLI providers, so every tool event was double-emitted
  (6 events in 85µs for 3 real tool calls observed in logs), bloating the
  TUI tool-call count and leaving phantom "Processing: <tool>" indicators
  when the premature fake completion raced the real one. The emit block
  is now gated behind `if is_cli` (`98afb47`).
- **Hallucinated `{"tool_calls":[...]}` JSON in content deltas** — some
  thinking models echo an OpenAI tool_call envelope as plain text inside
  `delta.content` while *also* emitting the real tool calls through
  `delta.tool_calls`. The text version was polluting assistant messages
  in the TUI. The parser now drops content deltas that start with
  `{"tool_calls"` while still processing `reasoning_content` and
  `finish_reason` on the same chunk (`45181df`).
- **Mid-stream quota / rate-limit errors are handled** — DashScope / Qwen
  / OpenAI all emit `{error: {message, type, code}}` shapes inline in an
  otherwise-200 SSE stream when quota is hit mid-flight. The parser now
  recognises these, maps 429 / "rate limit" / "quota" wording to
  `RateLimitExceeded`, and triggers the sticky-fallback swap instead of
  silently dropping the stream (`3f175a2`).
- **Provider-configured context window honored in the TUI header** — custom
  providers that set `providers.custom.<name>.context_window = 262144`
  were showing "202k/200k" in the ctx display because the header read
  the static `agent.context_limit` fallback while the enforcer used the
  real provider window. `context_window_for_model()` now delegates to
  `context_limit()` so both agree (`6129a88`).
- **Panic-free `open_tag_prefix_len`** — replaced a `chars().take()` that
  could underflow on short strings with `char_indices()` for byte-safe
  slicing (`091487d`).
- **Strip-tag opens split across SSE chunks** — the gaslighting detector
  now handles `<strip>` or similar markers that arrive mid-chunk across
  stream boundaries (`b9adfb4`).
- **Qwen HTTP 529 classified as retryable** — `overloaded_error` (529)
  from Qwen is now mapped to `StreamError` for automatic retry
  (`b29d9f6`).

#### Tool layer
- **`execute_code` now returns actionable error messages** — serde's
  cryptic `missing field 'code'` was being picked up by reasoning models
  as evidence that the whole tool layer was broken, seeding feedback
  loops where the model kept claiming tools were failing regardless of
  real output. The tool now pre-checks required fields and returns
  hints that name the tool + expected shape so the model can correct
  its next call instead of giving up (`cbe9081`).

#### Qwen native
- **`keys.toml` no longer accumulates non-secret config fields** — the
  background token refresher was writing `enabled = true` and
  `default_model = "coder-model"` to `[providers.qwen]` on every refresh,
  causing config drift and overwriting user-edited defaults. Only OAuth
  credentials live in `keys.toml` now, and existing installs self-heal
  on next refresh (`9bb5a80`).
- **Coder model id reverted** — OAuth flow uses the canonical
  `coder-model` id after an earlier mis-rename (`fcdd6af`).
- **Re-authed credentials sync into the running session** — the footer
  updates and the active sub-provider swap reflects the new credentials
  immediately instead of on next restart (`c736856`).

#### Qwen Code CLI
- **Real-time DB persistence of text + tool markers** — intermediate
  streaming state is now written to the session DB as it arrives, so
  resumes never lose partial turns (`0bb166c`).
- **Per-round usage drives ctx %** — the CLI envelope's cumulative
  usage field was inflating the ctx percentage across rounds; now uses
  the per-round delta (`6070960`).

#### Auto-approve propagation
- **`approval_policy = "auto-always"` now actually reaches `tool_loop`** —
  the TUI silently approved in its callback but `tool_loop.auto_approve_tools`
  stayed `false`, and `context.rs` would inject "AUTO-APPROVE OFF — tool
  approval is REQUIRED" into the compaction system prompt, which the LLM
  then echoed back. The flag is now propagated at `AgentService::new()`
  and again on `/approve` toggle via `rebuild_agent_service()` (`90748af`).

#### Agent / image handling
- **Image paths preserved for non-vision models** — when a model doesn't
  support vision, the image path text is now preserved in the user message
  instead of being silently dropped, so the model at least knows a file
  was referenced (`330c544`).

#### Onboarding
- **Brain onboarding auto-format + preview flow** — the wizard now
  auto-formats brain files and shows a preview before writing, catching
  mis-pasted or malformed content before it lands in `~/.opencrabs`
  (`69c87e2`).
- **Telegram test message delivery** — the onboarding Telegram test
  actually sends the message now, and the brain normalize hook runs on
  the delivered content so it matches what the wizard previewed
  (`5634c51`).
- **hf-hub stderr progress bar suppressed** — the initial embedding model
  download no longer dumps a raw hf-hub progress bar over the TUI
  (`29893b5`).

#### Channel fixes
- **New sessions inherit provider from most recent session** —
  channel-created sessions now start with the same provider/model as the
  latest active session instead of falling back to default (`fad8272`).
- **Discord stops defaulting to LM Studio on missing base_url** — custom
  providers without a configured base URL no longer silently fall back to
  LM Studio on Discord (`6819373`).

#### TUI
- **Don't wipe streamed reply when `response.content` is empty** — some
  providers return an empty `content` array on the final complete
  envelope after streaming; the TUI was overwriting the already-streamed
  reply with nothing (`3900c31`).
- **Full scrollback preserved during compaction** — compaction no longer
  truncates the visible scrollback buffer, only the LLM's in-context
  history (`bf6f5f9`).

## [0.3.1] - 2026-04-07
## [0.3.1] - 2026-04-07

### Added
- **F12 mouse capture toggle** — Toggle mouse capture on/off for native terminal text selection without exiting the TUI
- **Bang operator (`!cmd`)** — Run shell commands directly from the TUI input without an LLM round-trip; output shown as a system message (#58)
- **Auto-update on startup** — New `[agent] auto_update` config flag (default `true`) silently installs new releases on startup and hot-restarts; set to `false` to keep the prompt dialog (#59)

### Changed
- **`/evolve` is now programmatic** — Runs the EvolveTool directly instead of going through the LLM, so it can no longer be dropped or refused by a provider (#59)

### Fixed
- **`/new` slash command** — Wire up session creation via `/new` from within the TUI
- **Phantom Thinking label** — Hide Thinking label when reasoning content is empty
- **Compaction marker collapse** — Collapse stored compaction markers into system notices on reload; hide entirely when not applicable
- **OpenCode CLI live tool/text interleaving** — Proper interleaving of tool calls and text responses with footer deduplication
- **OpenCode CLI mid-stream text block dedup** — Clear text blocks on mid-stream flush to prevent duplication
- **OpenCode CLI dropped requests** — Use allow-list of terminal `step_finish` reasons (`stop`/`end_turn`/`max_tokens`); previously an `unknown` reason would prematurely terminate the response
- **Resume provider race fix** — Use `session.model` instead of provider default on resume to avoid wrong model selection
- **Onboarding brain generation** — Run brain file generation as a background task instead of blocking the UI
- **Onboarding Home Base tab cycling** — Tab key now cycles between path and seed inputs
- **Whisper download stderr suppression** — Suppress hf-hub progress bar noise during local whisper model download
- **Sanitize Unicode panic** — Eliminate `to_lowercase()` Unicode-expansion panic in `redact_secrets` and `redact_command` (#61)
- **API streaming think block** — Revert THINK_BLOCK buffer and strip echoed markers from API context
- **Rate-limit handling** — Drop pacer, relax retries to 10/20/30s with exponential backoff for 429s, name fallback in toast
- **Resume provider restore** — Restore session's own provider before running resume to avoid wrong provider selection
- **Message queue on cancel** — Clear message queue on cancel to prevent duplicate user messages

## [0.3.0] - 2026-04-07

### Added
- **Qwen Code CLI provider (full TUI)** — Complete integration with model fetch, CLI
  detection, display name, streaming, tool_use forwarding, and 1k free req/day via
  Qwen OAuth. Supports qwen3-coder-plus, qwen3.5-plus, qwen3.6-plus, and more.
  Install: `brew install qwen-code` or `npm install -g @qwen-code/qwen-code`
- **SIGINT handler + panic hook** — Proper terminal restoration on crash or Ctrl+C
  (no more garbled terminal after interrupt)
- **Mid-stream decode retry** — 3x backoff retry before provider fallback, reducing
  transient stream errors
- **Named provider index constants** — Replaced hardcoded 9/10/11 throughout TUI
  dialogs for maintainability

### Fixed
- **Qwen tool_use persistence** — Forward tool_use blocks so they get persisted and
  rendered correctly in session history
- **Qwen stop_reason looping** — Default missing stop_reason to EndTurn to prevent
  response loops
- **Qwen spawn args + display name** — Correct process arguments, tool mapping, and
  suppress internal tool blocks
- **Qwen static models** — Never invoke qwen subprocess for model list (uses hardcoded
  supported_models)
- **Active provider vision** — Now uses actual active provider instead of iterating
  priority order (was grabbing MiniMax before OpenRouter)
- **Streaming marker leak** — Stop reasoning/tool markers from leaking into visible
  response text
- **TUI input defense** — Stronger protection against mouse escape garbage and focus
  switch escape sequences
- **Telegram photo cleanup** — Extended temp file retention to 24h (images may be
  re-read hours later by analyze_image tool)
- **Custom provider context_window** — Honor custom provider's context_window override
  in compaction budget calculation
- **Subagent test isolation** — Per-thread dir override prevents cross-test pollution

### Changed
- Replaced all hardcoded provider boundary indices (9/10/11) with named constants
  throughout TUI dialogs and onboarding render
- Cargo.toml version 0.3.0
- CHANGELOG entries for 0.3.0

## [0.2.99] - 2026-04-06

### Added
- **Qwen Code CLI provider** — Full integration of Qwen Code as a CLI backend provider.
  1,000 free requests/day via Qwen OAuth, no API key required. Supports qwen3-coder-plus,
  qwen3.5-plus, qwen3.6-plus, and more. 256K context window, streaming, tools.
  Install: `npm install -g @qwen-code/qwen-code` or `brew install qwen-code`.
  Config: `[providers.qwen_code_cli] enabled = true`
- **Vision-first file processing** — All file attachments (PDFs, images) processed via
  vision model before text extraction, across TUI and all channel handlers
- **PDF-to-image rendering utility** — High-quality PDF page rendering for vision analysis
  (`pdf_vision.rs`), replaces plain text extraction with visual inspection
- **Proactive rate limiting for OpenRouter :free models** — Paces requests automatically
  to avoid account-level bans; shared global static limiter across all :free instances
- **File-based subagent progress streaming** — JSON status files (`pending`, `running`,
  `completed`, `failed`) for real-time sub-agent progress via spawn/status tools
- **Subagent provider/model defaults in config** — `[agent]` section with
  `subagent_provider`/`subagent_model`, injected into config.toml on startup

### Fixed
- **Rate limiter first-call wait** — `last_granted=0` treated as sentinel so first call
  no longer sleeps unnecessarily
- **Compaction text leak** — Residual context no longer survives compaction; cancel token
  threading fixed across session restarts
- **Vision registration priority** — `provider.vision_model` now takes priority over
  `image.vision` fallback
- **CI failures** — Binary test assertion and rate_limiter fmt/clippy errors squashed
- **Cosmetic cleanup** — Discord, utils, and test module fixes

### Refactored
- **Brain core injection** — USER.md replaces IDENTITY.md in TUI core brain; IDENTITY.md
  moved to contextual files for on-demand loading (cron/social sessions only)
- **Post-compaction brain recovery** — CODE.md injected as ~300-token best-practices
  summary instead of full 420-line file; explicit `load_brain_file("CODE.md")` directive
  before any code task

## [0.2.98] - 2026-04-05

### Added
- **Auto-fallback on rate limits** — When the primary provider hits a rate/account limit mid-stream, catches `RateLimitExceeded`, saves state, and resumes the same conversation on a fallback provider configured in `providers.fallback`
- **Fallback provider chain from config** — Reads `[providers.fallback]` at startup to build an ordered list of fallback providers. `has_fallback_provider()` and `try_get_fallback_provider()` for runtime queries
- **Telegram resume with full streaming pipeline** — Interrupted Telegram sessions now resume with typing indicator, tool status messages, edit loop, dedup, and rate-limit retry. Previously the user saw silence for minutes
- **Telegram bot commands autocompletion** — Registers all 9 slash commands (`help`, `models`, `usage`, `new`, `sessions`, `stop`, `compact`, `doctor`, `evolve`) via `setMyCommands` after bot auth. No manual BotFather setup needed

### Fixed
- **PDF text extraction** — Extract text from PDF files via `pdf_extract` instead of returning `Unsupported`
- **Context compaction runaway enforcement** — Two-tier budget enforcement: 65% soft trigger (LLM compaction with retries), 90% hard floor (forced truncation to 75%, cannot fail). Pre-truncate target now scales proportionally (85% of max_tokens) instead of hardcoded 170k, supporting custom providers with different context windows. Compaction is now silent to user — summary written to memory log only, no chat spam
- **Telegram duplicate messages** — Edit streaming message in-place instead of delete+send race; cancel guard moved before display queue to prevent stale messages after cancellation
- **Telegram dedup diagnostics** — INFO/WARN logging on the dedup path to trace exactly what's being stripped
- **TUI token counter stuck at 111K** — Removed monotonic guard so CLI-calibrated token count (~41K) reaches display instead of being blocked by the post-compaction tiktoken estimate (~111K)
- **Local timezone in logs** — Log timestamps now show local timezone with `%:z` offset instead of UTC
- **Rate limit detection in CLI errors** — Parses "rate limit", "429", "overloaded", "too many requests", "hit your limit" as `ProviderError::RateLimitExceeded`
- **Telegram resume race on bot auth** — Polls `tg.bot().await` up to 30s before calling `resume_session` to avoid the 328ms startup race

### Refactored
- **Context budget enforcement** — `enforce_context_budget()` with two-tier enforcement: 65% soft LLM compaction, 90% hard truncation floor. Safety truncation to 80% if compaction exhausts all retries. Removed CompactionSummary/Compacting progress events — compaction fully silent to user
- **Telegram resume pipeline** — Routes through `handler::resume_session()` instead of bare agent call with no streaming or feedback

### Testing
- **55 Telegram resume tests** — Cancel tokens, dedup logic, markdown-to-HTML, message splitting, pending approvals, bot wait loop, cancel guard ordering, token counter regression

[0.2.98]: https://github.com/adolfousier/opencrabs/compare/v0.2.97...v0.2.98

## [0.2.97] - 2026-04-04

### Added
- **Agent type system** — Typed subagents (`General`, `Explore`, `Plan`, `Code`, `Research`) with filtered tool registries. Each type gets a curated subset of the parent's tools, preventing recursive spawning and dangerous operations via `ALWAYS_EXCLUDED` list
- **Team orchestration** — `TeamManager` coordinates named groups of agents. New tools: `team_create` (spawn N typed agents as a named team), `team_delete` (cancel and clean up), `team_broadcast` (fan-out messages to all running agents in a team)
- **Subagent provider/model config** — `[agent]` section in config with `subagent_provider` and `subagent_model` fields. Spawned agents inherit the configured provider instead of always loading from global config
- **Subagent input loop** — `send_input` now works: spawned/resumed agents wait for input after completing a round instead of exiting. Enables multi-turn conversations with child agents

### Fixed
- **Tool call descriptions truncating instead of wrapping** — `render_tool_group` now wraps description headers and value lines to terminal width. Removed 80-char pre-truncation of bash commands in `format_tool_description`. Added `file_path`/`filePath` fallbacks for file-related tools
- **Double-escape cancel losing visible content** — Streaming response and active tool group now persisted to DB *before* `handle.abort()` fires, so cancelled content survives reload
- **Claude CLI subprocess leak on cancel** — Stream reader loop monitors `tx.closed()` via `tokio::select!` and kills the child process when the receiver is dropped
- **Telegram duplicate messages on cancel** — Added `cancel_token.is_cancelled()` guard before delivering final response, preventing stale agent results from posting after cancellation
- **Config overwriting existing channel settings** — `apply_config()` now scopes writes to only the current onboarding step. `from_config()` sets `EXISTING_KEY_SENTINEL` for all existing channel data so untouched fields are never overwritten
- **Pane switch not updating model display** — Session provider now swaps the agent to match the session's configured provider instead of overwriting the session
- **Tool input not persisted for CLI segments** — `CliSegment::Tool` now includes `"i"` field for `tool_input`, surviving DB reload

### Testing
- **84 subagent/team tests** — Manager state machine (27), SendInputTool (6), CloseAgentTool (5), WaitAgentTool (7), lifecycle (8), AgentType filtering (10+), TeamManager (10), TeamDeleteTool (4), TeamBroadcastTool (5), registry exclusion (1)
- **HTML comment strip tests aligned** — `strips_malformed_close_tag` → `preserves_malformed_close_tag`, `strips_unclosed_comment` → `preserves_unclosed_comment` to match actual (correct) behavior
- **1,772 total tests** (up from 1,687)

## [0.2.96] - 2026-04-02

### Added
- **OpenRouter reasoning support** — Send `include_reasoning: true` in requests to OpenRouter models. Thinking/reasoning output now displayed in collapsible sections for models that support it (e.g. Qwen 3.6 Plus)
- **Function calling detection** — Warn users when a model does not support tool use. Detects raw tool call JSON in text responses and appends a visible warning with model switch suggestion

### Fixed
- **Thinking/reasoning text truncation** — Reasoning content now wraps to screen width instead of truncating at the right edge. Long lines in collapsible thinking sections reflow properly on narrow terminals
- **LLM artifacts leaking to TUI** — `<!-- reasoning -->` tags, `</invoke>`, `</parameter>` XML fragments no longer rendered as plain text. `strip_llm_artifacts` applied to completed responses, intermediate text, and streaming render
- **Duplicate agent response on rebuild/evolve restart** — Agent responded twice with identical "Back online" messages because both a wake-up message and evolution message fired at startup. Merged into a single message
- **Evolution prompt leaked to user** — Internal `[SYSTEM:` instruction for evolution/rebuild was displayed as a visible user message. Now hidden from chat
- **Windows CI compilation** — `unsafe extern` for FFI blocks (Rust 2024 edition), unreachable code after platform-specific `bail!`, unused `voice_id` variable gated behind `local-tts` feature
- **browser_test example** — Gated behind `browser` feature flag so `--no-default-features` builds don't fail. Un-ignored `examples/` directory so CI has the file
- **Flaky concurrent profile test** — `ProfileRegistry::save()` now uses atomic write with file locking to prevent concurrent readers from seeing partially-written TOML

### Changed
- **`tool_choice: "auto"`** — OpenAI-compatible providers now send `tool_choice: "auto"` when tool definitions are present, enabling function calling on models that require explicit opt-in

## [0.2.95] - 2026-04-02

### Added
- **Up/Down arrow navigation for attached images** — Navigate between attached images in the input area using arrow keys. Visual indicator shows current position (e.g. "2/4"). Previously required detaching and reattaching to reorder
- **Rolling build output for /rebuild** — Build progress now shows as a single updating message with the last 6 compile lines, replacing the previous flood of 200+ individual system messages. Cleared automatically on restart
- **Rolling status quips for CLI providers** — Processing status messages now fire from the first keystroke even before tools have started, via a `processing` flag on the streaming snapshot. Previously required active tool calls to trigger
- **Multi-target cron delivery** — `deliver_to` field now supports comma-separated targets (e.g. `http://...,telegram:-12345`). Each target receives results independently. All existing cron jobs updated to deliver to both agentverse and Telegram

### Fixed
- **CLI token tracking showing near-zero usage** — `CliUsage::total_input()` returned only `input_tokens` (1-3 per message), excluding `cache_creation_input_tokens` (~80K) and `cache_read_input_tokens` (~14K). Every CLI provider message burned real API credits but reported $0.00 cost and ~6 tokens. Now includes all cache tokens in usage tracking, cost calculation, and session stats. `TokenUsage` struct gains `cache_creation_tokens`, `cache_read_tokens`, `billing_cache_creation`, and `billing_cache_read` fields separating context window tracking from billing. Cache-aware pricing (1.25x input rate for cache writes, 0.1x for cache reads)
- **Context window display showing 2.3M tokens** — CLI providers accumulated tiktoken estimates via `add_message()` without calibration, snowballing `context.token_count` to 2.3M and triggering false compaction. Now capped at model's context window. Billing tokens (cumulative across CLI tool rounds) tracked separately from context window (per-call values)
- **Per-session provider isolation** — Changing provider/model in TUI no longer changes it for Telegram/Discord/Slack sessions. Each session's provider is persisted in DB and restored on message receipt via `sync_provider_from_config`. Channel `/models` command no longer mutates global config
- **Custom provider dialog** — "+ New Custom Provider" now shows blank form fields instead of retaining values from previously loaded custom provider (e.g. nvidia). Dialog height increased to show all custom provider fields (Base URL, API Key, Model, Name, Context Window) without truncation
- **Config reload feedback loop causing silent crash** — Writing config inside a ConfigWatcher callback triggered an infinite reload cycle. Two additional triggers found and removed: `Config::write_key()` inside provider creation, and redundant `create_provider()` on every config reload event
- **Queued message ordering and Up-arrow dequeue** — Messages queued while the agent was processing could arrive out of order. Up-arrow now correctly dequeues the last queued message instead of the first
- **E2E test timeouts** — `e2e_opencode_streaming` wrapped with 30s timeout to prevent test suite hang under concurrent load. Gemini fetch tests gracefully skip on API key/network failures instead of crashing the suite

### Security
- **Trello API credentials moved from URL interpolation to query builder** — All 24 Trello client methods refactored to use `authed_get/post/put/delete` helpers that pass `key` and `token` via `reqwest::RequestBuilder::query()` instead of string interpolation. Resolves 24 CodeQL alerts
- **Gemini API key moved to request header** — API key now sent via `x-goog-api-key` header instead of URL query parameter across provider, fetch, and onboarding modules. Resolves 4 CodeQL alerts
- **Image tool API keys moved to request headers** — `analyze_image` and `generate_image` tools now pass API keys via headers instead of URL query strings. Resolves 2 CodeQL alerts
- **CI workflow permissions restricted** — Added top-level `permissions: contents: read` to `ci.yml` and `release.yml`, with explicit `contents: write` only on jobs that need it (`build-release`, `create-release`). Resolves 5 CodeQL alerts
- **Removed API key logging in tests** — Gemini fetch test no longer prints key length or prefix to stderr

### Changed
- **Brain file templates updated** — MEMORY.md template restructured as agent scratchpad for rules, corrections, and preferences. AGENTS.md template adds mandatory memory triggers. TOOLS.md template adds 15 missing tools

> **Existing users:** Your local brain files (`~/.opencrabs/*.md`) may be outdated. Ask your crab: *"Compare my brain files against the latest templates in `src/docs/reference/templates/` and append anything missing."*

### Testing
- **29 token tracking tests** — TokenUsage struct, cache-aware pricing, CLI/API flow, billable accumulation, cost regression, provider format deserialization
- **1,687 total tests** (up from 1,605)

## [0.2.94] - 2026-03-31

### Added
- **Multi-profile support** — Run multiple isolated OpenCrabs instances from a single installation. Each profile gets its own config, memory, sessions, skills, and gateway service under `~/.opencrabs/profiles/<name>/`. Create with `opencrabs profile create <name>`, switch with `opencrabs -p <name>` or `OPENCRABS_PROFILE` env var. Default profile (`~/.opencrabs/`) works exactly as before — zero breaking changes
- **Profile migration** — Copy config and brain files between profiles with `opencrabs profile migrate --from default --to hermes [--force]`. Migrates all `.md` and `.toml` files plus the `memory/` directory. Excludes DB, sessions, logs, and layout state so the target profile starts fresh with the source's personality and configuration
- **Profile export/import** — Share profiles as portable `.tar.gz` archives with `opencrabs profile export <name>` and `opencrabs profile import <path>`
- **Token-lock isolation** — PID-based lock files prevent two profiles from binding the same bot token (Telegram, Discord, Slack, Trello). Stale lock detection automatically cleans up locks from dead processes
- **Profile-aware daemon services** — `opencrabs -p hermes service install` creates profile-specific plist/systemd units (`com.opencrabs.daemon.hermes` / `opencrabs-hermes`). Multiple profile daemons can run simultaneously as separate OS services

### Fixed
- **CLI stream idle timeout too short** — CLI providers run tools internally (cargo build, cargo test, gh commands) that can take several minutes without producing stream events. The 60-second idle timeout caused premature stream termination → retry → fresh CLI session repeating all prior work. Now 10 minutes for CLI providers, 60 seconds for API providers
- **CLI token usage lost on EOF** — When Claude CLI exits without a `Result` message but accumulates token counts from `message_delta` events, the usage was silently discarded. Now flushes accumulated input/output tokens as a final `MessageDelta` + `MessageStop` on EOF
- **Service command compilation on Linux** — `_systemd_name` variable was prefixed with underscore (suppressing "unused" warning on macOS) but referenced without underscore in Linux-only code paths, causing CI build failure on ubuntu-latest
- **TUI duplicate text on streaming responses** — Streaming responses were doubled on screen due to intermediate text emission timing
- **Usage stats wrong totals** — Model name duplication in usage ledger (`opus` vs `opus-4-6`) caused inflated cost tracking. Now merges bare model names with their versioned equivalents
- **IntermediateText not firing for Telegram/channels on CLI providers** — CLI providers weren't emitting intermediate text events to channel handlers, causing silent gaps in multi-tool conversations
- **E2E test suite hanging on slow providers** — Added 30-second timeout to `e2e_opencode_streaming` test to prevent indefinite blocking under concurrent load
- **E2E tests crashing suite on API key/network failures** — Gemini and OpenCode E2E tests now gracefully skip with a warning instead of panicking when API keys are invalid or network is unreachable

### Changed
- **`opencrabs_home()` delegates to profile resolver** — All 30+ call sites automatically resolve to the active profile's directory. Logger, onboarding wizard, and brain file resolver no longer hardcode `~/.opencrabs`
- **Channel manager acquires token locks before spawning** — Telegram, Discord, Slack, and Trello channel connections check for token conflicts before starting. All locks released on TUI exit and daemon shutdown

### Testing
- **57 profile tests** — Name validation (8), token hashing (6), registry CRUD (10), path resolution (3), error messages (4), CRUD lifecycle, export/import roundtrip, token lock acquire/release/stale detection, migration with force/skip/nested dirs, isolation guarantees, daemon service argument generation
- **Usage ledger normalization tests** — Model name merging for bare vs versioned names
- **1,605 total tests**

### Docs
- **README profiles section** — Full command reference, directory structure diagram, token-lock isolation explanation, daemon service management, migration workflow
- **TESTING.md** — Updated with profile test counts and categories

## [0.2.93] - 2026-03-30

### Fixed
- **Crash recovery routes responses back to originating channel** — Previously `pending_requests` always stored `channel="tui"` and recovery only sent responses via TuiEvent. Now each channel (Telegram, Discord, Slack, WhatsApp, Trello) passes its name and `chat_id` through to `run_tool_loop`, which stores them in the DB. On restart, recovery routes responses back to the correct channel using the stored `channel_chat_id`
- **UTF-8 panics on multi-byte string truncation** — Byte-index slicing on multi-byte emoji (e.g. `🔺` at bytes 497..501) caused panics in `context.rs`, `panes.rs`, and Telegram handler. All string truncation now uses `floor_char_boundary`/`ceil_char_boundary` to land on valid UTF-8 boundaries
- **TUI responses vanishing when CLI model ends with tool calls** — Previous fix extracted only trailing text (after last tool) as the final response, but when the model ends with tool calls and no trailing text, `final_text` was empty. Reverted to extracting all text; Telegram dedup now happens in the handler by tracking sent intermediate texts and stripping them from the final response
- **TUI dropping trailing text after tool calls** — `complete_response` now updates the intermediate message instead of skipping it, ensuring text that follows tool call blocks renders correctly
- **Panic protection for Telegram message handler** — Nested `tokio::task::spawn` catches panics in the Telegram message handler instead of silently losing them

### Added
- **New DB migration: `pending_requests_channel_chat_id`** — Adds `channel` and `channel_chat_id` columns to `pending_requests` table for cross-channel crash recovery routing

### Testing
- **Crash recovery and self-healing tests for all channels** — Channel-specific pending request storage, `get_interrupted_for_channel` filtering, `delete_ids` selective deletion, multi-channel coexistence, UTF-8 safe string truncation with emoji/CJK, panic protection pattern verification
- **1,605 total tests** (up from 1,593)

## [0.2.92] - 2026-03-29

### Added
- **Self-healing config recovery** — When `config.toml` becomes corrupted or unloadable, OpenCrabs automatically restores from the last-known-good snapshot saved on every successful write. User sees a notification explaining what was recovered
- **Provider health tracking** — Per-provider success/failure history tracked in `~/.opencrabs/provider_health.json`. `/doctor` slash command shows health stats. Failed providers logged with timestamps for debugging intermittent API issues
- **DB integrity check on startup** — SQLite `PRAGMA integrity_check` runs at boot. If corruption is detected, a notification appears in TUI and all channels instead of silently failing
- **Unknown config key warnings** — Unknown top-level keys in `config.toml` now trigger a startup notification listing the unrecognized keys, catching typos like `[teelgram]` or `[a2a_gatway]`
- **Self-healing user notifications** — All self-healing events (config recovery, provider failures, integrity issues) surface as visible notifications across TUI, Telegram, Discord, Slack, and WhatsApp instead of hidden log entries

### Fixed
- **Telegram intermediate texts vanishing between tool rounds** — Messages sent during multi-tool iterations disappeared because new edits overwrote previous content. Telegram handler now maintains a persistent intermediate message stack with proper ordering
- **Telegram intermediate texts not sticking** — Follow-up fix: intermediate text messages were still being deleted prematurely during rapid tool execution. Reworked the message lifecycle to hold messages until the final response arrives
- **Duplicate final response on Telegram for CLI providers** — CLI providers return all content blocks in a single iteration. IntermediateText emitted the full text, then the final response repeated it. Now IntermediateText only emits text before the last tool block; final response only extracts text after it
- **Reasoning as fallback intermediate text** — When a CLI provider returns reasoning but no visible text between tool rounds, the reasoning content is now used as fallback intermediate text for channels instead of showing nothing
- **Non-focused panes hiding tool calls and thinking text** — `render_simple_message` skipped tool_group messages entirely, so non-focused split panes showed less content than the focused pane. Now shows compact tool call summaries and stripped reasoning text
- **Non-focused pane collapsed tool groups** — Tool groups in non-focused panes now display as single collapsed lines matching the focused pane style, with thinking indicators for reasoning blocks
- **Non-focused panes not scrolled to bottom** — Split panes that weren't focused appeared stuck at the top. Fixed scroll position calculation for inactive panes
- **Inactive split panes stale cache** — Cached render state for background panes wasn't invalidated when new messages arrived. Now clears cache on session updates
- **Tool calls showing running forever after completion** — Tool call status stayed at "running" spinner even after the tool finished. Now correctly transitions to success/failure state
- **Silently dropped errors across config, channels, and persistence** — 14 files had `let _ = ...` or `.ok()` swallowing errors in config writes, channel sends, tool connections, and pane state. All now surface errors via logging or user notifications
- **Remaining silent error drops in tools and channel handlers** — Second pass caught additional swallowed errors in Slack connect, Trello connect, slash commands, Telegram handler, and WhatsApp handler
- **Onboarding config write errors batched** — Config writes during onboarding used individual `let _ =` calls. Replaced with `try_write!` macros that batch errors and surface them at the end of each wizard step
- **Config::load() fallback-to-default** — Render, dialogs, messaging, and cron modules silently fell back to default config when load failed, masking real config issues. Now propagate errors or use the passed-in config reference
- **Custom provider name normalization** — Custom provider names with mixed case or whitespace were treated as different providers. Now normalized on both load and save
- **Case-insensitive tool input key lookup** — Tool input display descriptions used exact-case key matching, failing for providers that return keys in different casing
- **Cached state not cleaned on session delete** — Deleting a session left stale cached pane state behind. Now clears cache entries for the deleted session
- **`gateway` serde alias for A2A config** — Added `gateway` as a serde alias for the A2A config section, plus deduplication of typo warnings
- **Model selector wiping API keys on Enter** — Pressing Enter in the model selector could clear the API key for the selected provider. Now preserves existing keys
- **IntermediateText emission timing for CLI providers** — IntermediateText was emitted after clearing iteration state, losing the accumulated text. Now emits before clearing

### Changed
- **AgentService::new() requires &Config** — Constructor now takes an explicit `&Config` parameter instead of calling `Config::load()` internally. Eliminates hidden I/O, makes dependencies explicit, and enables test injection. All production callers and 11 test files updated

### Testing
- **27 self-healing system tests** — Config snapshot/restore, provider health tracking, DB integrity check, unknown key detection, notification delivery across all channels
- **All test files migrated to `AgentService::new_for_test()`** — 11 test files updated to use the new test constructor
- **1,593 total tests** (up from 1,564)

## [0.2.91] - 2026-03-29

### Added
- **Startup update prompt** — When a new version is available, a centered dialog appears on top of the splash screen asking the user to accept (Enter) or skip (Esc). Accepting triggers `/evolve` automatically; skipping returns to splash so the user sees their current version. After update, the binary restarts and splash shows the new version
- **`/doctor` channel command** — Health check now available directly on Telegram, Discord, Slack, and WhatsApp without going through the LLM. Returns provider status, channel config, voice config, and approval policy
- **Shared text command handler** — New `try_execute_text_command()` in `commands.rs` handles Help, Usage, Evolve, Doctor, and UserSystem commands in one place. All four channel handlers delegate through this shared function, eliminating duplicated command logic
- **Pane session preloading** — Restored split panes now preload their session messages from DB on startup, so pane content is visible immediately instead of blank
- **Persistent pane layout** — Split pane configuration (splits, sizes, focused pane) now saves to `~/.opencrabs/pane_layout.json` on quit and Ctrl+C, and restores on restart

### Fixed
- **UTF-8 char boundary panics** — `split_message()` in all 5 channel handlers (Telegram, Discord, Slack, WhatsApp, Trello) could panic on multi-byte characters (emojis, €, CJK). Now uses `is_char_boundary()` to find safe split points
- **Model switch errors silently swallowed** — Telegram, Discord, and Slack always showed "✅ Model switched" even when provider creation failed. Now surfaces the actual error with `⚠️` prefix
- **CLI provider ARG_MAX crash** — When OpenCode CLI conversation context exceeded OS `ARG_MAX` (~1MB on macOS), the spawn failed with "Argument list too long". The emergency compaction handler now catches this error, auto-compacts context, and retries. If compaction itself fails, falls back to hard truncation (keeps last 24 messages) with a marker telling the agent to use `search_session` for older context
- **`/evolve` hitting provider errors on channels** — `/evolve` was being routed through the LLM instead of executing directly. Now runs as a direct command on all channels (downloads and reinstalls without LLM involvement)
- **CLI tool calls lost on Esc×2 and restart** — Tool call results from CLI providers were not persisted to DB, so they vanished on double-escape cancel or process restart. Now saved alongside regular messages
- **Session not reloaded after double-escape cancel** — After cancelling with Esc×2, the session context was stale. Now reloads from DB to pick up any changes made during the cancelled operation
- **Thinking text unreadable on Telegram** — Thinking/reasoning blocks had poor formatting. Improved readability with proper styling
- **Model selector missing '↓ N more' indicator** — Long model lists didn't show a scroll indicator. Added count of hidden items below the visible list
- **Model list sorted alphabetically instead of by date** — Fetched models now sort newest-first so latest releases appear at the top
- **Pane layout lost on quit** — Split pane configuration was only in memory. Now persists to disk on quit and Ctrl+C

### Changed
- **`config.toml.example`** — Added z.ai GLM provider section with configuration examples

### Testing
- **2 emergency compaction tests** — `ArgTooLongMockProvider` and `ContextLengthMockProvider` verify the retry flow works after compaction/truncation
- **1,564 total tests** (up from 1,562)

## [0.2.90] - 2026-03-27

### Added
- **Daemon health endpoint** — New `[daemon] health_port = 8080` config option. When set, `opencrabs daemon` binds a lightweight `GET /health` endpoint returning 200 OK + JSON status. Useful for systemd watchdog, uptime monitors, and external health probes
- **Shared provider registry** — Single source of truth (`src/utils/providers.rs`) for all LLM provider metadata. TUI `/models`, `/onboard`, and channel `/models` all derive from `KNOWN_PROVIDERS` — no more hardcoded index-based match blocks that fall out of sync

### Fixed
- **Daemon mode Telegram/Discord dying silently** — Channel bots (Telegram long-polling, Discord gateway) would exit on network hiccups or token conflicts without restarting. Added retry loops with 5s backoff so daemon mode auto-reconnects instead of going unresponsive while the process stays alive
- **CLI providers missing from channel `/models`** — Claude CLI and OpenCode CLI were not listed in Telegram/Discord/Slack provider pickers because `configured_providers()` required an explicit `enabled = true` config entry. CLI providers are now always listed since they need no API key — matching TUI behavior
- **Channel providers out of sync with TUI** — Channels were missing zhipu (z.ai GLM), Claude CLI, and OpenCode CLI providers. All provider listings now derive from the shared registry

### Changed
- **CONTRIBUTING.md rewrite** — Anti-stub policy, step-by-step contribution workflows, exact CI commands, "What Gets Your PR Closed" section, and guidance for non-coders to open issues instead of submitting empty PRs

### Testing
- **10 daemon health tests** — DaemonConfig deserialization, health endpoint 200/404 responses, CLI providers always listed, API key providers gated correctly
- **1,562 total tests** (up from 1,424)

## [0.2.89] - 2026-03-27

### Added
- **Telegram rolling status quips** — During long CLI tool runs (subagents, 100+ tool rounds), Telegram now shows rotating fun messages like "☕ Grab a coffee — my sub-agents are on fire right now (42 tools, 2m 15s)". Each quip shows for 5s, vanishes, pauses 2s, then the next one appears. Auto-deletes when real streaming text arrives

### Fixed
- **OpenCode CLI permission rejection** — Non-interactive spawns auto-rejected tool calls (no TTY). Now sets `OPENCODE_PERMISSION` env var to allow all permissions including external directories
- **TUI provider mismatch after restart** — Loading a session overrode the config-enabled provider with the session's stale saved provider. Config is now authoritative — session metadata syncs to the active provider
- **Silent empty responses on stream drop** — When the provider stream dropped repeatedly, the TUI showed an empty response. Now injects a visible error message so the user knows what happened
- **OpenCode CLI tool calls not visible in TUI** — Tool call events were sent as invisible Ping instead of ContentBlock::ToolUse. Now emits proper stream events so helpers.rs fires ToolStarted/ToolCompleted progress events, restoring the expandable tool call groups
- **OpenCode CLI filesystem access** — Existing sessions locked tool execution to their original directory, blocking access to ~/Downloads/ etc. Now spawns at ~/ with explicit `--dir` flag so the sandbox covers the full user home
- **OpenCode CLI `cli_handles_tools` flag** — Was returning false, causing the tool_loop to attempt local re-execution of opencode's internal tool calls. Now correctly returns true
- **Duplicate assistant message for CLI providers** — helpers.rs flushed text as IntermediateText at stream end, then tool_loop emitted the same text again when iteration > 0. Skips the second emission for CLI providers

## [0.2.88] - 2026-03-26

### Added
- **Smart browser detection** — Auto-detect default Chromium-based browser (Chrome, Brave, Edge, Chromium) instead of hardcoded path. Feature flag docs and browser detection docs added to README

### Fixed
- **Slack/WhatsApp markdown formatting** — Messages were sent with raw markdown (`**bold**`, `~~strike~~`). New `markdown_to_mrkdwn` converter transforms to native format (`*bold*`, `~strike~`, `<url|text>`, `*Heading*`) before sending. Applied to handler response paths, streaming paths, and send tools for both Slack and WhatsApp. Discord uses standard markdown natively — no conversion needed
- **Gemini model fetching** — Multiple root causes: `GeminiModel` struct missing `#[serde(rename_all = "camelCase")]` so `supportedGenerationMethods` never deserialized; provider index 3 (Gemini) missing from `supports_model_fetch()` match; `/models` dialog passed `None` API key when navigating between providers
- **Model selector race condition** — Navigating between providers quickly caused stale async fetches to overwrite the current provider's model list (e.g. GPT models appearing under Claude CLI). `ModelSelectorModelsFetched` event now carries the provider index; handler discards results that don't match the currently selected provider
- **Model selector dialog oversized** — Dialog grew to fill the entire terminal with empty space. Height now sizes to content and caps at 75% of terminal height
- **API keys logged in plaintext** — Three locations were logging secret values: fetch entry logging, Gemini-specific logging, and `config/types.rs write_key()`. All removed — only `has_api_key=true/false` is logged now
- **CLI session ID conflicts** — Fresh session IDs per spawn for both Claude CLI and OpenCode CLI to prevent lock conflicts
- **CLI image routing** — CLI providers now route images through `analyze_image` instead of inline encoding
- **CLI error surfacing** — Error results from CLI providers are now surfaced to the user. Added Slack required scopes documentation
- **CLI cache token tracking** — Cache creation and cache read tokens excluded from context window tracking to prevent false compaction triggers

### Changed
- **Unified provider+model selection** — Extracted ~500 lines of duplicate provider/model selection logic from `/models` dialog and `/onboard` wizard into shared `ProviderSelectorState` module (`src/tui/provider_selector.rs`). Both consumers now embed this struct, eliminating sync drift between the two UIs

### Testing
- **21 Slack formatting tests** — Bold conversion, italic unchanged, strikethrough, inline code, code blocks, headings, links, mixed formatting, real-world plan messages, edge cases
- **Onboarding test fixes** — Tests now set API key after reaching ProviderAuth step (detect_existing_key clears it on Workspace→ProviderAuth transition)

## [0.2.87] - 2026-03-26

### Added
- **Full CLI command surface** — 20+ subcommands: `opencrabs status`, `doctor`, `agent` (interactive multi-turn CLI agent + single-message mode), `channel list/doctor`, `memory list/get/stats`, `session list/get`, `db init/stats/clear`, `cron add/list/remove/enable/disable/test`, `logs status/view/clean/open`, `service install/start/stop/restart/status/uninstall` (launchd on macOS, systemd on Linux), `completions` (bash/zsh/fish/powershell via `clap_complete`), `version`, `daemon`, `onboard`. Full CLI reference added to README
- **Split panes** — tmux-style horizontal (`|`) and vertical (`_`) pane splitting in TUI. Each pane runs its own session with independent provider, model, and context. Run 10 sessions side by side, all processing in parallel. `Tab` to cycle focus, `Ctrl+X` to close pane. Pane focus indicator `[n/total]` in status bar. 21 tests covering layout, focus, and management
- **Dynamic tool system** — Define custom tools at runtime via `~/.opencrabs/tools.toml`. HTTP and shell executors, template parameters (`{{param}}`), enable/disable without restart. The `tool_manage` meta-tool lets the agent create, remove, and reload tools on the fly. `DynamicToolRegistry` with `RwLock`-based concurrent access
- **Native browser automation** — Headless Chrome control via CDP (Chrome DevTools Protocol). 7 browser tools: `navigate`, `click`, `type`, `screenshot`, `eval_js`, `extract_content`, `wait_for_element`. Lazy-initialized singleton, stealth mode, persistent profile, display auto-detection. Feature-gated under `browser` (`--features browser`)
- **Multi-agent orchestration** — Spawn independent child agents for parallel task execution. 5 tools: `spawn_agent`, `wait_agent`, `send_input`, `close_agent`, `resume_agent`. Children run in isolated sessions with auto-approve and essential tools
- **DB-persisted channel sessions** — All 5 channels (Telegram, Discord, Slack, WhatsApp, Trello) now persist channel/group sessions in SQLite by title via `find_session_by_title`. Sessions survive process restarts — no more lost context after daemon restart
- **Slack user/channel name resolution** — User display names and channel names resolved via `users.info` and `conversations.info` API on each message. Agent sees "Adolfo Usier" instead of "U066SGWQZFG", stored messages have proper `sender_name` and `channel_name`
- **Slack event dedup + fast ack** — `on_push_event` returns ack immediately via `tokio::spawn`, deduplicates by message timestamp. Eliminates Slack retry storms that caused duplicate processing with slow CLI providers
- **LLM-generated channel greetings** — Channels send a personalized greeting on first connect via Slack `app_mention` handling
- **OpenCode model pricing** — MiMo V2 Pro/Omni, Nemotron 3, Big Pickle, Zen, Go
- **CLI reference in README** — Full CLI command table with descriptions and flags added to Core Features section + Table of Contents

### Fixed
- **Per-channel session isolation** — Owner DMs share TUI session, but groups/channels get isolated per-channel sessions keyed by `channel_id` (Telegram, Discord, Slack). Previously all messages shared the TUI session regardless of source
- **In-memory session HashMap replaced with DB** — Channel sessions were stored in an in-memory `HashMap` that was lost on every restart, creating new sessions each time. Now uses SQLite `find_session_by_title` across all 5 channels
- **Slack duplicate message processing** — Slack retried events when ack took >3s (common with CLI providers). Each retry was processed as a new message, causing cascading cancellations and repeated work. Fixed with timestamp dedup + background spawn
- **Slack empty sender/channel names** — `store_channel_msg` was storing `String::new()` for sender name and `None` for channel name. Channel history showed blank names
- **Streaming response text concatenation** — `IntermediateText` events were not clearing the streaming response buffer, causing text to concatenate across tool rounds
- **Persistent typing indicators** — Telegram and Slack typing indicators now persist during long agent responses
- **Onboarding API key requirement for CLI providers** — CLI providers (Claude CLI, OpenCode CLI) no longer require an API key during onboarding
- **Slack mention detection with unknown bot_user_id** — Falls back to `<@U...>` pattern matching when `auth.test` fails
- **Slack bot token hot-reload** — Bot token is re-read from config at runtime for `auth.test` and API calls
- **Browser stealth mode** — Persistent profile directory, display auto-detection for headed mode
- **CLI provider auto-compaction** — Trigger auto-compaction after token calibration for CLI providers
- **Claude CLI token usage** — Cache creation and cache read tokens now included in usage calculation
- **CLI text/tool interleaving** — Real-time streaming preserves text and tool call ordering; queued messages inject at tool boundaries
- **CLI reasoning bloat** — Stop forwarding reasoning blocks after first tool call to prevent context explosion
- **CLI tool name normalization** — Lowercase tool names from CLI providers now match TUI display

### Changed
- **CLI module refactored** — All types (`Cli`, `Commands`, subcommand enums) and `run()` moved from `mod.rs` to `args.rs`. Module file is now clean module declarations only

### Testing
- **21 split pane tests** — Layout, focus cycling, close, and management
- **Claude CLI cache token tests** — Unit tests for cache token usage calculation
- **Browser headless tests** — Test coverage for headless Chrome integration

## [0.2.86] - 2026-03-23

### Added
- **Tool call context in all channels** — Slack and Discord now show real-time tool call progress with context (e.g. `✅ grep ("pattern")`), matching Telegram's behavior. Each tool call gets its own message that updates on completion
- **Smart tool context hints** — Tool descriptions show meaningful context: `cron_manage ("delete 'daily-report'")` instead of bare `cron_manage`. Handles action+target patterns for cron_manage, plan, task_manager, with smart fallback for unknown tools

### Fixed
- **Claude CLI 60s idle timeout** — CLI streams were killed after 60s of tool execution silence. Now sends Ping keepalives during tool execution and offsets content block indices across tool rounds to prevent collisions
- **OpenCode CLI idle timeout** — Same keepalive fix applied to OpenCode CLI provider for ToolUse, ToolResult, and mid-loop StepFinish events
- **Claude CLI tool calls invisible in TUI** — Tool calls, parameters, and Ctrl+O expansion were completely hidden. Now forwards tool_use content_block_start and input_json_delta as real StreamEvents, with cli_handles_tools() preventing re-execution
- **Queued message display ordering** — Queued messages appeared on top instead of after the assistant response, creating consecutive user/assistant messages. Swapped IntermediateText before QueuedUserMessage
- **Thinking text missing paragraph breaks** — Thinking blocks from different tool rounds were concatenated without separators. Now inserts `\n\n` between rounds
- **Provider wizard reverting to wrong provider** — Wrong index mapping in `new()` and `from_config()` for Claude CLI and OpenCode CLI providers
- **Selected model reverting to Sonnet** — `selected_model` index was never resolved from config after fetching models
- **Agent description in collapsed tool view** — Collapsed tool calls showed "Processing: Agent" instead of "Processing: Agent: Research heyiolo Supabase usage"
- **CLI-normalized tool names** — `format_tool_description` now matches both "Agent" and "agent" for CLI-normalized names
- **Telegram tool completion context lost** — Completion line showed just `✅ tool_name` without the context hint. Now single-line format preserves context
- **Help text padding in provider dialog** — Bottom commands aligned with provider list
- **Images dropped by CLI providers** — `materialize_image()` saves base64 images to temp files for Claude CLI and OpenCode CLI
- **Fallback provider model remapping** — Fallback provider now remaps model to its default when the primary's model is unsupported
- **OpenCode CLI stream break on tool-calls** — Don't break stream on `step_finish` with reason `tool-calls`
- **Cron session isolation** — Dedicated shared "Cron" session prevents cron jobs from polluting TUI session context

## [0.2.85] - 2026-03-22

### Added
- **OpenCode CLI provider** — New `opencode-cli` provider that spawns the local `opencode` binary for free LLM completions — no API key or subscription needed. Includes NDJSON streaming, extended thinking support, and live model fetching via `opencode models`
- **z.ai GLM provider** — New built-in provider for Zhipu AI (z.ai) with two endpoint types: General API and Coding API. Live model fetching, streaming, and tool support. Configurable via onboarding wizard or `/models`
- **Alphabetical provider sorting** — Provider lists in `/models` and `/onboard:provider` dialogs are now sorted alphabetically for easier navigation
- **Visual line navigation** — Up/Down arrow keys navigate wrapped lines visually in the input editor instead of jumping by logical lines. Queued message indicator shows when a message is waiting
- **Native extended thinking support** — `Thinking` variant in `ContentBlock` for native extended thinking content blocks from Anthropic models
- **Cron default provider/model config** — New `[cron]` config section to set default `provider` and `model` for cron jobs independently from interactive sessions
- **Real-time tool streaming events** — Emit `ToolStarted`/`ToolCompleted` events during streaming for real-time TUI tool visibility
- **AI providers README table** — All built-in providers listed in a summary table with auth type, models, and features at a glance
- **wacore 0.4.1 + stable Rust** — Upgraded wacore/whatsapp-rust crates from 0.3 to 0.4.1. Implemented 5 new trait methods (`get_max_prekey_id`, `get_latest_sync_key_id`, `store_sent_message`, `take_sent_message`, `delete_expired_sent_messages`). Added `wa_sent_messages` migration table. Disabled simd feature to drop nightly requirement. `cargo install opencrabs` now works on stable Rust

### Fixed
- **CLI provider onboarding skips API key** — CLI providers (OpenCode CLI) go directly from provider selection to model selection, matching the `/models` dialog behavior
- **`/models` filter/navigate for CLI providers** — Typing to filter and Up/Down navigation now work for CLI provider model lists
- **Anthropic `thinking_delta` SSE parsing** — Handle `thinking_delta` events in the Anthropic SSE stream parser instead of ignoring them
- **Streaming spinner spacing** — Added spacing between streaming content and the status spinner line
- **Thinking blocks skipped in SSE parser** — Skip thinking blocks in Anthropic SSE parser and suppress noisy log output
- **Context management: re-compact instead of hard-truncate** — Removed hard-truncation that blindly dropped messages; now re-compacts context instead, preserving conversation continuity
- **Context budget lowered to 65%** — Prevents MiniMax tool-call degradation that occurred at higher context utilization
- **XML tool-call recovery** — Recover XML tool calls from model output instead of silently dropping them
- **Secret redaction in DB persistence** — Redact secrets from user messages before writing to the database
- **Tool events emitted at ContentBlockStop** — Tool events now fire at `ContentBlockStop` with fully parsed input JSON instead of at `ContentBlockStart` with empty input, fixing TUI tool display timing
- **UTF-8 boundary panic** — Use `floor_char_boundary()` to prevent panics on string truncation at multi-byte character boundaries
- **Input buffer cleared on queued message injection** — Prevents stale input from leaking into the next prompt
- **z.ai inline API errors surfaced** — API error responses from z.ai now displayed in the TUI instead of silently dropping the stream

### Testing
- **21 OpenCode CLI provider tests** — Unit, config, factory, and end-to-end tests covering provider creation, model resolution, and actual CLI completions

## [0.2.84] - 2026-03-20

### Added
- **Cron HTTP webhook delivery** — Generic HTTP webhook URLs now supported as `deliver_to` targets in cron jobs, enabling integration with any HTTP endpoint (Slack incoming webhooks, custom APIs, notification services, etc.)

### Fixed
- **Streaming filter eating XML tags in prose** — The `STRIP_OPEN_TAGS` array in the streaming filter included tool-call XML tags (`<tool_call>`, `<tool_use>`, `<result>`, etc.). When MiniMax M2.7 mentioned these tags in prose (e.g. describing commit history), the filter entered `inside_think=true`, couldn't find a closing tag, and silently dropped all remaining text — truncating entire responses. XML tool-call tags removed from streaming `STRIP_OPEN_TAGS` (keep only `<think>`, `<!-- reasoning -->`, `<!--`)
- **`<result>` tag hallucinations from MiniMax M2.7** — Strip `<result>` XML blocks echoed by MiniMax in response text
- **`<tool_use>` hallucinated XML tags from MiniMax M2.7** — Strip `<tool_use>` wrapper blocks echoed by MiniMax
- **XML tool-call hallucinations parsed as real tool calls** — `acd3477` introduced a parser that converts XML tool-call blocks into actual executable tool calls when MiniMax emits them as text
- **LLM artifacts stripped from Telegram and cron delivery** — Hallucinated `<!-- tools-v2: -->`, `<!-- /tools-v2: -->`, `<think>`, `</think>`, and XML block markers now stripped before delivery to Telegram and cron webhook outputs
- **LLM artifacts stripped from Discord, Slack, and WhatsApp** — Same artifact stripping extended across all remaining channels
- **XML hallucination inline execution reverted** — Inline XML tool-call execution was poisoning context; reverted to pure stripping approach

### Changed
- **API error display includes error_type** — Error responses now include the raw `error_type` field and full Anthropic error body in logs for easier debugging

### Fixed
- **Plan `complete_task` fields made optional** — `success` and `output` fields on `complete_task` are now optional with defaults to prevent plan execution from getting stuck when the LLM omits these fields

## [0.2.83] - 2026-03-18

### Added
- **MiniMax M2.7 as default model** — Updated default model from M2.5 to M2.7 across config, pricing, onboarding, docs, and tests. M2.5 remains available as an option

## [0.2.82] - 2026-03-18

### Added
- **5 sub-agent orchestration tools** — Agents can now spawn independent child agents for parallel task execution. Five new tools: `spawn_agent`, `wait_agent`, `send_input`, `close_agent`, `resume_agent`. Children run in isolated sessions with auto-approve and essential tools (no recursive spawning)

### Fixed
- **"1 tok" streaming output counter** — Output token display reset to "1 tok" after each tool call because the callback accumulator was reset on every TokenCount event. Moved accumulation to TUI side so it persists across tool loop iterations
- **Cancelled requests leave tool calls unstacked** — Late ToolCallStarted/Completed/IntermediateText events arriving after double-Escape cancel now dropped via `is_processing` guard, preventing orphaned tool entries in chat
- **Pending request recovery missing cancel token** — Restarted tasks couldn't be cancelled because the recovery path passed no CancellationToken. Now wires token via new `PendingResumed` TUI event
- **Strip `<param>` tags** — Broaden tool artifact stripping to also remove `<param>` XML blocks
- **Strip `<tool_code>` and `<tool_call>` blocks** — XML tool-call markers now stripped from streaming and iteration text
- **Strip all HTML comments** — HTML comment stripping broadened to prevent marker leaks in LLM output

### Testing
- **11 new tests** for strip_html_comments tool artifact stripping

## [0.2.81] - 2026-03-17

### Fixed
- **Context blows past 200K limit** — `enforce_context_budget` now guarantees context never exceeds 80% of `max_tokens`. Hard-truncation fallback drops oldest messages
- **Segfault on large embeddings** — Documents >32KB now skipped with placeholder to prevent llama-cpp-2 segfault
- **Duplicate agent spawns on resume** — HashSet dedup prevents 4 concurrent tasks instead of 2
- **Thinking indicator vanishes during tools** — Removed `active_tool_group.is_none()` condition
- **Escape/cancel doesn't abort running tools** — Tool execution now races against cancel token via `tokio::select!`
- **Queued messages appear inline** — Messages now appear in conversation flow at exact point consumed
- **"1 tok" bogus context display** — Token calibration rejects results below 100 tokens

## [0.2.80] - 2026-03-16

### Fixed
- **`/model` provider navigation jumps out of order** — Up/Down keys now follow the visual display order (static providers → existing custom names → "+ New Custom") instead of raw index order, matching `/onboard:provider` behavior
- **Queued messages stack as duplicate user bubbles** — Messages sent while the agent is processing no longer appear as dimmed duplicates in chat. They stay in the input area preview until the tool loop consumes them, then appear naturally in the conversation flow

## [0.2.79] - 2026-03-15

### Fixed
- **Infinite retry loop from XML tool-call fallback** — The XML fallback (added in 0.2.77) created synthetic tool IDs that providers like MiniMax rejected with "tool id not found", triggering unstoppable retry loops that couldn't be cancelled. Removed the XML fallback entirely; XML tool_call blocks are now stripped from output
- **`/stop` only killed latest agent call** — `/stop` now cancels all in-flight agent calls instead of only the most recent one

## [0.2.78] - 2026-03-14

### Fixed
- **Crash on multi-byte UTF-8 in repetition detection** — `detect_text_repetition` panicked when slicing the sliding window at a byte offset inside multi-byte characters like `❌` (3 bytes) or `—` (em-dash, 3 bytes). Now advances to the nearest valid char boundary before slicing. Same fix applied to the window drain logic
- **`<!-- tools-v2: -->` markers leaking into Telegram/channel output** — LLM echoes back tool result markers from conversation context. The streaming filter handles them during SSE parsing, but split chunks could let them through. Now stripped from `iteration_text` in the tool loop before emission to channels
- **Test coverage** — 1,423 tests (up from 1,420). Added 3 UTF-8 regression tests for repetition detection

## [0.2.77] - 2026-03-14

### Added
- **XML tool-call fallback** — Providers that emit tool calls as XML text (e.g. MiniMax `<tool_call><invoke name="...">`) are now parsed and executed. XML blocks are stripped from persisted content so raw markup doesn't appear in chat history
- **Self-improving agent instructions** — BOOT.md brain template now includes self-improving directives. Splash screen and taglines updated
- **TUI render clear tests** — 4 tests for ratatui buffer clearing behavior
- **Test coverage** — 1,420 tests (up from 1,406)

### Fixed
- **TUI garbled characters on scroll** — Splash screen logo line overflowed the fixed-width ASCII box by 10 characters (73 vs 63 inner width), writing stale chars into ratatui's double buffer that bled through when scrolling. Fixed logo text to fit within box. Added `Clear` widget before Paragraph render to wipe the entire chat area each frame, preventing any stale buffer content from bleeding through during navigation
- **Removed 128KB stream response cap** — Hard limit on streaming text was removed. Repetition detection (2KB sliding window + 200-byte substring matching), stream idle timeout (60s), user cancellation (`/stop`), and provider-side `max_tokens` are sufficient to handle runaway streams without arbitrarily truncating legitimate long responses

### Changed
- **Splash taglines refined** — Removed duplicated terms, rearranged for clarity

> **Existing users:** After updating, ask your Crabs to check for brain template diffs and update your brain files (e.g. "check for brain template updates and apply them")

## [0.2.76] - 2026-03-13

### Added
- **Streaming text repetition detection** — Detects when providers (e.g. MiniMax) loop the same content indefinitely during streaming. Uses a 2KB sliding window with 200-byte substring matching to catch loops early and terminate the stream cleanly
- **Human-readable error messages** — Cryptic provider errors like "error decoding response body" are now translated to actionable messages suggesting retry or model switch
- **Stream loop detection tests** — 12 tests covering repetition detection, false positive prevention, edge cases, and error message translation
- **Test coverage** — 1,406 tests (up from 1,394)

## [0.2.75] - 2026-03-12

### Added
- **Post-evolve brain update prompt** — After `/evolve` restarts, Crabs announces the new version, diffs brain templates, and offers to update user's brain files
- **WhatsApp error reporting** — Agent errors (session store failures, connection issues) now broadcast to the TUI with specific error messages, log paths, and retry/reset instructions
- **QR render tests** — 8 tests for Unicode QR code rendering (width consistency, expected characters, quiet zone)
- **WhatsApp state tests** — 7 tests for broadcast channel behavior (QR, connected, error channels)
- **Post-evolve tests** — 5 tests for version comparison and evolve message format
- **Test coverage** — 1,394 tests (up from 1,373)
- **Autostart-on-boot instructions** — README now covers systemd (Linux), launchd (macOS), and Task Scheduler (Windows)

### Fixed
- **WhatsApp QR popup width on Windows** — Used `unicode_width::UnicodeWidthStr` instead of `str::len()` for correct display column calculation (3 bytes per Unicode block char was tripling the width)

### Changed
- **Assets consolidated** — Screenshots, icons, and scripts moved from root directories into `src/assets/` and `src/scripts/`
- **SocialCrabs docs expanded** — Full setup guide, natural language usage examples, and per-platform command reference in README
- **GitHub Actions Node.js 24** — Added `FORCE_JAVASCRIPT_ACTIONS_TO_NODE24: true` to CI and release workflows

## [0.2.74] - 2026-03-12

### Added
- **Windows binary icon** — Embedded crab logo as application icon in Windows executables via `winresource` build script
- **Provider name normalization** — Channel commands now match provider names case-insensitively and resolve display names to config IDs (e.g. "GitHub Copilot" → "github-copilot"). Added GitHub Copilot to provider list — thanks @mariodian (#44)
- **Search tools documented in brain** — `brave_search` and `exa_search` now listed in TOOLS.md brain template so the LLM knows they exist when configured. Existing users: ask your Crabs to update its brain files

## [0.2.73] - 2026-03-12

### Added
- **Tool name normalization** — Providers that hallucinate tool names (e.g. MiniMax sending `"Plan: complete_task"` instead of `tool="plan"`) are now auto-corrected, preventing silent "Tool not found" failures
- **Test coverage** — 1,373 tests (up from 1,362). New: tool normalization (10), path traversal (2), custom brain file acceptance (1)

### Fixed
- **File tools restricted to working directory** — `read_file`, `write_file`, and `edit_file` now work with any absolute path on the system. Security is enforced by the approval mechanism, not a directory jail
- **Brain file allowlist too restrictive** — `load_brain_file` now accepts any `.md` file in `~/.opencrabs/`, not just a hardcoded list. User-created files like VOICE.md were silently rejected
- **`load_brain_file("all")` missed user files** — The "all" mode now scans the brain directory for user-created `.md` files in addition to built-in contextual files
- **Plan widget stuck after failed tool call** — If a plan tool call failed silently (e.g. hallucinated tool name), the plan widget stayed on screen indefinitely. Now auto-clears when the response completes and all tasks are done or the agent stops processing
- **Plan widget persists across restarts** — Stale InProgress plan files from previous runs no longer resurrect the plan widget on session load

### Changed
- GitHub Actions updated to Node.js 24 (`FORCE_JAVASCRIPT_ACTIONS_TO_NODE24: true`) ahead of June 2026 forced migration

> **⚠️ Note:** Any `.md` file placed in `~/.opencrabs/` root can now be loaded into brain context via `load_brain_file("all")` or by name. Avoid storing sensitive or non-brain files as `.md` in that directory.

## [0.2.72] - 2026-03-12

### Added
- **Collapse cargo build output** — Tool call details now collapse long `Compiling`/`Downloading`/`Fresh` blocks into single summary lines (e.g. "Compiled 100 crates")
- **Queued message preview** — Follow-up messages typed between tool calls now appear immediately in chat instead of waiting for tool completion
- **Mouse scroll wheel support** — Enable mouse capture for scroll wheel navigation in the TUI
- **CODE.md and SECURITY.md brain templates** — New brain templates seeded on install for coding standards and security patterns
- **Test coverage** — 1,362 tests (up from 1,286). New: provider navigation sync (8), brain templates (8), build output collapse (9), reasoning lines (6), AltGr input (8), system continuation (6), onboarding input helpers (23)

### Fixed
- **GitHub Copilot missing from provider resolution** — Copilot was added to TUI/config but not wired into `active_provider_and_model` candidates, so enabling it in config.toml silently fell through to the default provider
- **Provider navigation order with custom providers** — Custom providers (nvidia, opus, etc.) appeared between static providers visually but navigation jumped to wrong positions because internal index 6 ("+ New Custom") was between static and existing custom providers
- **Channel setup screens cropped on small terminals** — Channel list and all setup forms (Telegram, Discord, WhatsApp, Slack, Trello) now track focused field and scroll to keep it visible
- **Coming-soon channels cluttering the list** — Removed Signal, Google Chat, iMessage placeholders that couldn't be configured
- **Onboarding channel paste duplicating input** — Pasting a key appended to existing sentinel text instead of replacing it. Now uses cursor-aware paste with proper sentinel clearing
- **Windows non-US keyboard layouts** — Accept `/` and other characters that arrive via AltGr (Ctrl+Alt) on international keyboard layouts
- **Reasoning display losing newlines** — Preserve literal `\n` in reasoning/thinking streaming responses
- **Session model name desync** — Always sync display model name when switching sessions
- **Rebuild wake-up noise** — Hide internal rebuild wake-up message from chat history

### Docs
- Expanded README with full tool system documentation, CLI integrations, and companion tools
- Windows Defender troubleshooting section
- **CODE.md brain template updated** — Added problem-solving principles (never suppress errors, never give up on solutions, delete dead code)

> **Note for existing users:** This release adds new brain templates (CODE.md, SECURITY.md) and updates existing ones. If you installed OpenCrabs before v0.2.69, you may be missing these files. Ask your crab to check your brain templates and update them: *"Check my brain templates and update them if any are missing or outdated."*

## [0.2.71] - 2026-03-11

### Fixed
- **Streaming format loss on Copilot provider** — Newline-only stream deltas were dropped by a `.trim().is_empty()` check, stripping all markdown formatting from Copilot responses
- **Session restore on restart** — App now persists the last active session ID to `~/.opencrabs/last_session` and restores it on startup, instead of picking whichever session was most recently modified

## [0.2.70] - 2026-03-11

### Added
- **GitHub Copilot OAuth device flow** — Replaces the old GitHub Models PAT integration. Users authenticate via OAuth device flow (github.com/login/device), no PAT or GitHub CLI required. Automatic token refresh in the background. Works with any active Copilot subscription
- **Hard command blocklist for bash tool** — Catastrophic commands (rm -rf /, mkfs, dd on disks, etc.) are now blocked at the tool level before execution
- **Stable-first nightly-fallback for /evolve** — `cargo install` now tries stable toolchain first, falls back to nightly only if needed
- **Onboarding navigation improvements** — Shift+Tab moves backwards between fields, Ctrl+Backspace clears input, arrow keys navigate channel and provider setup screens
- **Test coverage** — 1,286 tests (up from 1,218). New: onboarding field navigation (36), Copilot provider (8), evolve tests (23), audio sanitization tests

### Fixed
- **keys.toml merge for GitHub provider** — OAuth tokens saved to keys.toml were never loaded back into config (pre-existing bug masked by old gh CLI fallback)

## [0.2.69] - 2026-03-11

### Added
- **GitHub Models provider** ([#41](https://github.com/adolfousier/opencrabs/issues/41)) — New provider with auto-detection via `gh auth token`. No API key needed for users already authenticated with the GitHub CLI. Supports GPT-4o, GPT-4.1, o3/o4-mini and all GitHub Models catalog
- **Custom provider management** — All `providers.custom.*` entries now appear as individual selectable items in both `/models` and `/onboard:providers`. Users can add unlimited custom providers (nvidia, ollama, lmstudio, etc.) and switch between them with a single keypress
- **Context window configuration** — New `context_window` field for custom/local providers in both UI screens and config.toml. Enables auto-compaction for models not recognized by name (e.g. local LLMs via LM Studio or Ollama)
- **Shift+Tab navigation** ([#43](https://github.com/adolfousier/opencrabs/issues/43)) — Move backwards between fields in all onboarding and setup screens. Shift+Tab reverses through fields, Escape goes back to the previous screen
- **CODE.md brain template** — Coding standards template for brain files: modular architecture, testing, security-first patterns
- **Test coverage** — 1,218 tests (up from 1,118). New: `context_window_test.rs` (14), `custom_provider_test.rs` (27), plus expanded voice onboarding, evolve, and file extract tests

### Fixed
- **`/models` crash on custom providers** — Index out of bounds panic when selecting existing custom providers (indices 7+) in the model selector dialog
- **Base URL corruption** — Switching between custom providers appended URLs instead of replacing them (e.g. `https://nvidia.com/v1http://127.0.0.1:1234`)
- **Provider index mapping** — Corrected index resolution for GitHub Models, model selection, and display names across onboarding and model selector
- **Session footer sync** — Provider/model details now update in the footer immediately after onboarding or model change
- **Onboarding quick-jump** — Shows provider/model details instead of generic "Settings saved" message
- **Factory hardening** — Provider factory never crashes on missing API keys; falls back gracefully through the provider chain
- **Custom provider list order** — Existing custom providers now appear before "+ New Custom Provider" button in both provider lists

### Docs
- Document nightly toolchain requirement for `cargo install` with system dependency instructions
- Add native TTS/STT comparison row to framework feature table

## [0.2.68] - 2026-03-10

### Added
- **Crash recovery dialog** — When the TUI crashes, a raw-terminal dialog lets users browse GitHub releases and roll back to older versions. Detects install method (pre-built binary, cargo install, source) and uses the correct upgrade strategy
- **Install method detection** — New `InstallMethod` enum (`Source`, `CargoInstall`, `PrebuiltBinary`) with runtime detection. Used by crash recovery and `/evolve`
- **Queued message preview** — Follow-up questions typed between tool calls now show a dimmed preview in the input box. Press Up to recall and edit before sending
- **Test coverage** — 1,118 tests (up from 1,089). New: queued message lifecycle (15), install method detection (6), OpenAI max_completion_tokens (4), evolve install-method dispatch (4)

### Fixed
- **OpenAI newer models error** (#40) — gpt-4.1-*, gpt-5-*, o1-*, o3-*, o4-* models reject `max_tokens`; now sends `max_completion_tokens` for these model families
- **TTS voice selection crash on missing python3 venv** (#39) — `local_tts_available()` now probes `python3 -c "import venv"` before offering local TTS. Error messages include platform-specific install instructions (e.g. `apt install python3-venv`)
- **Queued messages dropped between tool calls** — Messages queued during tool execution were flushed before tool groups completed, causing them to appear in the wrong position. Now flushed after tool group finalization
- **Evolve install-method awareness** — `/evolve` now dispatches to the correct upgrade path based on `InstallMethod::detect()`. Source builds suggest `/rebuild` instead of attempting binary download

## [0.2.67] - 2026-03-10

### Changed
- **Local STT engine: whisper-rs → rwhisper (candle)** — Replaced whisper-rs (ggml C) with rwhisper (candle-transformers, pure Rust). Eliminates ggml symbol conflicts with llama-cpp-sys-2 (issue #38). No C++ build dependency for whisper. Metal GPU acceleration on macOS
- **Panic strategy: abort → unwind** — Release builds now use `panic = "unwind"` so panics in rwhisper's internal threads produce backtraces instead of instant core dumps

### Added
- **Evolve health check + rollback** — `/evolve` now runs a health check on the new binary before swapping, backs up the current binary, and automatically rolls back if the new version fails post-swap verification
- **Runtime capability detection** — `local_stt_available()` (compile-time feature check) and `local_tts_available()` (runtime python3 probe, cached via OnceLock). Onboarding hides Local radio buttons when unavailable; mode cycling clamps to Off/API only
- **Wizard config reset** — `from_config()` resets saved Local STT/TTS mode to Off at load time if the capability is absent on the machine
- **Audio sanitization** — Scrub NaN/Inf from decoded audio, pad short audio to minimum 1s (16000 samples) to prevent candle tensor panics
- **Comprehensive test coverage** — 950 tests (up from ~840). New: evolve version comparison, audio sanitization, TTS/STT availability cycling, capability detection, wizard reset, codec support. Added `TESTING.md` with full documentation
- **TESTING.md** — Full test coverage documentation: 256+ tests across 12 modules with category breakdown

### Fixed
- **TUI broken on Linux (fd race)** — `suppress_stdout()` used `dup2` to redirect fd 1 during model loading, racing with TUI's `EnterAlternateScreen`. Removed process-wide fd redirection; background preload delayed 2s to start after alternate screen
- **Stdout bleed in /onboard:voice** — Restored `suppress_stdout()` as `pub(crate)` for `download_model()` and `LocalWhisper::new()` — safe since TUI is already in alternate screen
- **rwhisper crash on CPU Linux** — "illegal instruction core dumped" on older CPUs. Fixed via audio validation, padding, and panic=unwind
- **Local STT transcription timeout** — Added 300s timeout to prevent indefinite hangs
- **Typing indicator delay** — Show typing indicator immediately when processing voice messages
- **Removed unnecessary sandybridge rustflag** — Global `target-cpu=sandybridge` in `.cargo/config.toml` was unnecessary and spammed warnings on non-x86 platforms
- **TTS voice selection not persisting** — Enter on a downloaded Piper voice re-triggered download instead of advancing. Config was never saved because `apply_config()` was never reached
- **Linux CI missing ALSA dev** — `libasound2-dev` not installed on Ubuntu runners, breaking `--all-features` builds. Added to CI and release workflows including ARM64 cross-compile
- **Release workflow resilience** — Individual platform build failures no longer block the GitHub Release creation

### Docs
- Document `RUSTFLAGS="-C target-cpu=native"` for AVX1-only CPUs (Sandy Bridge) in README
- Add `local-stt` and `local-tts` to feature flags table in README

## [0.2.66] - 2026-03-09

### Fixed
- **Windows MSVC build** — Duplicate ggml symbols (LNK2005) from whisper-rs-sys and llama-cpp-sys-2 resolved with `/FORCE:MULTIPLE` linker flag. aws-lc-sys `__builtin_bswap` errors fixed by forcing CMake builder on Windows
- **TTS reads markdown literally** — Strip formatting markers (`**`, `` ` ``, ```` ``` ````, `#`, bullets) before sending text to Piper TTS. Code block content is kept and read aloud naturally
- **STT transcript cleanup** — Collapse whitespace in whisper transcription output
- **Single WhatsApp bot instance** — Onboarding subscribes to agent bot events via broadcast channels instead of creating a separate bot instance that conflicts with the agent

## [0.2.65] - 2026-03-09

### Added
- **Local TTS via Piper** — On-device text-to-speech using Piper (Python venv + ONNX voice models). Six voice presets (Ryan, Amy, Lessac, Kristin, Joe, Cori). Configurable via `tts_mode = "local"` and `local_tts_voice` in config.toml. Gated behind `local-tts` feature flag (enabled by default)
- **Off/API/Local mode for TTS** — TTS mode selector in `/onboard:voice` with three options: Off, API (OpenAI TTS), Local (Piper). Matches the existing STT mode selector
- **Voice preview after download** — Plays "Hey! I am {name}. Nice to meet you!" via system audio (afplay/aplay) after a Piper voice model downloads
- **WhatsApp session reset** — Press R on the WhatsApp onboarding screen to delete session.db and re-pair with a fresh QR code

### Fixed
- **Telegram voice waveform missing** — `pcm_to_opus` was producing WAV (RIFF header) instead of OGG/Opus. Now properly encodes via `opusic-sys` with OGG container (RFC 7845) and resamples Piper's 22050 Hz to 48000 Hz
- **Voice switching race condition** — `PiperDownloadProgress` events arriving after `PiperDownloadComplete` re-set progress, blocking re-download on voice switch
- **TTS config not persisted via quick-jump** — `/onboard:voice` quick-jump returned `WizardAction::Cancel` which dropped settings. New `QuickJumpDone` action calls `apply_config()` before closing
- **Piper venv never installed** — `setup_piper_venv()` was defined but never called before downloading voice models
- **Voice preview used wrong voice name** — `PiperDownloadComplete` event now carries the `voice_id` string instead of reading the wizard's selection index
- **Removed unnecessary `whisper-rs-sys` dependency** — Explicit dep removed; `whisper-rs` pulls it in transitively
- **Windows build failure** — Whisper log callback used wrong type (`u32` vs `ggml_log_level`) causing cross-platform compilation error
- **Release workflow duplicate test job** — Removed redundant test job from release.yml that was blocking releases since v0.2.60

## [0.2.64] - 2026-03-09

### Added
- **Local TTS via Piper** — On-device text-to-speech using Piper (Python venv + ONNX voice models). Six voice presets (Ryan, Amy, Lessac, Kristin, Joe, Cori). Configurable via `tts_mode = "local"` and `local_tts_voice` in config.toml. Gated behind `local-tts` feature flag (enabled by default)
  - `src/channels/voice/local_tts.rs` (new), `src/channels/voice/service.rs`, `src/channels/voice/mod.rs`, `Cargo.toml`
- **Off/API/Local mode for TTS** — TTS mode selector in `/onboard:voice` with three options: Off, API (OpenAI TTS), Local (Piper). Matches the existing STT mode selector
  - `src/tui/onboarding/voice.rs`, `src/tui/onboarding/types.rs`, `src/config/types.rs`
- **Voice preview after download** — Plays "Hey! I am {name}. Nice to meet you!" via system audio (afplay/aplay) after a Piper voice model downloads
  - `src/channels/voice/local_tts.rs`

### Fixed
- **Telegram voice waveform missing** — `pcm_to_opus` was producing WAV (RIFF header) instead of OGG/Opus. Telegram's `send_voice` API requires OGG/Opus to display the voice waveform. Now properly encodes via `opusic-sys` (already linked) with OGG container (RFC 7845) and resamples Piper's 22050 Hz to 48000 Hz. Zero new system dependencies
  - `src/channels/voice/local_tts.rs`, `Cargo.toml`
- **Voice switching race condition** — `PiperDownloadProgress` events arriving after `PiperDownloadComplete` re-set `tts_voice_download_progress` to `Some(0.0)`, blocking re-download on voice switch. Now ignores stale progress after completion and resets download state on voice navigation
  - `src/tui/app/state.rs`, `src/tui/onboarding/voice.rs`
- **TTS config not persisted via quick-jump** — `/onboard:voice` quick-jump returned `WizardAction::Cancel` which dropped settings without saving. New `QuickJumpDone` action calls `apply_config()` before closing
  - `src/tui/onboarding/types.rs`, `src/tui/onboarding/input.rs`, `src/tui/app/dialogs.rs`
- **Piper venv never installed** — `setup_piper_venv()` was defined but never called before downloading voice models. Added `pathvalidate` to pip install (required by piper-tts)
  - `src/tui/app/dialogs.rs`, `src/channels/voice/local_tts.rs`
- **Voice preview used wrong voice name** — `PiperDownloadComplete` event now carries the `voice_id` string instead of reading the wizard's selection index (which could change during async download)
  - `src/tui/events.rs`, `src/tui/app/state.rs`, `src/tui/app/dialogs.rs`
- **Removed unnecessary `whisper-rs-sys` dependency** — Explicit `whisper-rs-sys` dep removed; `whisper-rs` pulls it in as transitive dep. Log suppression now uses `whisper_rs::set_log_callback` instead of direct sys FFI
  - `Cargo.toml`, `src/channels/voice/local_whisper.rs`

## [0.2.63] - 2026-03-08

### Added
- **Local voice transcription (whisper.cpp)** — Full on-device speech-to-text via whisper.cpp behind the `local-stt` feature flag. Send voice notes on Telegram, WhatsApp, Discord, or Slack and get instant local transcription — zero API calls, zero latency, zero cost. Configurable via `stt_mode = "local"` and `local_stt_model` in config.toml. Supports tiny/base/small/medium models with automatic download via `/onboard:voice`. OGG/Opus decoding via `symphonia-adapter-libopus` for Telegram voice notes. STT dispatch routes between Groq Whisper API and local whisper.cpp based on config
  - `src/channels/voice/service.rs`, `src/channels/voice/local_whisper.rs`, `src/channels/voice/mod.rs`, `Cargo.toml`
- **Channel hot-reload** — Channels are now dynamically spawned/stopped when `channels.*.enabled` changes in config.toml at runtime. No restart needed. New `ChannelManager` reconciles running agents against config on every reload
  - `src/channels/manager.rs` (new), `src/channels/mod.rs`, `src/cli/ui.rs`
- **21 voice STT dispatch tests** — STT routing, config defaults, audio decode, codec registration, mock API dispatch
  - `src/tests/voice_stt_dispatch_test.rs` (new), `src/tests/mod.rs`

### Fixed
- **whisper.cpp TUI bleeding** — Suppressed whisper.cpp/ggml stderr output via no-op C log callback. Model loading and inference debug lines no longer bleed into the TUI
  - `src/channels/voice/local_whisper.rs`, `Cargo.toml` (`whisper-rs-sys` dependency)
- **Onboarding model fallback writing `"default"` to config** — `selected_model_name()` fell back to literal `"default"` when no models were loaded, which got written to config.toml and caused MiniMax API to reject all requests. Now returns empty string; caller skips write
  - `src/tui/onboarding/models.rs`
- **Voice setup test** — Updated `test_voice_setup_defaults` assertion to match new STT mode select as first field
  - `src/tui/onboarding/tests.rs`
- **Windows CI flaky test** — `test_concurrent_sessions_independent` used `tokio::join!` with in-memory SQLite causing contention on Windows. Runs sequentially now
  - `src/brain/agent/service/tests/parallel_sessions.rs`

## [0.2.62] - 2026-03-08

### Added
- **Provider sync across TUI and channels** — Model/provider switches now propagate to all agents (TUI, Telegram, Discord, Slack, WhatsApp) via config. Each channel syncs its provider on every message, and TUI syncs on config reload
  - `src/channels/commands.rs`, `src/tui/app/state.rs`, `src/config/types.rs`
- **Channel commands persist to session history** — `/help`, `/models`, `/usage`, `/sessions`, `/new`, `/stop` now save to session DB so they appear in TUI history and give the agent context
  - `src/channels/commands.rs`
- **Crate-level docs for docs.rs** — Rewritten landing page with current providers, channels, features, and architecture table. Added `rust-version = 1.91` (MSRV)
  - `src/lib.rs`, `src/main.rs`, `Cargo.toml`

### Fixed
- **sqlx → rusqlite upgrade path** — Auto-detect databases previously managed by sqlx (`_sqlx_migrations` table with `user_version=0`) and stamp migration version so existing databases don't fail on startup
  - `src/db/database.rs`
- **TUI model switch ordering** — Write `default_model` to config before `rebuild_agent_service()` so the provider loads the correct model instead of the stale one
  - `src/tui/app/dialogs.rs`
- **Channel model switch errors surfaced** — `switch_model` now returns errors to the user instead of silently dropping them. Model change is persisted to session history. Custom providers supported
  - `src/channels/commands.rs`, `src/channels/telegram/agent.rs`, `src/channels/discord/agent.rs`, `src/channels/slack/handler.rs`
- **`/models` hanging on OpenRouter/custom providers** — Added 10-second timeout on `fetch_models()`. Prefers config models (instant) over live fetch. Falls back to current model if fetch fails
  - `src/channels/commands.rs`
- **`/models` showing stale current model** — Provider picker now reads from config instead of the channel's separate (stale) AgentService instance
  - `src/channels/commands.rs`
- **slack_send blocks schema for OpenAI** — Added missing `items` field to `blocks` array schema. OpenAI strictly validates JSON schemas and rejects arrays without `items`. Closes #36
  - `src/brain/tools/slack_send.rs`
- **Flaky parallel_sessions test** — Fixed SQLite contention in concurrent write test by running sequentially (test validates provider isolation, not write concurrency)
  - `src/brain/agent/service/tests/parallel_sessions.rs`

### Removed
- **Stale model from channel prefix** — Removed `| Model: X` from all channel system instructions since channel agents have separate AgentService instances making it unreliable
  - `src/channels/telegram/handler.rs`, `src/channels/discord/handler.rs`, `src/channels/slack/handler.rs`, `src/channels/whatsapp/handler.rs`

## [0.2.61] - 2026-03-07

### Added
- **Cross-platform setup script** — `scripts/setup.sh` detects OS (macOS, Debian/Ubuntu, Fedora/RHEL, Arch, WSL) and installs all build dependencies (cmake, pkg-config, build tools) plus Rust nightly. One-liner: `bash <(curl -sL .../scripts/setup.sh)`
  - `scripts/setup.sh` (new)

### Fixed
- **Daily date-based config backup** — Config, keys, and commands files now use date-based backup filenames instead of overwriting a single backup

### Docs
- **Per-platform build prerequisites** — README now documents macOS (`brew install cmake pkg-config`), Fedora (`dnf`), and Arch (`pacman`) dependencies alongside existing Debian/Ubuntu instructions. Added one-liner setup reference
  - `README.md`

## [0.2.60] - 2026-03-07

### Added
- **A2A Send tool** — Agent-to-agent communication via A2A Protocol RC v1.0. Four actions: `discover` (fetch Agent Card), `send` (create task with message), `get` (poll task status), `cancel` (abort task). JSON-RPC 2.0 over HTTP with optional Bearer token auth
  - `src/brain/tools/a2a_send.rs` (new), `src/brain/tools/mod.rs`, `src/cli/ui.rs`
- **18 unit tests** for a2a_send — schema validation, approval logic, parameter validation, response text extraction, auth headers, Default impl
  - `src/brain/tools/a2a_send.rs`

### Fixed
- **Cron jobs spawn new sessions** — Cron scheduler now shares the TUI's active session via `Arc<Mutex<Option<Uuid>>>` instead of creating new sessions. Falls back to initial session, then most recent — never spawns new
  - `src/cron/scheduler.rs`, `src/cli/ui.rs`
- **`/compact` fails silently** — Compaction errors were logged but not shown to user. Now returns visible error message with troubleshooting hints
  - `src/brain/agent/service/tool_loop.rs`

### Improved
- **Channel context injection** — All channel handlers (Telegram, Discord, Slack, WhatsApp) now inject last 30 group messages as context before responding, so the agent stays aware of conversation flow
  - `src/channels/telegram/handler.rs`, `src/channels/discord/handler.rs`, `src/channels/slack/handler.rs`, `src/channels/whatsapp/handler.rs`
- **Telegram passive logging** — Voice, photo, and document messages in groups are now logged to `channel_messages` table after text extraction
  - `src/channels/telegram/handler.rs`
- **`/compact` on all channels** — Wired `ChannelCommand::Compact` to Telegram, Discord, Slack, and WhatsApp handlers
  - `src/channels/commands.rs`, `src/channels/telegram/handler.rs`, `src/channels/discord/handler.rs`, `src/channels/slack/handler.rs`, `src/channels/whatsapp/handler.rs`

### Docs
- **A2A README update** — Accurate A2A section with api_key config, message/stream endpoints, a2a_send tool docs, two-agent connection guide, Bearer auth examples, security & persistence notes
  - `README.md`

### Tests
- 6 unit tests for cron session resolution logic
  - `src/tests/cron_test.rs`

## [0.2.59] - 2026-03-07

### Added
- **Fallback provider chain** — Configure multiple fallback providers that are tried in sequence when the primary fails. Supports single (`provider = "openrouter"`) or array (`providers = ["openrouter", "anthropic"]`). Runtime retry wraps the primary provider transparently — no code changes needed downstream
  - `src/brain/provider/fallback.rs` (new), `src/brain/provider/factory.rs`, `src/config/types.rs`
- **Per-provider vision model** — Set `vision_model` in any provider config. The LLM calls `analyze_image` as a tool, which uses the vision model on the same provider API to describe images — giving any model vision capability without swapping the chat model. Falls back to Gemini vision when configured. MiniMax auto-injects `vision_model = "MiniMax-Text-01"` on first run
  - `src/brain/tools/provider_vision.rs` (new), `src/brain/provider/factory.rs`
- **Session working directory persistence** — `/cd` changes now persist to DB per session, restored on session switch. Shown as `~/path` badge in sessions screen
  - `src/db/models.rs`, `src/services/session.rs`, `src/tui/app/messaging.rs`, `src/tui/render/sessions.rs`, `src/migrations/20260307000001_add_session_working_dir.sql`
- **35 new tests** — Fallback chain config (9), runtime fallback behavior (10), vision model wiring (7), factory integration (4), active provider vision discovery (6)
  - `src/tests/fallback_vision_test.rs`

### Fixed
- **Update checker semver comparison** — Used string inequality instead of proper version comparison. Now uses `is_newer()` with lexicographic semver segments, and detects source builds via `source_cargo_version()`
  - `src/brain/tools/evolve.rs`
- **Home directory in TUI paths** — Footer and help screen showed full `/Users/username/...` paths. Now collapsed to `~/...`
  - `src/tui/render/input.rs`, `src/tui/render/help.rs`

### Docs
- **Fallback & vision docs** — Updated TOOLS.md, AGENTS.md, and BOOT.md templates with fallback provider config and vision_model documentation

> **Existing users:** Your local brain files at `~/.opencrabs/` are not updated automatically. Ask your Crab to fetch the latest templates from `src/docs/reference/templates/` and merge updates into your workspace brain files. New features: `[providers.fallback]` for provider chain failover, `vision_model` per provider. Also ask your Crab if you have image/vision setup in place — if not, it can help configure it. If you have multiple providers with API keys already set, your Crab can wire up fallback protection in config.toml for you.

## [0.2.58] - 2026-03-07

### Fixed
- **Vision images in OpenAI-compatible providers** — `ContentBlock::Image` was silently dropped because `OpenAIMessage.content` only supported strings. Changed to `serde_json::Value` to support polymorphic content (string or array with `image_url` parts). Fixes image/vision failures on Telegram and all channels
  - `src/brain/provider/custom_openai_compatible.rs`

### Docs
- **Image & file handling in brain templates** — Added `<<IMG:path>>` documentation to AGENTS.md and TOOLS.md templates so the agent knows how to handle incoming images from channels instead of hallucinating non-existent tools
  - `src/docs/reference/templates/AGENTS.md`, `src/docs/reference/templates/TOOLS.md`

> **Existing users:** Your local brain files at `~/.opencrabs/` are not updated automatically. Ask your Crab to compare templates at `src/docs/reference/templates/` against `~/.opencrabs/TOOLS.md` and `~/.opencrabs/AGENTS.md` and patch in the new image handling sections.

## [0.2.57] - 2026-03-07

### Added
- **Two-step `/models` flow** — `/models` now shows a provider picker first, then model picker for the selected provider. Works across Telegram (inline buttons), Discord (buttons), Slack (action buttons), and WhatsApp (plain text). Handles providers without `/models` endpoint via config fallback
  - `src/channels/commands.rs`, `src/channels/telegram/agent.rs`, `src/channels/telegram/handler.rs`, `src/channels/discord/agent.rs`, `src/channels/discord/handler.rs`, `src/channels/slack/handler.rs`
- **`/new` and `/sessions` commands** — Create new sessions and switch between recent sessions from any channel. Inline buttons on Telegram/Discord/Slack, plain text on WhatsApp. Owner uses shared TUI session, non-owners get per-user sessions
  - `src/channels/commands.rs`, `src/channels/telegram/agent.rs`, `src/channels/telegram/handler.rs`, `src/channels/discord/agent.rs`, `src/channels/discord/handler.rs`, `src/channels/slack/handler.rs`, `src/channels/whatsapp/handler.rs`
- **User-defined slash commands on channels** — Custom commands from `commands.toml` (e.g. `/credits`) now work from Telegram, Discord, Slack, and WhatsApp. `action = "prompt"` forwards to the agent, `action = "system"` displays directly
  - `src/channels/commands.rs`, `src/channels/telegram/handler.rs`, `src/channels/discord/handler.rs`, `src/channels/slack/handler.rs`, `src/channels/whatsapp/handler.rs`
- **Custom commands in /help** — User-defined commands now appear in a "Custom Commands" section on both channel `/help` and the TUI help screen, sorted alphabetically with descriptions
  - `src/channels/commands.rs`, `src/tui/render/help.rs`
- **Emoji picker** — Type `:` followed by a shortcode to trigger an emoji autocomplete popup in the TUI. Arrow keys to navigate, Tab/Enter to insert, Esc to dismiss. Powered by the `emojis` crate
  - `src/tui/app/state.rs`, `src/tui/app/input.rs`, `src/tui/render/input.rs`, `src/tui/render/mod.rs`, `Cargo.toml` (`emojis = "0.8.0"`)
- **VOICE.md template** for voice configuration docs
  - `src/docs/reference/templates/VOICE.md`
- **"Why OpenCrabs?" README section** — security & binary size comparison vs Node.js frameworks
  - `README.md`

### Fixed
- **Context counter accuracy** — System brain tokens are now counted, and token counts no longer drop between requests
  - `src/brain/agent/service/builder.rs`, `src/brain/agent/service/context.rs`, `src/brain/agent/service/tool_loop.rs`, `src/tui/app/state.rs`
- **Stream bleed between sessions** — Streaming state is now cleared on session switch, preventing leftover content from appearing in a new session
  - `src/tui/app/messaging.rs`
- **Session switch confirmation shows name** — Channel callbacks now display the session title (e.g. "Chat") instead of a truncated UUID
  - `src/channels/telegram/agent.rs`, `src/channels/discord/agent.rs`, `src/channels/slack/handler.rs`

### Removed
- **HTTP gateway onboarding step** — Removed 339 lines of dead code. The gateway was inherited from OpenClaw's web UI design but never used; OpenCrabs runs via TUI/daemon and the A2A server handles external connections
  - `src/config/types.rs`, `src/tui/onboarding/` (7 files), `src/tui/onboarding_render.rs`, `src/tui/render/help.rs`, `src/brain/tools/config_tool.rs`

### Tests
- 277-line context tracking test suite for brain token counting
  - `src/brain/agent/service/tests/context_tracking.rs` (new)
- 14 unit tests for channel commands: `format_number`, `format_help`, `provider_display_name`, `match_user_command_inner`
  - `src/channels/commands.rs`

## [0.2.56] - 2026-03-06

### Added
- **Daily release check notification** -- Background task checks GitHub for new releases on startup (after 10s delay) and every 24 hours. When an update is available, shows a temporary system message in chat prompting the user to `/evolve`. Reuses the existing `check_for_update()` extracted from the evolve tool
  - `src/brain/tools/evolve.rs`, `src/tui/app/state.rs`

### Fixed
- **Context counter showing 243/200K when real usage was 19K** -- Two compounding bugs. First, MiniMax and OpenAI compatible providers send real usage in a final `choices: []` chunk after the `finish_reason` chunk. We emitted `MessageStop` on `finish_reason` and never captured the real token counts, falling back to tiktoken estimates. Second, the calibration formula subtracted `tool_count * 500` from input tokens to isolate message-only count. With ~38 tools that was 19,000 subtracted from 19,286 estimated input, leaving 286 tokens as the context count shown to the user
  - `src/brain/provider/custom_openai_compatible.rs`, `src/brain/agent/service/helpers.rs`, `src/brain/agent/service/tool_loop.rs`
- **Streaming stop_reason overwritten by deferred usage delta** -- When providers send a usage-only `MessageDelta` after the `finish_reason` chunk, `stop_reason` was overwritten with `None`. Now only updates `stop_reason` when the delta carries one
  - `src/brain/agent/service/helpers.rs`

### Tests
- 5 new streaming usage unit tests covering deferred vs inline usage patterns, tool calls with deferred usage, content preservation, and zero-start override
  - `src/brain/agent/service/tests/streaming_usage.rs` (new)

## [0.2.55] - 2026-03-06

### Added
- **Cumulative usage ledger** — New `usage_ledger` table tracks all token/cost usage permanently. Deleting or compacting sessions no longer resets usage stats. All-time totals in TUI, channel commands, and `/usage` tool now read from the ledger. Migration auto-backfills from existing sessions
  - `src/db/repository/usage_ledger.rs` (new), `src/migrations/20260306000001_add_usage_ledger.sql` (new), `src/services/session.rs`, `src/tui/render/dialogs.rs`, `src/tui/app/state.rs`, `src/brain/tools/slash_command.rs`, `src/channels/commands.rs`

### Fixed
- **Compaction overhaul — zero-truncation, DB persistence, exhaustive summaries** (closes #29) — Complete rewrite of the compaction system. Context is NEVER truncated before summarization — the full conversation reaches the LLM. Compaction prompt expanded to 10-section exhaustive format (chronological analysis, code snippets, user preferences with exact quotes, recovery playbook with `gh` CLI, personalized continuation message). Manual `/compact` now calls the real compaction pipeline instead of faking it. Compaction markers persist to DB so restarts load only from the last compaction point forward. All 5 compaction paths (manual, pre-loop, mid-loop x2, emergency) now persist markers. 24 compaction tests
  - `src/brain/agent/service/context.rs`, `src/brain/agent/service/tool_loop.rs`, `src/brain/agent/context.rs`, `src/tests/compaction_test.rs`, `src/tui/app/messaging.rs`, `src/docs/reference/templates/AGENTS.md`
- **TUI context counter wrong after restart** — After compacting and restarting, the TUI showed 200K/200K because `load_session` counted ALL DB messages instead of only post-compaction ones. Now filters through `messages_from_last_compaction` to match what the agent actually sees
  - `src/tui/app/messaging.rs`, `src/brain/agent/service/context.rs`
- **`/compact` placeholder visible in chat** — The internal `[SYSTEM: Compact context now...]` trigger message no longer shows as a user message in chat. The TUI already displays a "Compacting context..." system message
  - `src/tui/app/messaging.rs`
- **Metal destructor crash on macOS exit** — Replaced `std::process::exit` with `libc::_exit` to skip C atexit handlers that trigger llama.cpp's Metal GPU device destructor assertion on Apple Silicon. Clean exit, no more backtrace spam
  - `src/main.rs`, `Cargo.toml` (`libc = "0.2.182"`)
- **Empty context sent to compaction summarizer** — Fixed `trim_to_target` gutting context before the summarizer saw it. Removed dead method entirely
  - `src/brain/agent/context.rs`, `src/brain/agent/service/tool_loop.rs`, `src/tests/compaction_test.rs`

### Upgrade Notes
> **Existing users:** Update your brain files to get the new compaction behavior docs:
> ```sh
> cp src/docs/reference/templates/AGENTS.md ~/.opencrabs/AGENTS.md
> ```
> Or ask your agent: *"Update AGENTS.md from the repo template"*
>
> The `usage_ledger` migration runs automatically on first start — your existing session usage is backfilled so no data is lost.
>
> **Always check brain files for updates after upgrading** — templates evolve with each release and your local copies may be outdated.

## [0.2.54] - 2026-03-06

### Added
- **`/evolve` — binary self-update from GitHub releases** — New tool and slash command that checks the latest GitHub release, downloads the platform-specific binary, atomically replaces the current executable, and exec()-restarts into the new version. No Rust toolchain required. Fallback to legacy asset naming for backward compatibility with older releases. Available as `/evolve` slash command, `evolve` agent tool, and in the command palette
  - `src/brain/tools/evolve.rs` (new), `src/brain/tools/mod.rs`, `src/cli/ui.rs`, `src/tui/app/messaging.rs`, `src/tui/app/state.rs`, `src/tui/render/help.rs`, `src/brain/tools/slash_command.rs`, `Cargo.toml` (`flate2`, `tar`)
- **Versioned release assets** — CI now produces assets named `opencrabs-v{version}-{platform}.tar.gz` (e.g. `opencrabs-v0.2.54-macos-arm64.tar.gz`) instead of versionless names, making downloads unambiguous
  - `.github/workflows/release.yml`

### Fixed
- **Smarter post-compaction brain recovery** — Instead of blindly loading all brain files after compaction (which bloated context), the agent now receives a pre-compaction snapshot of the last 8 messages alongside the summary. This lets it analyze what context it needs and selectively load only relevant brain files. 22 new end-to-end compaction tests
  - `src/brain/agent/service/context.rs`, `src/brain/agent/service/tool_loop.rs`, `src/tests/compaction_test.rs` (new), `src/tests/mod.rs`

### Upgrade Notes
> **Existing users:** Update your brain files to include `/evolve` tool docs:
> ```sh
> cp src/docs/reference/templates/TOOLS.md ~/.opencrabs/TOOLS.md
> cp src/docs/reference/templates/AGENTS.md ~/.opencrabs/AGENTS.md
> ```
> Or ask your agent: *"Update TOOLS.md and AGENTS.md from the repo templates"*
>
> **Future updates:** After this version, just type `/evolve` to update — no manual steps needed.

## [0.2.53] - 2026-03-05

### Added
- **Cron jobs — full production implementation** (`ae79eee`) (closes #28) — Background `CronScheduler` polls DB every 60s, executes due jobs in isolated agent sessions with configurable provider/model/thinking. CLI subcommands: `cron add/list/remove/enable/disable` with name/UUID resolution. `CronManageTool` agent tool (5 actions: create/list/delete/enable/disable) with approval gates on create/delete. Telegram delivery via Bot API HTTP POST, Discord/Slack logged only. 43 tests covering CLI parsing, repository CRUD, cron expression validation, scheduler logic, and agent tool operations
  - `src/cron/mod.rs` (new), `src/cron/scheduler.rs` (new), `src/brain/tools/cron_manage.rs` (new), `src/cli/cron.rs` (new), `src/tests/cron_test.rs` (new), `src/brain/tools/mod.rs`, `src/cli/mod.rs`, `src/cli/ui.rs`, `src/lib.rs`
- **Passive message capture for Discord, Slack, and WhatsApp** (`027377a`) — All non-directed group messages are now stored in `channel_messages` table for `channel_search` tool access. Previously only Telegram captured messages. Discord captures at allowed_channels/dm_only/mention drop points and directed messages. Slack captures at the same drop points. WhatsApp captures all text messages after content extraction. Connect tools updated to pass `ChannelMessageRepository`. 24 tests for channel_search repository and tool operations
  - `src/channels/discord/handler.rs`, `src/channels/discord/agent.rs`, `src/channels/slack/handler.rs`, `src/channels/slack/agent.rs`, `src/channels/whatsapp/handler.rs`, `src/channels/whatsapp/agent.rs`, `src/brain/tools/discord_connect.rs`, `src/brain/tools/slack_connect.rs`, `src/brain/tools/whatsapp_connect.rs`, `src/cli/ui.rs`, `src/tests/channel_search_test.rs` (new)

### Fixed
- **Agent loses brain context after compaction** (`21c119e`) (closes #27) — After auto-compaction, the agent no longer reloads brain files (SOUL.md, AGENTS.md, USER.md, TOOLS.md) and answers without its identity, capabilities, or user preferences. Post-compaction instruction now mandates calling `load_brain_file` with `name="all"` as the first action before continuing the task
  - `src/brain/agent/service/tool_loop.rs`

### Upgrade Notes
> **Existing users:** Your local brain files at `~/.opencrabs/` are not auto-updated. To get the latest `cron_manage` tool docs and `channel_search` guidance, update your brain files from the repo templates:
> ```sh
> cp src/docs/reference/templates/TOOLS.md ~/.opencrabs/TOOLS.md
> cp src/docs/reference/templates/AGENTS.md ~/.opencrabs/AGENTS.md
> ```
> Or ask your agent: *"Update TOOLS.md and AGENTS.md from the repo templates"* — it can use `write_opencrabs_file` to do it for you.

## [0.2.52] - 2026-03-05

### Added
- **Reply-to-message context across all channels** (closes #26) — When a user replies to a specific message, the agent now receives the quoted message text and sender as context. Previously the agent had no way to know what message was being referenced
  - **Telegram** (`c1f51be`) — Extracts `reply_to_message()` text and sender. Bot replies labeled "assistant", user replies show sender name
  - **Discord** (`26dc53e`) — Extracts `referenced_message` content and author. Bot replies labeled "assistant"
  - **Slack** (`b00c8bb`) — Detects thread replies via `thread_ts`. Slack events don't embed parent message text, so thread context is noted without parent content
  - **WhatsApp** (`00d4e02`) — Extracts quoted message from `ExtendedTextMessage` context_info. Sender shown as phone number from participant JID
- **Cron jobs DB layer** (`43e7448`) — New `cron_jobs` table migration, `CronJob` model with full scheduling fields (cron expression, timezone, provider, model, thinking, auto_approve, deliver_to), `CronJobRepository` with insert/list/find/delete/enable/disable/update_last_run. Foundation for scheduled isolated sessions via CLI or agent `cron_manage` tool
  - `src/migrations/20260305000002_add_cron_jobs.sql` (new), `src/db/repository/cron_job.rs` (new), `src/db/models.rs`, `src/db/repository/mod.rs`, `Cargo.toml` (`cron = "0.15"`)

### Fixed
- **Native text selection restored** (`6962572`) — Disabled mouse capture that was blocking terminal text selection. Users can now select and copy text normally with left-click drag + keyboard copy
- **API key look-alike in test fixture** (`eb0b3f2`) — Replaced realistic-looking Google API key pattern in sanitize test with clear fake placeholder to avoid false positive leak alerts

### Improved
- **Brain templates updated** (`3e5033b`, `a590fce`, `43e7448`) — TOOLS.md template: `telegram_send` 16→19 actions (`get_chat_administrators`, `get_chat_member_count`, `get_chat_member`), added `channel_search` tool with `list_chats`/`recent`/`search` operations, empty state guidance for agents, `cron_manage` tool (5 actions: create/list/delete/enable/disable), system CLI tools reference (gh, gog, docker, ssh, node, etc.) with full gh and gog command docs. Updated `commands.toml.example` with `/chats` and `/history` example commands
- **README.md** (`43e7448`) — Added "Cron Jobs & Heartbeats" section with CLI examples, agent tool description, options table, HEARTBEAT.md usage, and heartbeat vs cron comparison

## [0.2.51] - 2026-03-05

### Added
- **Telegram message history capture and search** (`20c6008`) — Passive capture of Telegram group messages into new `channel_messages` table. New `channel_search` tool with `list_chats`, `recent`, and `search` operations. Telegram Bot API cannot fetch history, so the handler stores all group messages (directed and non-directed) as they arrive for on-demand retrieval. New migration, `ChannelMessageRepository`, and `ChannelMessage` model. Discord/Slack already have API-based history fetching via existing tools
  - `src/migrations/20260305000001_add_channel_messages.sql` (new), `src/db/repository/channel_message.rs` (new), `src/db/models.rs`, `src/db/mod.rs`, `src/brain/tools/channel_search.rs` (new), `src/brain/tools/mod.rs`, `src/channels/telegram/handler.rs`, `src/channels/telegram/agent.rs`, `src/brain/tools/telegram_connect.rs`, `src/cli/ui.rs`
- **Telegram chat and member info** (`20c6008`) — `get_chat` (chat details), `get_chat_administrators` (admin list with roles), `get_chat_member_count`, `get_chat_member` (user status lookup). Agent previously had no way to query Telegram chats or members. 19 telegram_send actions total
  - `src/brain/tools/telegram_send.rs`
- **Click-to-select and right-click-to-copy messages** (`5498970`, `d7fe7d7`) — Left-click highlights a message with subtle background, right-click copies clean content to clipboard via `pbcopy`/`xclip`/`xsel` with 2s cyan notification. Separate notification system from error messages. Line-to-message mapping built during render for coordinate lookup
  - `src/tui/app/state.rs`, `src/tui/app/input.rs`, `src/tui/events.rs`, `src/tui/render/chat.rs`, `src/tui/runner.rs`
- **Vim-style cross-platform input bindings** (`fbb92ad`, `de05b5c`, `8309804`) — `Ctrl+J` (newline), `Ctrl+W` (delete word), `Ctrl+U` (delete to line start). macOS Option key doesn't send ALT modifier in terminals, so Alt+Enter/Alt+Backspace never worked — vim bindings are the reliable cross-platform alternative. Crossterm `DISAMBIGUATE_ESCAPE_CODES` keyboard enhancement. Comprehensive delete-word key matching across terminal encodings (Backspace+modifiers, DEL `0x7f`, raw Ctrl+H/W). Up/Down arrows jump to start/end of line before entering history on single-line input. Home/End and Ctrl+U are line-aware in multiline
  - `src/tui/app/state.rs`, `src/tui/app/input.rs`, `src/tui/events.rs`, `src/tui/runner.rs`
- **Detailed Telegram logging** (`255a293`) — Verbose tracing for group/channel interactions to diagnose message routing
  - `src/channels/telegram/handler.rs`

### Fixed
- **Context display showed raw API tokens including tool schema overhead** (`2532b51`) — `AgentResponse.context_tokens` now uses calibrated `context.token_count` (message-only) instead of raw API `input_tokens` which included ~22k tool schema overhead for 44 tools. Display no longer shows inflated 210k/200k
  - `src/brain/agent/service/tool_loop.rs`, `src/brain/agent/service/tests/basic.rs`
- **Owner detection used HashSet random iteration order** (`89ae548`) — `HashSet::iter().next()` is non-deterministic, causing the wrong user to be identified as owner. Fixed to use `tg_cfg.allowed_users.first()` (Vec order from config = deterministic, first entry = owner)
  - `src/channels/telegram/handler.rs`
- **Forward TokenCount events to TUI during channel interactions** (`b98027a`) — TUI now receives real-time token count updates when messages arrive via Telegram/Discord/Slack/WhatsApp
  - `src/channels/telegram/handler.rs`
- **Restore most recent session from DB on daemon restart** (`c423410`) — Daemon no longer starts with a blank session after restart
  - `src/cli/ui.rs`

### Improved
- **Help screen colors** (`eb10a96`) — Orange section titles, cyan command keys matching TUI theme. Added INPUT EDITING section documenting all keybindings
  - `src/tui/render/help.rs`
- **README keyboard shortcuts** — Updated with vim bindings, mouse actions, multiline navigation
  - `README.md`

## [0.2.50] - 2026-03-04

### Changed
- **Config hot-reload via `watch` channel** — Replaced per-channel `Mutex` copies of config (allowlists, voice, respond_to, idle_timeout) with a single `tokio::sync::watch<Config>` channel. All channels now read the latest config per-message from the watch receiver. Removed `allowed_users`/`allowed_phones` HashSet fields from all channel states and 4 separate allowlist callbacks in `ui.rs`
  - `src/channels/factory.rs`, `src/channels/{telegram,discord,slack,whatsapp}/{mod,agent,handler}.rs`, `src/cli/ui.rs`, `src/brain/tools/{telegram,discord,slack,whatsapp}_connect.rs`, `src/brain/tools/whatsapp_send.rs`
- **TTS voice/model read from `[providers.tts]`** — Added `voice` and `model` fields to `TtsProviders` so `voice = "echo"` under `[providers.tts]` is actually picked up. Previously serde silently ignored the field
  - `src/config/types.rs`, `src/channels/factory.rs`, `src/channels/{telegram,discord,slack,whatsapp}/handler.rs`
- **Default TTS voice changed from "ash" to "echo"**; both `stt_enabled` and `tts_enabled` now default to `false` (user must opt in)
  - `src/config/types.rs`

### Fixed
- **Telegram "Session not found" after TUI quit** — The retry logic checked for `"SessionNotFound"` (camelCase) but the error Display produces `"Session not found"` (lowercase), so recovery never triggered. Now correctly matches and creates a fresh session (closes #24)
  - `src/channels/telegram/handler.rs`
- **Duplicate data delivery on Telegram** — LLM sent data both as streaming text response AND via `telegram_send`, resulting in the same content appearing twice. Added channel context prefix to all handlers telling the LLM its text response is auto-delivered (closes #23)
  - `src/channels/{telegram,discord,slack,whatsapp}/handler.rs`
- **Groq API key in test fixture** — Replaced real-format Groq key in `redact_secrets` test with obviously fake placeholder (closes #25)
  - `src/utils/sanitize.rs`

## [0.2.49] - 2026-03-04

### Added
- **Channel commands (`/help`, `/usage`, `/models`, `/stop`)** — All four commands now work on Telegram, Discord, Slack, and WhatsApp. Shared `commands.rs` module handles parsing; each channel renders platform-native responses (inline keyboards, action rows, Block Kit buttons)
  - `src/channels/commands.rs` (new), `src/channels/mod.rs`, `src/channels/telegram/handler.rs`, `src/channels/discord/handler.rs`, `src/channels/slack/handler.rs`, `src/channels/whatsapp/handler.rs`
- **`/stop` cancels running agent on channels** — `CancellationToken` per session, equivalent to double-Escape in TUI. Immediately aborts streaming/tool loop mid-run
  - `src/channels/telegram/mod.rs`, `src/channels/discord/mod.rs`, `src/channels/slack/mod.rs`, `src/channels/whatsapp/mod.rs`, all handler files
- **`/models` interactive model switching on channels** — Platform-native buttons (Telegram `InlineKeyboardMarkup`, Discord `ActionRow`, Slack Block Kit) with `model:` callback handlers
  - `src/channels/telegram/agent.rs`, `src/channels/discord/agent.rs`, `src/channels/slack/handler.rs`
- **Agent `slash_command` tool returns real data** — `/models`, `/usage`, `/help`, `/doctor`, `/sessions` now execute and return actual context instead of "TUI-only" errors, enabling the agent to read config, check health, and switch models via `config_manager`
  - `src/brain/tools/slash_command.rs`, `src/brain/tools/trait.rs`, `src/brain/agent/service/tool_loop.rs`
- **`service_context` on `ToolExecutionContext`** — Tools can now access `ServiceContext` for DB queries (used by `/usage` and `/sessions`)

### Fixed
- **Image API key stored under wrong path** — Onboarding wrote to flat `[image]` section in keys.toml instead of `[providers.image.gemini]`, inconsistent with all other provider keys. Added `ImageProviders` struct, merge logic, and legacy fallback
  - `src/config/types.rs`, `src/tui/onboarding/config.rs`
- **Channel commands section in README** — Documented `/help`, `/usage`, `/models`, `/stop` for all channels including WhatsApp
- **`keys.toml` parse errors now surface visibly** — Invalid TOML (e.g. unquoted emails) previously caused silent key merge failure, breaking provider startup with no error. Now prints warning to stderr and logs error. `/doctor` validates keys.toml syntax
  - `src/config/types.rs`, `src/brain/provider/factory.rs`, `src/brain/tools/slash_command.rs`

### Improved
- **Telegram tool calls as individual messages** — Each tool call now gets its own message (context + result) instead of all tools stacked in the response. Response streams cleanly at the bottom
  - `src/channels/telegram/handler.rs`
- **Intermediate agent texts visible on Telegram** — Agent commentary between tool rounds (e.g. "Found one! Let me reply to this:") now appears as individual messages, matching TUI behavior
  - `src/channels/telegram/handler.rs`

## [0.2.48] - 2026-03-04

### Added
- **Telegram thinking/reasoning stream** — Live `💭` reasoning content streams during inference, vanishes on tool calls and response chunks, keeping the conversation clean
  - `src/channels/telegram/handler.rs`
- **`quick_jump` mode for `/onboard:<step>` deep-links** — Any `/onboard:step` (except ModeSelect) opens locked to that single step: no progress dots, centered title, Enter confirms, Esc exits to chat. Step-change detection reverts navigation attempts
  - `src/tui/onboarding/wizard.rs`, `src/tui/onboarding/input.rs`, `src/tui/app/messaging.rs`, `src/tui/onboarding_render.rs`
- **Deferred health re-check on Enter** — In `/doctor` quick_jump mode, Enter resets all checks to Pending (visible flash), tick resolves them next frame. Reloads config from disk so external changes are picked up
  - `src/tui/onboarding/config.rs`, `src/tui/onboarding/fetch.rs`, `src/tui/app/state.rs`
- **YOLO (permanent) approval button on all channels** — Telegram, Discord, Slack, and WhatsApp now offer a 🔥 YOLO button alongside Always (session), persisting `auto-always` to config.toml so approval survives restarts
  - `src/channels/telegram/handler.rs`, `src/channels/telegram/agent.rs`, `src/channels/discord/handler.rs`, `src/channels/discord/agent.rs`, `src/channels/slack/handler.rs`, `src/channels/whatsapp/handler.rs`, `src/channels/whatsapp/mod.rs`, `src/utils/approval.rs`, `src/utils/mod.rs`

### Fixed
- **Redundant `check_approval_policy()` in tool loop** — Removed config-level short-circuit that was bypassing per-tool approval logic, fixing 3 approval policy test failures on CI
  - `src/brain/agent/service/tool_loop.rs`
- **CI and Release workflows running redundantly on tag push** — Added `tags-ignore: v*` to CI, added test gate (`needs: test`) to Release workflow
  - `.github/workflows/ci.yml`, `.github/workflows/release.yml`
- **`/doctor` standalone mode (closes #21)** — No onboarding chrome, Enter/Esc exit, removed redundant `/onboard:health` command
  - `src/tui/onboarding_render.rs`, `src/tui/app/state.rs`, `src/tui/render/help.rs`
- **Trello API Token not loaded in `from_config()`** — Health check falsely reported "No API Token provided" even when configured
  - `src/tui/onboarding/wizard.rs`
- **Model selector filter not working (closes #20)** — Filter text was typed but never applied to the displayed model list
  - `src/tui/render/dialogs.rs`
- **UTF-8 crash on multi-byte text in all channels** — 12 unsafe byte-index string slices replaced with `truncate_str()`, fixing panic on accented/emoji characters (e.g. Portuguese `õ`)
  - `src/channels/telegram/handler.rs`, `src/channels/discord/handler.rs`, `src/channels/slack/handler.rs`, `src/channels/whatsapp/handler.rs`
- **`quick_jump` blocking all step navigation** — Guard was catching internal step changes (field switching, channel sub-steps), not just step completion. Moved guard into `next_step()` so only step completion exits in deep-link mode
  - `src/tui/onboarding/input.rs`, `src/tui/onboarding/navigation.rs`, `src/tui/onboarding/wizard.rs`
- **Approval policy not persisting from channels** — Channels only offered "Always (session)" which wrote `auto-session`, downgrading the default YOLO policy. Now properly offers both session and permanent options
- **Updated README and commands.toml.example** with all `/onboard:*` sub-commands, `/doctor`, `/whisper`
  - `README.md`, `commands.toml.example`

## [0.2.47] - 2026-03-03

### Changed
- **Centralized tool approval into shared `utils::approval` module** — Replaced per-channel `auto_approve_session: Mutex<bool>` fields in Discord, Slack, Telegram, and WhatsApp with a single config-driven source of truth. Two new functions (`check_approval_policy`, `persist_auto_session_policy`) read/write `config.toml` directly, and the core `tool_loop.rs` checks policy first before delegating to any channel callback. Approval callbacks moved from `mod.rs` to `handler.rs` as free functions across all channels
  - `src/utils/approval.rs` (new), `src/utils/mod.rs`, `src/brain/agent/service/tool_loop.rs`, `src/channels/discord/mod.rs`, `src/channels/discord/handler.rs`, `src/channels/slack/mod.rs`, `src/channels/slack/handler.rs`, `src/channels/telegram/mod.rs`, `src/channels/telegram/handler.rs`, `src/channels/whatsapp/mod.rs`, `src/channels/whatsapp/handler.rs`, `src/channels/trello/handler.rs`

### Fixed
- **Telegram streaming message stuck at top between tool calls** — Streaming now uses separate `tools` and `response` fields with a `recreate` flag that deletes the old message and creates a fresh one below the approval buttons after each tool completion, so the conversation flows naturally downward instead of getting stuck above approval messages. Thanks @opryshok for reporting #17 and #16 — your bug reports directly drove this fix and the v0.2.46 improvements
  - `src/channels/telegram/handler.rs`, `src/channels/telegram/agent.rs`
- **Race condition in approval registration across all channels** — Pending approval is now registered BEFORE sending the approval message (not after), preventing a window where the user could click before the handler was ready
  - `src/channels/discord/handler.rs`, `src/channels/slack/handler.rs`, `src/channels/telegram/handler.rs`
- **TUI "Always" approval choice not persisting** — Clicking "AllowAlways" in the TUI now writes `approval_policy = "auto-session"` to `config.toml` so the choice survives restarts and is respected by all channels
  - `src/tui/app/input.rs`, `src/tui/app/messaging.rs`, `src/tui/app/state.rs`

### Added
- **Tracing/logging across all channel approval flows** — Every approval request, response, and edge case now logs via `tracing::info!` / `tracing::warn!` for easier debugging
  - `src/channels/discord/agent.rs`, `src/channels/discord/handler.rs`, `src/channels/slack/handler.rs`, `src/channels/telegram/agent.rs`, `src/channels/telegram/handler.rs`, `src/channels/whatsapp/handler.rs`
- **Cross-channel approval awareness** — TUI reads approval policy from `config.toml` on session create/load, so a policy set via Telegram or any other channel is picked up everywhere
  - `src/tui/app/messaging.rs`, `src/tui/app/state.rs`

## [0.2.46] - 2026-03-03

### Fixed
- **Telegram tool approval stuck after clicking Yes/Always** (`3716bf9`) — Three root causes fixed: (1) `ApprovalCallback` now returns `(bool, bool)` where the second bool propagates "Always" back into `tool_context.auto_approve`, so it persists across the entire tool loop instead of resetting after a few steps. (2) Race condition: pending approval is registered BEFORE sending the message, not after. (3) Tool input truncated to 3500 chars to prevent silent Telegram API rejection on long inputs. Closes #17
  - `src/brain/agent/service/types.rs`, `src/brain/agent/service/tool_loop.rs`, `src/channels/telegram/mod.rs`, `src/channels/discord/mod.rs`, `src/channels/slack/mod.rs`, `src/channels/whatsapp/handler.rs`, `src/tui/app/state.rs`, `src/cli/ui.rs`
- **Telegram missing tool call context and formatting** (`3716bf9`) — `ToolCompleted` events were being dropped by the progress callback; tool indicators used unsupported markdown. Now shows tool start/completion with proper labels. Improved `markdown_to_telegram_html` with headers, links, lists, underscore italic, and strikethrough. Closes #16
  - `src/channels/telegram/handler.rs`
- **Multiline Up/Down arrow keys never navigated lines** (`e27f59b`) — The multiline branch consumed all Up/Down events when the buffer contained newlines, even at boundaries (cursor at position 0 or end), blocking fall-through to history navigation. Now yields at boundaries: Up at position 0 falls through to history, Down at end of buffer does nothing
  - `src/tui/app/input.rs`
- **Light mode unreadable — user messages and UI text invisible on light terminals** (`009e8e3`) — Removed hardcoded dark user message background `Rgb(30,30,38)`. Replaced `Color::White` (invisible on light backgrounds) with `Color::Reset` (terminal's default foreground) across all render files. Diff backgrounds changed from dark RGB to ANSI named colors (Green/Red/DarkGray) that adapt to both themes
  - `src/tui/render/chat.rs`, `src/tui/render/input.rs`, `src/tui/render/help.rs`, `src/tui/render/dialogs.rs`, `src/tui/render/sessions.rs`, `src/tui/render/tools.rs`
- **Streaming token count duplicated ctx counter in input bar** (`2f8bc09`) — The per-chunk tiktoken counter was adding `ctx + output` together and feeding it back into `last_input_tokens`, making the tool group display show the same "28K ctx" already in the input bar, plus a duplicate timer. Now output tokens are tracked separately and displayed as a per-response count next to the timer: `(7s · 42 tok)`. The duplicate ctx+timer block below tool groups is removed
  - `src/tui/events.rs`, `src/tui/app/state.rs`, `src/tui/app/messaging.rs`, `src/tui/render/chat.rs`, `src/cli/ui.rs`

### Changed
- **Removed auto-backup logic** (`e142698`) — Git handles versioning; the custom backup mechanism was redundant

## [0.2.45] - 2026-03-03

### Added
- **Real-time token count during streaming** (`65a0278`) — The context usage display in the input box now increments live as the model responds: each streaming chunk is counted via tiktoken (cl100k_base) and fires a `TokenCountUpdated` event, so the counter ticks up token by token (e.g. `45K → 45.1K → 45.3K`) instead of jumping at the end of each API round-trip. The API-reported real count resets the baseline after each response, keeping the display accurate across multi-tool loops
  - `src/cli/ui.rs`
- **Elapsed time + ctx in thinking indicator** (`65a0278`) — The "OpenCrabs is thinking..." spinner now shows elapsed seconds and current context size: `⠙ OpenCrabs is thinking... 3s · 45K ctx`
  - `src/tui/render/mod.rs`
- **Running token count below active tool groups** (`65a0278`) — While tool calls execute, a subtle `45K ctx · 3s` line is rendered below the live tool group so you can see context growth during multi-tool sequences
  - `src/tui/render/chat.rs`
- **`opencrabs daemon` command** (`be61993`) — New headless subcommand: same full channel setup (Telegram, Discord, Slack, WhatsApp) as the TUI, but no terminal UI. Blocks on Ctrl-C. Designed for use by the systemd/LaunchAgent service installed during onboarding. Fixes the daemon not working after `opencrabs init` (issue #12)
  - `src/cli/mod.rs`, `src/cli/ui.rs`
- **28 CLI parsing unit tests** (`be61993`) — Full test coverage for all CLI subcommands including the new `daemon` command. Wired into `lib.rs` under `#[cfg(test)]`
  - `src/tests/cli_test.rs`, `src/tests/mod.rs`, `src/lib.rs`
- **Hot-reload for all three config files** (`1675fd2`) — `config_watcher` now watches `config.toml`, `keys.toml`, and `commands.toml`. Changing any of them is picked up within ~300ms without restart. Provider is swapped live when keys change (via `AgentService::swap_provider`). TUI refreshes approval policy and slash commands on reload
  - `src/utils/config_watcher.rs`, `src/cli/ui.rs`, `src/tui/app/state.rs`
- **`config.toml` and `commands.toml` annotated examples** (`4fdc1a6`) — Full annotated `config.toml` example added to the README Configuration section. New `commands.toml` section with complete syntax and action types reference. New `commands.toml.example` file in the project root matching the style of `keys.toml.example`. Two new Table of Contents entries added
  - `README.md`, `commands.toml.example` (new)

### Fixed
- **Daemon service not starting after install** (`be61993`) — systemd `ExecStart` was missing the `daemon` subcommand arg and `systemctl --user start` was never called after enable. macOS LaunchAgent plist was also missing the `daemon` arg in `ProgramArguments`. Both fixed. Closes #12
  - `src/tui/onboarding/config.rs`
- **config_watcher test hanging the test runner** (`be61993`) — Blocking `rx.recv()` loop inside `spawn_blocking` kept the tokio runtime from shutting down after tests. Fixed with a 200ms-poll loop and hard 3s deadline so the blocking thread exits cleanly
  - `src/utils/config_watcher.rs`
- **Nightly rustfmt CI failures** (`3208ac7`) — `telegram/mod.rs` and `whatsapp/handler.rs` had formatting differences between local stable `rustfmt` and the nightly toolchain used by CI. Fixed by running `cargo fmt` through the pinned nightly toolchain from `rust-toolchain.toml`
  - `src/channels/telegram/mod.rs`, `src/channels/whatsapp/handler.rs`
- **Redundant `.max(0)` on usize after `saturating_sub`** (`00fc64d`) — Clippy `unnecessary_min_or_max` lint: `usize::saturating_sub(1)` already clamps at 0, `.max(0)` was always a no-op. Removed from three fields in onboarding channels
  - `src/tui/onboarding/channels.rs`
- **llama-cpp-2 Metal segfault on macOS 26 arm64** (`118ea65`) — Bumped `llama-cpp-2` from `0.1.134` to `0.1.137` which includes the upstream Metal fix. Thanks @Pibomeister (PR #13)
  - `Cargo.toml`, `Cargo.lock`

### Changed
- **Default approval policy changed to `auto-always` for new users** (`3ed02ef`) — New installations no longer prompt before every tool call. The agent works autonomously out of the box. Existing users with `approval_policy` set in `config.toml` are unaffected (serde `default` only applies when the field is absent). To opt back into per-call prompts: run `/approve` → "Approve-only (always ask)"
  - `src/config/types.rs`, `README.md`
- **Telegram allowlist hot-reload extended to Discord and Slack** (`2b9b8c6`, `bd95b52`) — `allowed_users` lists for all three text channels now update at runtime when `config.toml` changes, without restart. Builds on the allowlist hot-reload foundation contributed by @Pibomeister (PR #14)
  - `src/channels/telegram/mod.rs`, `src/channels/discord/handler.rs`, `src/channels/slack/handler.rs`, `src/utils/config_watcher.rs`

## [0.2.44] - 2026-03-02

### Added
- **Google Gemini provider** (`e715536`) — Full `Provider` trait implementation against the Gemini REST API (`generativelanguage.googleapis.com/v1beta`). Streaming via SSE, tool use with `functionDeclarations`/`functionCall`/`functionResponse`, vision (multimodal `inlineData`), 1M–2M token context window. Live model list fetched from the Gemini API during onboarding and `/models`. Auth via `?key=` query param
  - `src/brain/provider/gemini.rs` (new), `src/brain/provider/factory.rs`, `src/brain/provider/mod.rs`
- **Image generation & vision tools** (`e715536`) — Two new agent tools powered by `gemini-3.1-flash-image-preview` ("Nano Banana"), independent of the main chat provider:
  - `generate_image` — Generate an image from a text prompt; saves PNG to `~/.opencrabs/images/`; returns file path for channel delivery
  - `analyze_image` — Analyze an image file path or URL via Gemini vision; works even when the main model doesn't support vision
  - `src/brain/tools/generate_image.rs` (new), `src/brain/tools/analyze_image.rs` (new), `src/brain/tools/mod.rs`
- **ImageSetup onboarding step** (`e715536`, `1336b89`, `f534b24`) — Step 7 in Advanced mode (after VoiceSetup, before Daemon). Toggle Vision Analysis and Image Generation independently; API key input with mask/replace mode; existing key detection. Model labeled as `gemini-3.1-flash-image-preview (🍌 Nano Banana)`. Persistent "get a free key at aistudio.google.com" hint shown when no key is set. Navigation: Space/↑↓ to toggle, Tab/Enter to continue, BackTab/Esc to go back
  - `src/tui/onboarding/types.rs`, `src/tui/onboarding/wizard.rs`, `src/tui/onboarding/navigation.rs`, `src/tui/onboarding/fetch.rs`, `src/tui/onboarding/config.rs`, `src/tui/onboarding_render.rs`
- **`/onboard:image` deep-link** (`e715536`) — Jump directly to the ImageSetup step from chat at any time
  - `src/tui/app/messaging.rs`
- **On-demand brain file loading** (`3224048`) — `build_core_brain()` replaces `build_system_brain()` at startup — injects only SOUL.md + IDENTITY.md (~1-2k tokens). All other brain files listed in a memory index; loaded by the agent via `load_brain_file(name)` tool on demand. `name="all"` loads everything. Dramatically reduces baseline token overhead for every message
  - `src/brain/prompt_builder.rs`, `src/brain/tools/load_brain_file.rs` (new), `src/cli/ui.rs`
- **`write_opencrabs_file` tool** (`8f3d648`) — Writes any file inside `~/.opencrabs/` (brain files, config, keys). Replaces the broken agent pattern of using `edit_file`/`write_file` which are locked to the working directory by `validate_path_safety()`
  - `src/brain/tools/write_opencrabs_file.rs` (new), `src/brain/tools/mod.rs`
- **`respond_to` selector in Telegram/Discord/Slack onboarding** (`9ecc8f0`) — New field in each channel's setup step; choose `all` / `dm_only` / `mention` mode during onboarding instead of editing config.toml manually
  - `src/tui/onboarding/types.rs`, `src/tui/onboarding/fetch.rs`, `src/tui/onboarding_render.rs`, `src/tui/onboarding/config.rs`
- **Google Image API Key in health check** (`6923174`) — When image features are enabled, the health check step verifies the Google AI key is present
  - `src/tui/onboarding/config.rs`
- **`send_file` action — discord_send and slack_send** (`905e9ef`) — New action uploads a local file as a native attachment. Discord: file attachment in channel. Slack: file upload via API. Both tools now at 17 actions
  - `src/brain/tools/discord_send.rs`, `src/brain/tools/slack_send.rs`
- **`add_attachment` action — trello_send** (`ac44fc3`) — New action uploads a local image or file as a Trello card attachment via multipart upload; returns the hosted Trello URL. Tool now at 22 actions
  - `src/brain/tools/trello_send.rs`, `src/channels/trello/client.rs`
- **Full file/image/audio input pipeline across all channels** (`9aed2ea`, `5bc33f5`) — Unified `classify_file(bytes, mime, filename) → FileContent` utility routes incoming files across every channel: images → vision pipeline (`<<IMG:path>>`), text/code/data files → extracted inline (up to 8 000 chars), audio → STT, PDFs → note to paste or use `analyze_image`. Trello: card attachments are fetched and processed on every incoming comment. Slack: voice/STT support added (was missing). All channels now handle images, text files, documents, and audio with consistent behavior
  - `src/utils/file_extract.rs` (new), `src/utils/mod.rs`, `src/channels/telegram/handler.rs`, `src/channels/discord/handler.rs`, `src/channels/whatsapp/handler.rs`, `src/channels/slack/handler.rs`, `src/channels/slack/agent.rs`, `src/brain/tools/slack_connect.rs`, `src/channels/trello/client.rs`, `src/channels/trello/handler.rs`, `src/channels/trello/models.rs`
- **TUI text file input** (`3e4460e`) — Paste or type any text file path in the TUI input field — the file is read and inlined automatically as `[File: name]\n```\ncontent\n``` `. Works at paste time and submit time. Supports `.txt`, `.md`, `.json`, `.yaml`, `.toml`, `.rs`, `.py`, `.go`, `.sql`, and 20+ other formats
  - `src/tui/app/state.rs`, `src/tui/app/messaging.rs`

### Fixed
- **Generated images delivered as native media across all channels** (`60584ff`) — `<<IMG:path>>` markers in agent replies are now unwrapped and delivered natively on every channel: Telegram `send_photo`, WhatsApp image message, Discord file attachment, Slack file upload, Trello card attachment + `![filename](url)` embed in comment. Previously the raw marker string was sent as plain text
  - `src/channels/telegram/handler.rs`, `src/channels/discord/handler.rs`, `src/channels/whatsapp/handler.rs`, `src/channels/slack/handler.rs`, `src/channels/trello/handler.rs`
- **Trello outgoing images — upload attachment + embed inline** (`3e4460e`) — Agent replies containing `<<IMG:path>>` on Trello are now uploaded as card attachments via `add_attachment_to_card` and embedded in the comment as `![filename](url)`. Previously the marker was silently dropped
  - `src/channels/trello/handler.rs`
- **Channel tool approval + TUI real-time updates follow-up** (`248b719`) — Follow-up fixes to tool approval flows and TUI live-update reliability across all remote channels after the v0.2.43 multi-channel expansion
  - `src/channels/*/mod.rs`, `src/brain/agent/service/tool_loop.rs`, `src/tui/app/state.rs`
- **Clippy: collapse nested if blocks** (`2595550`) — Fixed two `collapsible_if` lint errors in `messaging.rs` (TUI text file detection) and `whatsapp/handler.rs` (document attachment handling)
  - `src/tui/app/messaging.rs`, `src/channels/whatsapp/handler.rs`
- **TUI silent message queue after errors** (`dc815ce`) — After any agent error, `processing_sessions` was never cleared for the current session, causing all subsequent `send_message` calls to be silently queued with no agent running. Fixed by unconditionally removing the session from `processing_sessions` and `session_cancel_tokens` in the `TuiEvent::Error` handler before branching on current vs background session
  - `src/tui/app/state.rs`
- **TUI real-time updates during channel tool loops** (`b44f1ff`) — Remote channel tool loops (Telegram, WhatsApp, etc.) were not firing `session_updated_tx` on each chunk, causing the TUI to only refresh at the end of a long tool sequence. Now fires after every tool call completion
  - `src/brain/agent/service/tool_loop.rs`
- **Attachment input shows "Image #N" instead of full path** (`2609583`) — Attachment display in the TUI input bar was showing the full file path; now shows `Image #N`, `Document #N` placeholders matching the `<<IMG:...>>` / `<<DOC:...>>` injection format
  - `src/tui/render/input.rs`
- **WhatsApp TTS — upload media before sending audio message** (`135b4d6`) — TTS audio was being sent via `send_audio` before uploading to WhatsApp media servers, causing delivery failures. Now uploads first, then sends with the returned media ID
  - `src/channels/whatsapp/handler.rs`
- **WhatsApp handler regression — empty `allowed_phones` + connect tool** (`5a32d49`) — Empty `allowed_phones` in config was incorrectly blocking all messages including the owner. `whatsapp_connect` tool now correctly writes the config entry. Owner bypass re-validated
  - `src/channels/whatsapp/handler.rs`, `src/brain/tools/whatsapp_connect.rs`
- **WhatsApp security — block owner→contact processing** (`0fc8b2e`) — Messages sent by the owner *to* a contact were being processed as if the contact sent them, exposing the agent to arbitrary tool execution from outgoing messages
  - `src/channels/whatsapp/handler.rs`
- **WhatsApp outgoing `allowed_users` enforcement** (`1707e0f`) — Outgoing messages to contacts not in `allowed_users` were being processed; now strictly gated
  - `src/channels/whatsapp/handler.rs`
- **Context display reset immediately after compaction** (`2c4ca8e`) — After `/compact`, the context percentage in the TUI header was not resetting to the new value until the next message; now resets immediately
  - `src/tui/app/state.rs`

### Changed
- **Per-channel config structs** (`f28e229`) — Replaced the single flat `ChannelConfig` with 8 dedicated structs (`TelegramConfig`, `DiscordConfig`, `SlackConfig`, `WhatsAppConfig`, `TrelloConfig`, etc.) for cleaner config parsing, better type safety, and simpler channel-specific fields. Trello `board_ids` replaces the previous `allowed_channels` field
  - `src/config/types.rs`, all channel modules, `src/tui/onboarding/`

## [0.2.43] - 2026-03-02

### Added
- **Telegram full control — 16 actions + live streaming + approval buttons** (`c1ba37c`) — `telegram_send` tool expanded from `send` to 16 actions: `send`, `reply`, `edit`, `delete`, `pin`, `unpin`, `forward`, `send_photo`, `send_document`, `send_location`, `send_poll`, `send_buttons`, `get_chat`, `ban_user`, `unban_user`, `set_reaction`. LLM response streams live into a Telegram message with `▋` cursor (edits every 1.5 s). Session resilience: re-fetches bot from DB if lost across restarts. Idle session timeout per-user
  - `src/brain/tools/telegram_send.rs`, `src/channels/telegram/handler.rs`, `src/channels/telegram/mod.rs`
- **Discord full control — 16 actions + session idle timeout** (`3459d0b`) — `discord_send` tool expanded to 16 actions mirroring Telegram: `send`, `reply`, `edit`, `delete`, `pin`, `unpin`, `forward`, `send_photo`, `send_document`, `send_location`, `send_poll`, `send_buttons`, `get_guild`, `kick_user`, `ban_user`, `set_reaction`. Idle session timeout for per-user sessions
  - `src/brain/tools/discord_send.rs`, `src/channels/discord/`
- **Slack full control — 16 actions + sender context injection** (`89c9e71`) — `slack_send` tool expanded to 16 actions: `send`, `reply`, `react`, `unreact`, `edit`, `delete`, `pin`, `unpin`, `get_messages`, `get_channel`, `list_channels`, `get_user`, `list_members`, `kick_user`, `set_topic`, `send_blocks`. Non-owner messages now prepend sender identity `[Slack message from {uid} in channel {ch}]`
  - `src/brain/tools/slack_send.rs`, `src/channels/slack/handler.rs`
- **WhatsApp typing indicator** (`9f3b1fa`) — Sends `composing` chat state on message receipt, `paused` on completion so the user sees a native typing indicator while the agent processes
  - `src/channels/whatsapp/handler.rs`
- **Tool approval — 3-button UI across all channels** (`f6b8523`, `586cccd`, `816147c`) — All four remote channels now show ✅ Yes / 🔁 Always (session) / ❌ No approval prompts matching the TUI, powered by channel-native interactive elements (WhatsApp `ButtonsMessage`, Telegram inline keyboard, Discord `CreateButton`, Slack `SlackBlockButtonElement`). "Always" sets session-level `auto_approve_session` flag — no further prompts for that session
  - `src/channels/whatsapp/mod.rs`, `src/channels/telegram/mod.rs`, `src/channels/discord/mod.rs`, `src/channels/slack/mod.rs`
- **Tool input context in Telegram streaming indicator** (`3da472a`, `af4b96b`) — Streaming status line now shows a brief hint of what the tool is doing (e.g. `⚙ bash: git status`) so the user has context while waiting
  - `src/channels/telegram/handler.rs`
- **TUI auto-refresh when remote channels process messages** (`7b95209`) — After every `run_tool_loop` completion, `AgentService` fires a `session_updated_tx` notification. The TUI listens, calling `load_session` if the updated session is the current one (and not already being processed by the TUI), or marking it as unread otherwise. Real-time TUI updates when Telegram/WhatsApp/Discord/Slack messages are processed — no manual session switch required
  - `src/brain/agent/service/builder.rs`, `src/brain/agent/service/tool_loop.rs`, `src/tui/events.rs`, `src/tui/app/state.rs`, `src/cli/ui.rs`

### Fixed
- **SQLite WAL mode + larger pool** (`1ec5c3b`) — Enables write-ahead logging so concurrent reads (TUI) and writes (channel agents) don't block each other; pool size increased from 5 to 20 connections. Eliminates channel concurrency timeouts
  - `src/services/` (DB setup)
- **WhatsApp sender identity** (`00cc01b`) — Strips device suffix from JID (`:N@s.whatsapp.net` → `@s.whatsapp.net`) before phone-number comparison; injects `[WhatsApp message from {name} ({phone})]` for non-owner messages; fetches contact display name when available
  - `src/channels/whatsapp/handler.rs`
- **WhatsApp reply to chat JID instead of device JID** (`24c1e5d`) — Was replying to the device-scoped JID (`:0@s.whatsapp.net`) causing delivery failures in group chats and multi-device setups; now replies to the canonical chat JID
  - `src/channels/whatsapp/handler.rs`
- **Inject sender context for non-owner Discord and Telegram messages** (`e00374a`) — Non-owner messages now prepend `[Discord/Telegram message from {name} (ID {uid}) in channel {ch}]` so the agent knows who it's talking to instead of assuming the owner
  - `src/channels/discord/handler.rs`, `src/channels/telegram/handler.rs`
- **Secret sanitization — redact API keys from all display surfaces** (`436808e`, `d3a2380`) — New `utils::redact_tool_input()` function recursively walks tool input JSON, redacting values for sensitive keys (`authorization`, `api_key`, `token`, `secret`, `password`, `bearer`, etc.) and inline bash command patterns (`Bearer xxx`, `api_key=xxx`, URL passwords). Applied to TUI tool history, TUI approval dialogs, and all four remote channel approval messages
  - `src/utils/sanitize.rs` (new), `src/tui/render/tools.rs`, `src/channels/*/mod.rs`
- **WhatsApp upstream log noise suppressed** (`f6b8523`) — Added `whatsapp_rust::client=error` and `whatsapp_rust=warn` directives to filter upstream TODO stub log lines
  - `src/logging/logger.rs`

### Changed
- **Context budget enforcement refactored** (`d8ab8f0`) — Extracted repeated 80%/90% compaction logic into `enforce_context_budget()` helper on `AgentService`. 80 %: triggers LLM compaction. 90 %: hard-truncates to 80 % first, then compacts. Up to 3 retries on LLM compaction failure, then warns user to run `/compact`
  - `src/brain/agent/service/tool_loop.rs`
- **`send_message_with_tools_and_callback`** — Per-call approval and progress callback overrides; remote channels pass their own callbacks without touching service-level defaults
  - `src/brain/agent/service/messaging.rs`, `src/brain/agent/service/tool_loop.rs`

## [0.2.42] - 2026-03-01

### Added
- **Native Trello channel** (`80c7b05`) — TrelloAgent authenticates and makes credentials available for tool use. Default mode is tool-only — the AI acts on Trello only when explicitly asked via `trello_send`. Opt-in polling available via `poll_interval_secs` in config; when enabled, only responds to explicit `@bot_username` mentions from allowed users. Board names resolved automatically — mix human-readable names and 24-char IDs freely
  - `src/channels/trello/` (agent, client, handler, models, mod)
- **`trello_connect` tool** (`80c7b05`) — Verify credentials, resolve boards by name, persist to config, spawn agent, confirm with open card count. Accepts comma-separated board names or IDs
  - `src/brain/tools/trello_connect.rs`
- **`trello_send` tool — 21 actions** (`80c7b05`) — Full Trello control without exposing credentials in URLs: `add_comment`, `create_card`, `move_card`, `find_cards`, `list_boards`, `get_card`, `get_card_comments`, `update_card`, `archive_card`, `add_member_to_card`, `remove_member_from_card`, `add_label_to_card`, `remove_label_from_card`, `add_checklist`, `add_checklist_item`, `complete_checklist_item`, `list_lists`, `get_board_members`, `search`, `get_notifications`, `mark_notifications_read`
  - `src/brain/tools/trello_send.rs`
- **`/onboard:<step>` deep-links** (`e4975e4`) — Jump directly to any onboarding step: `/onboard:provider`, `/onboard:channels`, `/onboard:voice`, `/onboard:health`, etc. `/doctor` alias for `/onboard:health`
  - `src/tui/app/messaging.rs`, `src/tui/app/state.rs`, `src/tui/render/help.rs`

### Fixed
- **WhatsApp voice notes silently dropped** (`8e29655`) — Handler was skipping all non-text messages including voice notes (ptt). Now only skips if no text AND no audio AND no image
  - `src/channels/whatsapp/handler.rs`
- **STT key missing from channel factory** (`8e29655`, `d0a7651`) — `ChannelFactory` was built with `config.voice` which has `stt_provider=None`. All channel agents (WhatsApp, Discord, dynamic Telegram) now receive the fully resolved `VoiceConfig` with `stt_provider`/`tts_provider` populated
  - `src/cli/ui.rs`
- **Channel `allowed_users` unified** (`e4975e4`) — Removed `allowed_ids` from `ChannelConfig`, unified into `allowed_users: Vec<String>` with backward-compat deserializer accepting legacy TOML integer arrays. Fixed health check false failures: Discord and Slack channel IDs were read from wrong field
  - `src/config/types.rs`, channel agents
- **Channel config not passed to agents** (`406503b`) — `telegram_connect`, `discord_connect`, `slack_connect` now pass `respond_to` and `allowed_channels` from persisted config to agent constructors (previously hardcoded to defaults)
  - `src/brain/tools/telegram_connect.rs`, `discord_connect.rs`, `slack_connect.rs`
- **Tool expand (`Ctrl+O`) shows full params** (`0aba196`) — Expanded tool view now shows complete untruncated input params line by line. In-flight calls show a "running..." spinner. DB-reconstructed entries degrade gracefully
  - `src/tui/render/tools.rs`, `src/tui/app/state.rs`
- **Error/warning messages auto-dismiss after 2.5 s** (`4408a69`, `d0a7651`) — Timer resets correctly on user action; covers all clear-sites in input, messaging, and dialogs
  - `src/tui/app/dialogs.rs`, `input.rs`, `messaging.rs`
- **Thinking indicator sticky above input** (`406503b`) — Moved out of scrollable chat into a dedicated layout chunk — never scrolls away
  - `src/tui/render/mod.rs`, `chat.rs`
- **`/onboard` resets to first screen** (`d0a7651`) — Pre-loads existing config values while resetting to `ModeSelect` so health check shows correct state
  - `src/tui/app/messaging.rs`, `src/tui/onboarding/wizard.rs`
- **CI Windows build** (`001ed00`) — Replaced removed `aws-bedrock`/`openai` features with `telegram,discord,slack` in Windows CI workflow
  - `.github/workflows/ci.yml`
- **Trello agent tool-only by default** (`7ca6b6b`) — Removed automatic polling and auto-replies. Agent starts in tool-only mode (credentials stored, no polling). `poll_interval_secs` in `[channels.trello]` config opts in to polling; even then only @mentions from allowed users trigger a response. Adds `poll_interval_secs: Option<u64>` to `ChannelConfig`
  - `src/channels/trello/agent.rs`, `src/config/types.rs`, `src/cli/ui.rs`, `src/brain/tools/trello_connect.rs`, `config.toml.example`

## [0.2.41] - 2026-03-01

### Fixed
- **WhatsApp onboarding — always test connection on Enter** (`676ab29`) — Pressing Enter on the phone field now always triggers a test message, matching Telegram/Discord/Slack behavior. Previously the test was gated on `whatsapp_connected`, so re-opening the app with an existing session silently skipped the test and just advanced
  - `src/tui/onboarding/channels.rs`
- **WhatsApp onboarding — reconnect from existing session for test** (`676ab29`) — When no live client is in memory (app reopened after prior pairing), `test_whatsapp_connection` now calls `reconnect_whatsapp()` which reuses the stored `session.db` without wiping it — no new QR scan required
  - `src/brain/tools/whatsapp_connect.rs`, `src/tui/app/dialogs.rs`
- **WhatsApp test message includes brand header** (`676ab29`) — Test message now prepends `🦀 *OpenCrabs*\n\n` (the `MSG_HEADER` constant) so it reads consistently with all other WhatsApp messages sent by the agent
  - `src/tui/app/dialogs.rs`
- **WhatsApp onboarding — post-QR UX overhaul** (`676ab29`) — After scanning the QR code the popup dismisses, the wizard advances to the phone allowlist field, shows any previously configured number (sentinel pattern), and allows confirm-or-replace before testing. Navigation keys (Tab/BackTab/S) always work regardless of test state; only Enter is blocked while a test is in-flight
  - `src/tui/onboarding/channels.rs`, `src/tui/onboarding_render.rs`, `src/tui/app/dialogs.rs`, `src/tui/app/state.rs`, `src/brain/tools/whatsapp_connect.rs`
- **Clippy `collapsible_match` errors** (`ff66828`) — Collapsed nested `if`-in-`match` arms into match guards across `input.rs` (WhatsApp paste handler) and `markdown.rs` (`Tag::BlockQuote`, `TagEnd::Heading`, `TagEnd::Item`, `Event::HardBreak|SoftBreak`)
  - `src/tui/onboarding/input.rs`, `src/tui/markdown.rs`
- **CI nightly clippy/rustfmt** (`a65c0ab`) — Added `rustfmt` and `clippy` components to `rust-toolchain.toml` so nightly CI jobs resolve the tools without network fallback; pinned workflow to `main` branch trigger
  - `rust-toolchain.toml`, `.github/workflows/`

## [0.2.40] - 2026-02-28

### Added
- **Live plan checklist widget** (`7e1b4db`) — A real-time task panel appears above the input box whenever the agent is executing a plan. Shows plan title, progress bar (`N/M  ████░░  X%`), and per-task status rows (`✓` completed, `▶` in-progress, `·` pending, `✗` failed) with per-status colors. Height is `min(task_count + 2, 8)` rows; zero height when no plan is active. Panel is session-isolated — each session tracks its own plan file (`~/.opencrabs/agents/session/.opencrabs_plan_{uuid}.json`) and reloads on session switch
  - `src/tui/render/plan_widget.rs` (new), `src/tui/render/mod.rs`, `src/tui/app/state.rs`, `src/tui/app/messaging.rs`

### Fixed
- **Live ctx counter during agent tool loops** (`1cb46a9`) — `TokenCountUpdated` events now sync `last_input_tokens` so the `ctx: N/M` display in the status bar ticks up live during streaming and tool execution instead of freezing until `ResponseComplete`
  - `src/tui/app/state.rs`
- **Ctx shows base context on session load and new session** (`1cb46a9`) — Status bar no longer starts at `–` or `0` on a fresh session. It immediately reflects system prompt + tool definition token cost via `base_context_tokens()` (system prompt tokens + tool count × 60)
  - `src/brain/agent/service/builder.rs`, `src/tui/app/messaging.rs`
- **Plan tool auto-approves on finalize** (`9fca3ec`) — `finalize` now sets `PlanStatus::Approved` directly and instructs the agent to begin executing tasks immediately. Previously the tool returned `PendingApproval` and printed "STOP — wait for user response", causing a double-approval (tool dialog + follow-up message) and blocking task execution
  - `src/brain/tools/plan_tool.rs`, `src/brain/prompt_builder.rs`
- **`read_only_mode` dead code removed** (`9fca3ec`) — Remnant field and all callers from the deleted Plan Mode feature purged from `ToolExecutionContext`, tool implementations, `send_message_with_tools_and_mode`, A2A handlers, and tests
  - `src/tui/app/messaging.rs`, `src/tui/app/state.rs`, `src/brain/tools/bash.rs`, `src/brain/tools/edit_file.rs`, `src/brain/tools/write_file.rs`, `src/brain/tools/code_exec.rs`, `src/brain/tools/notebook.rs`
- **MiniMax `</think>` block stripping** (`9b0b8d0`) — MiniMax sometimes closes reasoning blocks with `</think>` instead of `<!-- /reasoning -->`. Extended the think-tag filter to handle this closing variant
  - `src/brain/provider/custom_openai_compatible.rs`

### Changed
- **Complete TUI color overhaul — gray, orange, and cyan palette** (`2796889`, `3d88f11`, `a33fddc`) — All three legacy accent colors replaced for a cohesive warm-neutral scheme:
  - `Color::Blue` / `Rgb(70,130,180)` → `Color::Gray` / `Rgb(120,120,120)` — borders, titles, section headers
  - `Color::Yellow` / `Rgb(184,134,11)` → `Color::Rgb(215,100,20)` muted orange — active/pending states, ctx warning, approval badge
  - `Color::Green` / green-dominant `Rgb` values → `Color::Cyan` / `Rgb(60–80,165–190,165–190)` — success states, completed tasks, diff additions, ctx-ok indicator
  - `src/tui/render/chat.rs`, `src/tui/render/dialogs.rs`, `src/tui/render/help.rs`, `src/tui/render/input.rs`, `src/tui/render/plan_widget.rs`, `src/tui/render/sessions.rs`, `src/tui/render/tools.rs`, `src/tui/onboarding_render.rs`

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

[0.1.2]: https://github.com/adolfousier/opencrabs/releases/tag/v0.1.2

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

[0.1.1]: https://github.com/adolfousier/opencrabs/releases/tag/v0.1.1

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

[0.3.10]: https://github.com/adolfousier/opencrabs/compare/v0.3.10...v0.3.11
[0.3.9]: https://github.com/adolfousier/opencrabs/compare/v0.3.9...v0.3.10
[0.3.8]: https://github.com/adolfousier/opencrabs/releases/tag/v0.3.8
[0.3.7]: https://github.com/adolfousier/opencrabs/releases/tag/v0.3.7
[0.3.6]: https://github.com/adolfousier/opencrabs/releases/tag/v0.3.6
[0.3.5]: https://github.com/adolfousier/opencrabs/releases/tag/v0.3.5
[0.3.4]: https://github.com/adolfousier/opencrabs/releases/tag/v0.3.4
[0.3.3]: https://github.com/adolfousier/opencrabs/releases/tag/v0.3.3
[0.3.2]: https://github.com/adolfousier/opencrabs/releases/tag/v0.3.2
[0.3.1]: https://github.com/adolfousier/opencrabs/releases/tag/v0.3.1
[0.3.0]: https://github.com/adolfousier/opencrabs/releases/tag/v0.3.0
[0.2.99]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.99
[0.2.98]: https://github.com/adolfousier/opencrabs/compare/v0.2.97...v0.2.98
[0.2.97]: https://github.com/adolfousier/opencrabs/compare/v0.2.96...v0.2.97
[0.2.96]: https://github.com/adolfousier/opencrabs/compare/v0.2.95...v0.2.96
[0.2.95]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.95
[0.2.94]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.94
[0.2.93]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.93
[0.2.92]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.92
[0.2.91]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.91
[0.2.90]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.90
[0.2.89]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.89
[0.2.88]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.88
[0.2.87]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.87
[0.2.86]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.86
[0.2.85]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.85
[0.2.84]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.84
[0.2.83]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.83
[0.2.82]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.82
[0.2.81]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.81
[0.2.80]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.80
[0.2.79]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.79
[0.2.78]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.78
[0.2.77]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.77
[0.2.76]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.76
[0.2.75]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.75
[0.2.74]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.74
[0.2.73]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.73
[0.2.72]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.72
[0.2.71]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.71
[0.2.70]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.70
[0.2.69]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.69
[0.2.68]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.68
[0.2.67]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.67
[0.2.66]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.66
[0.2.65]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.65
[0.2.64]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.64
[0.2.63]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.63
[0.2.62]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.62
[0.2.61]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.61
[0.2.60]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.60
[0.2.59]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.59
[0.2.58]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.58
[0.2.57]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.57
[0.2.56]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.56
[0.2.55]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.55
[0.2.54]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.54
[0.2.53]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.53
[0.2.52]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.52
[0.2.51]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.51
[0.2.50]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.50
[0.2.49]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.49
[0.2.48]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.48
[0.2.47]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.47
[0.2.46]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.46
[0.2.45]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.45
[0.2.44]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.44
[0.2.43]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.43
[0.2.42]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.42
[0.2.41]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.41
[0.2.40]: https://github.com/adolfousier/opencrabs/releases/tag/v0.2.40
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

