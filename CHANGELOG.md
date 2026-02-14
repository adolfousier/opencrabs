# Changelog

All notable changes to OpenCrab will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
