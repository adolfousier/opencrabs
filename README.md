[![Rust](https://img.shields.io/badge/rust-%23000000.svg?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Rust Edition](https://img.shields.io/badge/rust-2024_edition-orange.svg)](https://www.rust-lang.org/)
[![Ratatui](https://img.shields.io/badge/ratatui-%23000000.svg?style=for-the-badge&logo=rust&logoColor=white)](https://ratatui.rs)
[![Docker](https://img.shields.io/badge/docker-%23000000.svg?style=for-the-badge&logo=docker&logoColor=white)](https://docker.com)
[![CI](https://github.com/adolfousier/opencrabs/actions/workflows/ci.yml/badge.svg)](https://github.com/adolfousier/opencrabs/actions/workflows/ci.yml)
[![GitHub Stars](https://img.shields.io/github/stars/adolfousier/opencrabs?style=social)](https://github.com/adolfousier/opencrabs)

# 🦀 OpenCrabs

**The autonomous, self-improving AI agent. Single Rust binary. Every channel.**

> Autonomous, self-improving multi-channel AI agent built in Rust. Inspired by [Open Claw](https://github.com/openclaw/openclaw).

```
    ___                    ___           _
   / _ \ _ __  ___ _ _    / __|_ _ __ _| |__  ___
  | (_) | '_ \/ -_) ' \  | (__| '_/ _` | '_ \(_-<
   \___/| .__/\___|_||_|  \___|_| \__,_|_.__//__/
        |_|

 🦀 The autonomous, self-improving AI agent. Single Rust binary. Every channel.

```

**Author:** [Adolfo Usier](https://github.com/adolfousier)

⭐ Star us on [GitHub](https://github.com/adolfousier/opencrabs) if you like what you see!

---

## Why OpenCrabs?

OpenCrabs runs as a **single binary on your terminal** — no server, no gateway, no infrastructure. It makes direct HTTPS calls to LLM providers from your machine. Nothing else leaves your computer.

### OpenCrabs vs Node.js Agent Frameworks

| | **OpenCrabs** (Rust) | **Node.js Frameworks** (e.g. Open Claw) |
|---|---|---|
| **Binary size** | **17–22 MB** single binary, zero dependencies | **1 GB+** `node_modules` with hundreds of transitive packages |
| **Runtime** | None — runs natively | Requires Node.js runtime + npm install |
| **Attack surface** | Zero network listeners. Outbound HTTPS only | Server infrastructure: open ports, auth layers, middleware |
| **API key security** | Keys on your machine only. `zeroize` clears them from RAM on drop, `[REDACTED]` in all debug output | Keys in env vars or config. GC doesn't guarantee memory clearing. Heap dumps can leak secrets |
| **Data residency** | 100% local — SQLite DB, embeddings, brain files, all in `~/.opencrabs/` | Server-side storage, potential multi-tenant data, network transit |
| **Supply chain** | Single compiled binary. Rust's type system prevents buffer overflows, use-after-free, data races at compile time | npm ecosystem: typosquatting, dependency confusion, prototype pollution |
| **Memory safety** | Compile-time guarantees — no GC, no null pointers, no data races | GC-managed, prototype pollution, type coercion bugs |
| **Concurrency** | tokio async + Rust ownership = zero data races guaranteed | Single-threaded event loop, worker threads share memory unsafely |
| **Native TTS/STT** | Built-in local speech-to-text (whisper.cpp) and text-to-speech — ~130 MB total stack, fully offline | No native voice. Requires external APIs (Google, AWS, Azure) or heavy Python dependencies (PyTorch, ~5 GB+) |
| **Telemetry** | Zero. No analytics, no tracking, no remote logging | Server infra typically includes monitoring, logging pipelines, APM |

### What stays local (never leaves your machine)

- All chat sessions and messages (SQLite)
- Tool executions (bash, file reads/writes, git)
- Memory and embeddings (local vector search)
- Voice transcription in local STT mode (whisper.cpp, on-device)
- Brain files, config, API keys

### What goes out (only when you use it)

- Your messages to the LLM provider API (Anthropic, OpenAI, GitHub Copilot, etc.)
- Web search queries (optional tool)
- GitHub API via `gh` CLI (optional tool)
- Browser automation (optional, `browser` feature — auto-detects Chromium-based browsers via CDP, not Firefox)
- Dynamic tool HTTP requests (only when you define HTTP tools in `tools.toml`)

---

## Table of Contents

- [Screenshots](#-screenshots)
- [Why OpenCrabs?](#why-opencrabs)
- [Core Features](#-core-features)
- [CLI Commands](#cli)
- [Supported AI Providers](#-supported-ai-providers)
- [Agent-to-Agent (A2A) Protocol](#-agent-to-agent-a2a-protocol)
- [Quick Start](#-quick-start)
- [Onboarding Wizard](#-onboarding-wizard)
- [API Keys (keys.toml)](#-api-keys-keystoml)
- [Configuration (config.toml)](#-configuration-configtoml)
- [Commands (commands.toml)](#-commands-commandstoml)
- [Dynamic Tools (tools.toml)](#-dynamic-tools-toolstoml)
- [Using Local LLMs](#-using-local-llms)
- [Configuration](#-configuration)
- [Tool System](#-tool-system)
- [Keyboard Shortcuts](#-keyboard-shortcuts)
- [Debug and Logging](#-debug-and-logging)
- [Cron Jobs & Heartbeats](#-cron-jobs--heartbeats)
- [Architecture](#-architecture)
- [Project Structure](#-project-structure)
- [Development](#-development)
- [Platform Notes](#-platform-notes)
- [Troubleshooting](#-troubleshooting)
- [Companion Tools](#-companion-tools)
- [Disclaimers](#-disclaimers)
- [Contributing](#-contributing)
- [License](#-license)
- [Acknowledgments](#-acknowledgments)

---

## 📸 Screenshots

https://github.com/user-attachments/assets/7f45c5f8-acdf-48d5-b6a4-0e4811a9ee23

---

## 🎯 Core Features

### AI & Providers
| Feature | Description |
|---------|-------------|
| **Multi-Provider** | Anthropic Claude, OpenAI, GitHub Copilot (uses your Copilot subscription), OpenRouter (400+ models), MiniMax, Google Gemini, z.ai GLM (General API + Coding API), Claude CLI, OpenCode CLI, and any OpenAI-compatible API (Ollama, LM Studio, LocalAI). Model lists fetched live from provider APIs — new models available instantly. Each session remembers its provider + model and restores it on switch |
| **Fallback Providers** | Configure a chain of fallback providers — if the primary fails, each fallback is tried in sequence automatically. Any configured provider can be a fallback. Config: `[providers.fallback] providers = ["openrouter", "anthropic"]` |
| **Per-Provider Vision** | Set `vision_model` per provider — the LLM calls `analyze_image` as a tool, which uses the vision model on the same provider API to describe images. The chat model stays the same and gets vision capability via tool call. Gemini vision takes priority when configured. Auto-configured for known providers (e.g. MiniMax) on first run |
| **Real-time Streaming** | Character-by-character response streaming with animated spinner showing model name and live text |
| **Claude CLI** | Use your Claude Code CLI directly via the `claude` CLI — no additional setup. Just install Claude Code, authenticate, and select Claude CLI as your provider |
| **Local LLM Support** | Run with LM Studio, Ollama, or any OpenAI-compatible endpoint — 100% private, zero-cost |
| **Cost Tracking** | Per-message token count and cost displayed in header; `/usage` shows all-time breakdown grouped by model with real costs + estimates for historical sessions |
| **Context Awareness** | Live context usage indicator showing actual token counts (e.g. `ctx: 45K/200K (23%)`); auto-compaction at 70% with tool overhead budgeting; accurate tiktoken-based counting calibrated against API actuals |
| **3-Tier Memory** | (1) **Brain MEMORY.md** — user-curated durable memory loaded every turn, (2) **Daily Logs** — auto-compaction summaries at `~/.opencrabs/memory/YYYY-MM-DD.md`, (3) **Hybrid Memory Search** — FTS5 keyword search + local vector embeddings (embeddinggemma-300M, 768-dim) combined via Reciprocal Rank Fusion. Runs entirely local — no API key, no cost, works offline |
| **Dynamic Brain System** | System brain assembled from workspace MD files (SOUL, IDENTITY, USER, AGENTS, TOOLS, MEMORY) — all editable live between turns |
| **Multi-Agent Orchestration** | Spawn typed child agents (General, Explore, Plan, Code, Research) for parallel task execution. Five tools: `spawn_agent`, `wait_agent`, `send_input`, `close_agent`, `resume_agent`. Each type gets a role-specific system prompt and filtered tool registry. Configurable subagent provider/model. Children run in isolated sessions with auto-approve — no recursive spawning |

### Multimodal Input
| Feature | Description |
|---------|-------------|
| **Image Attachments** | Paste image paths or URLs into the input — auto-detected and attached as vision content blocks for multimodal models |
| **PDF Support** | Attach PDF files by path — native Anthropic PDF support; for other providers, text is extracted locally via `pdf-extract` |
| **Document Parsing** | Built-in `parse_document` tool extracts text from PDF, DOCX, HTML, TXT, MD, JSON, XML |
| **Voice (STT)** | Voice notes transcribed via **API** (Groq Whisper `whisper-large-v3-turbo`) or **Local** (whisper.cpp via `whisper-rs`, runs on-device). Choose mode in `/onboard:voice`. Local mode: select model size (Tiny 75 MB / Base 142 MB / Small 466 MB / Medium 1.5 GB), download from HuggingFace, zero API cost. Included by default |
| **Voice (TTS)** | Agent replies to voice notes with audio via **API** (OpenAI TTS `gpt-4o-mini-tts`) or **Local** (Piper TTS, runs on-device via Python venv). Choose mode in `/onboard:voice`. Local mode: select voice (Ryan / Amy / Lessac / Kristin / Joe / Cori), auto-downloads from HuggingFace, zero API cost. Falls back to text if disabled |
| **Attachment Indicator** | Attached images show as `[IMG1:filename.png]` in the input title bar |
| **Image Generation** | Agent generates images via Google Gemini (`gemini-3.1-flash-image-preview` "Nano Banana") using the `generate_image` tool — enabled via `/onboard:image`. Returned as native images/attachments in all channels |

### Messaging Integrations
| Feature | Description |
|---------|-------------|
| **Telegram Bot** | Full-featured Telegram bot — owner DMs share TUI session, groups get isolated per-group sessions (keyed by chat ID). Photo/voice support, allowed user IDs, allowed chat/group IDs, `respond_to` filter (`all`/`dm_only`/`mention`). Passive group message capture — all messages stored for context even when bot isn't mentioned |
| **WhatsApp** | Connect via QR code pairing at runtime or from onboarding wizard. Text + image, shared session with TUI, phone allowlist (`allowed_phones`), session persists across restarts |
| **Discord** | Full Discord bot — text + image + voice. Owner DMs share TUI session, guild channels get isolated per-channel sessions. Allowed user IDs, allowed channel IDs, `respond_to` filter. Full proactive control via `discord_send` (17 actions): `send`, `reply`, `react`, `unreact`, `edit`, `delete`, `pin`, `unpin`, `create_thread`, `send_embed`, `get_messages`, `list_channels`, `add_role`, `remove_role`, `kick`, `ban`, `send_file`. Generated images sent as native Discord file attachments |
| **Slack** | Full Slack bot via Socket Mode — owner DMs share TUI session, channels get isolated per-channel sessions. Allowed user IDs, allowed channel IDs, `respond_to` filter. Full proactive control via `slack_send` (17 actions): `send`, `reply`, `react`, `unreact`, `edit`, `delete`, `pin`, `unpin`, `get_messages`, `get_channel`, `list_channels`, `get_user`, `list_members`, `kick_user`, `set_topic`, `send_blocks`, `send_file`. Generated images sent as native Slack file uploads. Bot token + app token from `api.slack.com/apps` (Socket Mode required). **Required Bot Token Scopes:** `chat:write`, `channels:history`, `groups:history`, `im:history`, `mpim:history`, `users:read`, `files:read`, `files:write`, `reactions:write`, `app_mentions:read` |
| **Trello** | Tool-only by default — the AI acts on Trello only when explicitly asked via `trello_send`. Opt-in polling via `poll_interval_secs` in config; when enabled, only `@bot_username` mentions from allowed users trigger a response. Full card management via `trello_send` (22 actions): `add_comment`, `create_card`, `move_card`, `find_cards`, `list_boards`, `get_card`, `get_card_comments`, `update_card`, `archive_card`, `add_member_to_card`, `remove_member_from_card`, `add_label_to_card`, `remove_label_from_card`, `add_checklist`, `add_checklist_item`, `complete_checklist_item`, `list_lists`, `get_board_members`, `search`, `get_notifications`, `mark_notifications_read`, `add_attachment`. API Key + Token from `trello.com/power-ups/admin`, board IDs and member-ID allowlist configurable |

#### File & Media Input Support

When users send files, images, or documents across any channel, the agent receives the content automatically — no manual forwarding needed. Example: a user uploads a dashboard screenshot to a Trello card with the comment _"I'm seeing this error"_ — the agent fetches the attachment, passes it through the vision pipeline, and responds with full context.

| Channel | Images (in) | Text files (in) | Documents (in) | Audio (in) | Image gen (out) |
|---------|-------------|-----------------|----------------|------------|-----------------|
| **Telegram** | ✅ vision pipeline | ✅ extracted inline | ✅ / PDF note | ✅ STT | ✅ native photo |
| **WhatsApp** | ✅ vision pipeline | ✅ extracted inline | ✅ / PDF note | ✅ STT | ✅ native image |
| **Discord** | ✅ vision pipeline | ✅ extracted inline | ✅ / PDF note | ✅ STT | ✅ file attachment |
| **Slack** | ✅ vision pipeline | ✅ extracted inline | ✅ / PDF note | ✅ STT | ✅ file upload |
| **Trello** | ✅ card attachments → vision | ✅ extracted inline | — | — | ✅ card attachment + embed |
| **TUI** | ✅ paste path → vision | ✅ paste path → inline | — | ✅ STT | ✅ `[IMG: name]` display |

Images are passed to the active model's vision pipeline if it supports multimodal input, or routed to the `analyze_image` tool (Google Gemini vision) otherwise. Text files (`.txt`, `.md`, `.json`, `.csv`, source code, etc.) are extracted as UTF-8 and included inline up to 8 000 characters — in the TUI simply paste or type the file path.

### Terminal UI
| Feature | Description |
|---------|-------------|
| **Cursor Navigation** | Full cursor movement: Left/Right arrows, Ctrl+Left/Right word jump, Home/End, Delete, Backspace at position |
| **Input History** | Persistent command history (`~/.opencrabs/history.txt`), loaded on startup, capped at 500 entries |
| **Inline Tool Approval** | Claude Code-style `❯ Yes / Always / No` selector with arrow key navigation |
| **Inline Plan Approval** | Interactive plan review selector (Approve / Reject / Request Changes / View Plan) |
| **Session Management** | Create, rename, delete sessions with persistent SQLite storage; each session remembers its provider + model — switching sessions auto-restores the provider (no manual `/models` needed); token counts and context % per session |
| **Split Panes** | Horizontal (`\|` in sessions) and vertical (`_` in sessions) pane splitting — tmux-style. Each pane runs its own session with independent provider, model, and context. Run 10 sessions side by side, all processing in parallel. `Tab` to cycle focus, `Ctrl+X` to close pane |
| **Parallel Sessions** | Multiple sessions can have in-flight requests to different providers simultaneously. Send a message in one session, switch to another, send another — both process in parallel. Background sessions auto-approve tool calls; you'll see results when you switch back |
| **Scroll While Streaming** | Scroll up during streaming without being yanked back to bottom; auto-scroll re-enables when you scroll back down or send a message |
| **Compaction Summary** | Auto-compaction shows the full summary in chat as a system message — see exactly what the agent remembered |
| **Syntax Highlighting** | 100+ languages with line numbers via syntect |
| **Markdown Rendering** | Rich text formatting with code blocks, headings, lists, and inline styles |
| **Tool Context Persistence** | Tool call groups saved to DB and reconstructed on session reload — no vanishing tool history |
| **Multi-line Input** | Alt+Enter / Shift+Enter for newlines; Enter to send |
| **Abort Processing** | Escape×2 within 3 seconds to cancel any in-progress request |

### Agent Capabilities
| Feature | Description |
|---------|-------------|
| **Full Terminal Access** | 30+ built-in tools (file I/O, glob, grep, web search, code execution, image gen/analysis, memory search, cron jobs) plus **any CLI tool on your system** via `bash` — GitHub CLI, Docker, SSH, Python, Node, ffmpeg, curl, and everything else just work |
| **Per-Session Isolation** | Each session is an independent agent with its own provider, model, context, and tool state. Sessions can run tasks in parallel against different providers — ask Claude a question in one session while Kimi works on code in another |
| **Self-Sustaining** | Agent can modify its own source, build, test, and hot-restart via Unix `exec()` |
| **Self-Improving** | Learns from experience — saves reusable workflows as custom commands, writes lessons learned to memory, updates its own brain files. All local, no data leaves your machine |
| **Dynamic Tools** | Define custom tools at runtime via `~/.opencrabs/tools.toml` — the agent can call them autonomously like built-in tools. HTTP and shell executors, template parameters (`{{param}}`), enable/disable without restart. The `tool_manage` meta-tool lets the agent create, remove, and reload tools on the fly |
| **Browser Automation** | Native browser control via CDP (Chrome DevTools Protocol). Auto-detects your default Chromium-based browser (Chrome, Brave, Edge, Arc, Vivaldi, Opera, Chromium) and uses its profile — your logins, cookies, and extensions carry over. 7 browser tools: navigate, click, type, screenshot, eval JS, extract content, wait for elements. Headed or headless mode with display auto-detection. **Note:** Firefox is not supported (no CDP) — if Firefox is your default, OpenCrabs falls back to the first available Chromium browser. Feature-gated under `browser` (included by default) |
| **Natural Language Commands** | Tell OpenCrabs to create slash commands — it writes them to `commands.toml` autonomously via the `config_manager` tool |
| **Live Settings** | Agent can read/write `config.toml` at runtime; Settings TUI screen (press `S`) shows current config; approval policy persists across restarts. Default: auto-approve (use `/approve` to change) |
| **Web Search** | DuckDuckGo (built-in, no key needed) + EXA AI (neural, free via MCP) by default; Brave Search optional (key in `keys.toml`) |
| **Debug Logging** | `--debug` flag enables file logging; `DEBUG_LOGS_LOCATION` env var for custom log directory |
| **Agent-to-Agent (A2A)** | HTTP gateway implementing A2A Protocol RC v1.0 — peer-to-peer agent communication via JSON-RPC 2.0. Supports `message/send`, `message/stream` (SSE), `tasks/get`, `tasks/cancel`. Built-in `a2a_send` tool lets the agent proactively call remote A2A agents. Optional Bearer token auth. Includes multi-agent debate (Bee Colony) with confidence-weighted consensus. Task persistence across restarts |
| **Profiles** | Run multiple isolated instances from the same installation. Each profile gets its own config, keys, memory, sessions, and database. Create with `opencrabs profile create <name>`, switch with `-p <name>`. Migrate config between profiles with `profile migrate`. Export/import for sharing. Token-lock isolation prevents two profiles from using the same bot credential |

### CLI
| Command | Description |
|---------|-------------|
| `opencrabs` | Launch interactive TUI (default) |
| `opencrabs chat` | Launch TUI with optional `--session <id>` to resume, `--onboard` to force wizard |
| `opencrabs run <prompt>` | Execute a single prompt non-interactively. `--auto-approve` / `--yolo` for unattended. `--format text\|json\|markdown` |
| `opencrabs agent` | Interactive CLI agent — multi-turn conversation in your terminal, no TUI. `-m <msg>` for single-message mode |
| `opencrabs status` | System overview: version, provider, channels, database, brain, cron, dynamic tools |
| `opencrabs doctor` | Full diagnostics: config, provider connectivity, database, brain, channels, CLI tools in PATH |
| `opencrabs init` | Initialize configuration (`--force` to overwrite) |
| `opencrabs config` | Show current configuration (`--show-secrets` to reveal keys) |
| `opencrabs onboard` | Run the onboarding setup wizard |
| `opencrabs channel list` | List all configured channels with enabled/disabled status |
| `opencrabs channel doctor` | Run health checks on all enabled channels |
| `opencrabs memory list` | List brain files and memory entries |
| `opencrabs memory get <name>` | Show contents of a specific memory or brain file |
| `opencrabs memory stats` | Memory statistics: file count, total size, entry count |
| `opencrabs session list` | List all sessions with provider, model, token count (`--all` includes archived) |
| `opencrabs session get <id>` | Show session details and recent messages |
| `opencrabs db init` | Initialize database |
| `opencrabs db stats` | Show database statistics |
| `opencrabs db clear` | Clear all sessions and messages (`--force` to skip confirmation) |
| `opencrabs cron add\|list\|remove\|enable\|disable\|test` | Manage scheduled cron jobs |
| `opencrabs logs status\|view\|clean\|open` | Log management |
| `opencrabs service install\|start\|stop\|restart\|status\|uninstall` | OS service management (launchd on macOS, systemd on Linux) |
| `opencrabs daemon` | Run in headless daemon mode — channels only, no TUI |
| `opencrabs completions <shell>` | Generate shell completions (bash, zsh, fish, powershell) |
| `opencrabs version` | Print version and exit |

Global flags: `--debug` (enable file logging), `--config <path>` (custom config file), `--profile <name>` / `-p <name>` (run as a named profile).

### Profiles — Multi-Instance Crab Agents

Run multiple isolated OpenCrabs instances from the same installation. Each profile gets its own config, brain files, memory, sessions, database, and gateway service.

| Command | Description |
|---------|-------------|
| `opencrabs profile create <name>` | Create a new profile with fresh config and brain files |
| `opencrabs profile list` | List all profiles with last-used timestamps |
| `opencrabs profile delete <name>` | Delete a profile and all its data |
| `opencrabs profile export <name> -o profile.tar.gz` | Export a profile as a portable archive |
| `opencrabs profile import profile.tar.gz` | Import a profile from an archive |
| `opencrabs profile migrate --from <name> --to <name>` | Copy config and brain files between profiles (no DB or sessions) |
| `opencrabs -p <name>` | Launch OpenCrabs as the specified profile |

**Default profile:** `~/.opencrabs/` — works exactly as before. No migration needed. Users who never touch profiles see zero difference.

**Named profiles** live at `~/.opencrabs/profiles/<name>/` with full isolation:
```
~/.opencrabs/
├── config.toml          # default profile
├── opencrabs.db
├── profiles.toml        # profile registry
├── locks/               # token-lock files
└── profiles/
    ├── hermes/          # named profile
    │   ├── config.toml
    │   ├── keys.toml
    │   ├── opencrabs.db
    │   ├── SOUL.md
    │   └── memory/
    └── scout/
        └── ...
```

**Token-lock isolation:** Two profiles cannot use the same bot credential (Telegram token, Discord token, etc.). On startup, each profile acquires a lock on its channel tokens. If another profile already holds the lock, the channel refuses to start — preventing two instances from fighting over the same bot.

**Profile migration:** Use `opencrabs profile migrate --from default --to hermes` to copy all `.md` brain files, `.toml` config files, and `memory/` entries to a new profile. Sessions and database are not copied — the new profile starts clean. Add `--force` to overwrite existing files in the target profile. After migrating, customize the new profile's `SOUL.md`, `IDENTITY.md`, and `config.toml` to give it a different personality and provider setup.

### Daemon & Service

Run profiles as background services:

```bash
# Install as system service (macOS launchd / Linux systemd)
opencrabs -p hermes service install
opencrabs -p hermes service start

# Each profile gets its own service
# macOS: com.opencrabs.daemon.hermes
# Linux: opencrabs-hermes.service

# Manage independently
opencrabs -p hermes service status
opencrabs -p hermes service stop
opencrabs -p hermes service uninstall
```

Multiple profiles can run as simultaneous daemon services with full isolation.

**Environment variable:** Set `OPENCRABS_PROFILE=hermes` to select a profile without the `-p` flag. Useful for systemd services, cron jobs, and daemon mode.

---

## 🌐 Supported AI Providers

| Provider | Auth | Models | Streaming | Tools | Notes |
|----------|------|--------|-----------|-------|-------|
| [Anthropic Claude](#anthropic-claude) | API key | Claude Opus 4.6, Sonnet 4.5, Haiku 4.5+ | ✅ | ✅ | Cost tracking, automatic retry |
| [OpenAI](#openai) | API key | GPT-5 Turbo, GPT-5 | ✅ | ✅ | |
| [GitHub Copilot](#github-copilot) | OAuth | GPT-4o, Claude Sonnet 4+ | ✅ | ✅ | Uses your Copilot subscription — no API charges |
| [OpenRouter](#openrouter--400-models-one-key) | API key | 400+ models | ✅ | ✅ | Free models available (DeepSeek-R1, Llama 3.3, etc.) |
| [Google Gemini](#google-gemini) | API key | Gemini 2.5 Flash, 2.0, 1.5 Pro | ✅ | ✅ | 1M+ context, vision, image generation |
| [MiniMax](#minimax) | API key | M2.7, M2.5, M2.1, Text-01 | ✅ | ✅ | Competitive pricing, auto-configured vision |
| [z.ai GLM](#zai-glm) | API key | GLM-4.5 through GLM-5 Turbo | ✅ | ✅ | General API + Coding API endpoints |
| [Claude CLI](#claude-code-cli) | CLI auth | Via `claude` binary | ✅ | ✅ | Uses your Claude Code subscription |
| [OpenCode CLI](#opencode-cli) | None | Free models (Mimo, etc.) | ✅ | ✅ | Free — no API key or subscription needed |
| [Custom](#custom-openai-compatible) | Optional | Any | ✅ | ✅ | Ollama, LM Studio, Groq, NVIDIA, any OpenAI-compatible API |

### Anthropic Claude

**Models:** `claude-opus-4-6`, `claude-sonnet-4-5-20250929`, `claude-haiku-4-5-20251001`, plus legacy Claude 3.x models

**Setup** in `keys.toml`:
```toml
[providers.anthropic]
api_key = "sk-ant-api03-YOUR_KEY"
```

> **OAuth tokens no longer supported.** Anthropic disabled OAuth (`sk-ant-oat`) for third-party apps as of Feb 2026. Only console API keys (`sk-ant-api03-*`) work. See [anthropics/claude-code#28091](https://github.com/anthropics/claude-code/issues/28091).

**Features:** Streaming, tools, cost tracking, automatic retry with backoff

#### Claude Code CLI 

Use your Claude Code CLI. OpenCrabs spawns the local `claude` CLI for completion.

**Setup:**
1. Install [Claude Code CLI](https://github.com/anthropics/claude-code) and authenticate (`claude login`)
2. Enable in `config.toml`:
```toml
[providers.claude_cli]
enabled = true
```

OpenCrabs handles all tools, memory, and context locally — the CLI is just the LLM backend.

### OpenAI

**Models:** GPT-5 Turbo, GPT-5

**Setup** in `keys.toml`:
```toml
[providers.openai]
api_key = "sk-YOUR_KEY"
```

### GitHub Copilot

**Use your GitHub Copilot subscription** — no API charges, no tokens to manage. OpenCrabs authenticates via the same OAuth device flow used by VS Code and other Copilot tools.

**Setup** — select GitHub Copilot in the onboarding wizard and press Enter. You'll see a one-time code to enter at [github.com/login/device](https://github.com/login/device). Once authorized, models are fetched from the Copilot API automatically.

**Requirements:** An active [GitHub Copilot](https://github.com/features/copilot) subscription (Individual, Business, or Enterprise).

<details><summary>Manual config (without wizard)</summary>

The OAuth token is saved automatically during onboarding. If you need to re-authenticate, run `/onboard:provider` and select GitHub Copilot.

Enable in `config.toml`:
```toml
[providers.github]
enabled = true
default_model = "gpt-4o"
base_url = "https://api.githubcopilot.com/chat/completions"
```
</details>

**Features:** Streaming, tools, OpenAI-compatible API at `api.githubcopilot.com`. Copilot-specific headers (`copilot-integration-id`, `editor-version`) are injected automatically. Short-lived API tokens are refreshed in the background every ~25 minutes.

### OpenRouter — 400+ Models, One Key

**Setup** in `keys.toml` — get a key at [openrouter.ai/keys](https://openrouter.ai/keys):
```toml
[providers.openrouter]
api_key = "sk-or-YOUR_KEY"
```

Access 400+ models from every major provider through a single API key — Anthropic, OpenAI, Google, Meta, Mistral, DeepSeek, Qwen, and many more. Includes **free models** (DeepSeek-R1, Llama 3.3, Gemma 2, Mistral 7B) and stealth/preview models as they drop.

Model list is **fetched live** from the OpenRouter API during onboarding and via `/models` — no binary update needed when new models are added.

### Google Gemini

**Models:** `gemini-2.5-flash`, `gemini-2.0-flash`, `gemini-1.5-pro` — fetched live from the Gemini API

**Setup** in `keys.toml` — get a key at [aistudio.google.com](https://aistudio.google.com):
```toml
[providers.gemini]
api_key = "AIza..."
```

Enable and set default model in `config.toml`:
```toml
[providers.gemini]
enabled = true
default_model = "gemini-2.5-flash"
```

**Features:** Streaming, tool use, vision, 1M+ token context window, live model list from `/models` endpoint

> **Image generation & vision:** Gemini also powers the separate `[image]` section for `generate_image` and `analyze_image` agent tools. See [Image Generation & Vision](#-image-generation--vision) below.

### MiniMax

**Models:** `MiniMax-M2.7`, `MiniMax-M2.5`, `MiniMax-M2.1`, `MiniMax-Text-01`

**Setup** — get your API key from [platform.minimax.io](https://platform.minimax.io). Add to `keys.toml`:

```toml
[providers.minimax]
api_key = "your-api-key"
```

MiniMax is an OpenAI-compatible provider with competitive pricing. It does not expose a `/models` endpoint, so the model list comes from `config.toml` (pre-configured with available models).

### z.ai GLM

**Models:** `glm-4.5`, `glm-4.5-air`, `glm-4.6`, `glm-4.7`, `glm-5`, `glm-5-turbo` — fetched live from the z.ai API

**Setup** — get your API key from [open.bigmodel.cn](https://open.bigmodel.cn). Add to `keys.toml`:

```toml
[providers.zhipu]
api_key = "your-api-key"
```

Enable and choose endpoint type in `config.toml`:
```toml
[providers.zhipu]
enabled = true
default_model = "glm-4.7"
endpoint_type = "api"  # "api" (General API) or "coding" (Coding API)
```

z.ai GLM (Zhipu AI) offers two endpoint types selectable during onboarding or via `/models`:
- **General API** (`api`) — standard chat completions at `open.bigmodel.cn/api/paas/v4`
- **Coding API** (`coding`) — code-optimized endpoint at `codeapi.bigmodel.cn/api/paas/v4`

Both use the same API key and model names. The endpoint type can be toggled in the onboarding wizard or `/models` dialog.

**Features:** Streaming, tools, OpenAI-compatible API, live model list from `/models` endpoint

### OpenCode CLI

Use the [OpenCode](https://github.com/opencode-ai/opencode) CLI as a free LLM backend — no API key or subscription needed. OpenCrabs spawns the local `opencode` binary for completions.

**Setup:**
1. Install [OpenCode CLI](https://github.com/opencode-ai/opencode) (`go install github.com/opencode-ai/opencode@latest` or download from releases)
2. Enable in `config.toml`:
```toml
[providers.opencode_cli]
enabled = true
default_model = "opencode/mimo-v2-pro-free"
```

Models are fetched live from `opencode models`. Free models like `mimo-v2-pro-free` work without any authentication.

**Features:** Streaming, tools, extended thinking support, NDJSON event protocol

### Custom (OpenAI-Compatible)

**Use for:** Ollama, LM Studio, LocalAI, Groq, or any OpenAI-compatible API.

**Setup** in `config.toml` — every custom provider needs a name (the label after `custom.`):

```toml
[providers.custom.lm_studio]
enabled = true
base_url = "http://localhost:1234/v1"  # or your endpoint
default_model = "qwen2.5-coder-7b-instruct"
# Optional: list your available models — shows up in /models and /onboard
# so you can switch between them without editing config
models = ["qwen2.5-coder-7b-instruct", "llama-3-8B", "mistral-7B-instruct"]
```

> **Local LLMs (Ollama, LM Studio):** No API key needed — just set `base_url` and `default_model`.
>
> **Remote APIs (Groq, Together, etc.):** Add the key in `keys.toml` using the same name:
> ```toml
> [providers.custom.groq]
> api_key = "your-api-key"
> ```

> **Note:** `/chat/completions` is auto-appended to base URLs that don't include it.

**Multiple custom providers** coexist — define as many as you need with different names and switch between them via `/models`:

```toml
[providers.custom.lm_studio]
enabled = true
base_url = "http://localhost:1234/v1"
default_model = "qwen2.5-coder-7b-instruct"

[providers.custom.ollama]
enabled = false
base_url = "http://localhost:11434/v1"
default_model = "mistral"
```

The name after `custom.` is a label you choose (e.g. `lm_studio`, `nvidia`, `groq`). The one with `enabled = true` is active. Keys go in `keys.toml` using the same label. All configured custom providers persist — switching via `/models` just toggles `enabled`.

#### Free Prototyping with NVIDIA API + Kimi K2.5

[Kimi K2.5](https://build.nvidia.com/moonshotai/kimi-k2.5) is a frontier-scale multimodal Mixture-of-Experts (MoE) model available **for free** on the NVIDIA API Catalog — no billing setup or credit card required. It handles complex reasoning and image/video understanding, making it a strong free alternative to paid models like Claude or Gemini for experimentation and agentic workflows.

**Tested and verified** with OpenCrabs Custom provider setup.

**Quick start:**

1. Sign up at the [NVIDIA API Catalog](https://build.nvidia.com/) and verify your account
2. Go to the [Kimi K2.5 model page](https://build.nvidia.com/moonshotai/kimi-k2.5) and click **Get API Key** (or "View Code" to see an auto-generated key)
3. Configure in OpenCrabs via `/models` or `config.toml`:

```toml
[providers.custom.nvidia]
enabled = true
base_url = "https://integrate.api.nvidia.com/v1"
default_model = "moonshotai/kimi-k2.5"
```

```toml
# keys.toml
[providers.custom.nvidia]
api_key = "nvapi-..."
```

**Provider priority:** MiniMax > OpenRouter > Anthropic > OpenAI > GitHub Copilot > Gemini > z.ai GLM > Claude CLI > OpenCode CLI > Custom. The first provider with `enabled = true` is used on new sessions. Each provider has its own API key in `keys.toml` — no sharing or confusion.

**Per-session provider:** Each session remembers which provider and model it was using. Switch to Claude in one session, Kimi in another — when you `/sessions` switch between them, the provider restores automatically. No need to `/models` every time. New sessions inherit the current provider.

### Fallback Providers

If your primary provider goes down, fallback providers are tried automatically in sequence. Any provider with API keys already configured can be a fallback:

```toml
[providers.fallback]
enabled = true
providers = ["openrouter", "anthropic"]  # tried in order on failure
```

At runtime, if a request to the primary fails, each fallback is tried until one succeeds. Supports single (`provider = "openrouter"`) or multiple providers.

### Per-Provider Vision Model

If your default model doesn't support vision but another model on the same provider does, set `vision_model`. The LLM calls `analyze_image` as a tool — the vision model describes the image and returns the description to the chat model as context:

```toml
[providers.minimax]
default_model = "MiniMax-M2.7"
vision_model = "MiniMax-Text-01"  # describes images for the chat model
```

MiniMax auto-configures this on first run. Works with any provider — just set `vision_model` to a vision-capable model on the same API.

---

## 🖼️ Image Generation & Vision

OpenCrabs supports image generation and vision analysis via Google Gemini. These features are independent of the main chat provider — you can use Claude for chat and Gemini for images.

### Setup

1. Get a free API key from [aistudio.google.com](https://aistudio.google.com)
2. Run `/onboard:image` in chat (or go through onboarding Advanced mode) to configure
3. Or add manually to `keys.toml`:

```toml
[image]
api_key = "AIza..."
```

And `config.toml`:
```toml
[image.generation]
enabled = true
model = "gemini-3.1-flash-image-preview"

[image.vision]
enabled = true
model = "gemini-3.1-flash-image-preview"
```

### Agent Tools

When enabled, two tools become available to the agent automatically:

| Tool | Description |
|------|-------------|
| `generate_image` | Generate an image from a text prompt — saves to `~/.opencrabs/images/` and returns the file path |
| `analyze_image` | Analyze an image file or URL via Gemini vision — works even when your main model doesn't support vision |

**Example prompts:**
- _"Generate a pixel art crab logo"_ → agent calls `generate_image`, returns file path
- _"What's in this image: /tmp/screenshot.png"_ → agent calls `analyze_image` via Gemini

### Model

Both tools use `gemini-3.1-flash-image-preview` ("Nano Banana") — Gemini's dedicated image-generation model that supports both vision input and image output in a single request.

---

## 🤝 Agent-to-Agent (A2A) Protocol

OpenCrabs includes a built-in A2A gateway — an HTTP server implementing the [A2A Protocol RC v1.0](https://google.github.io/A2A/) for peer-to-peer agent communication. Other A2A-compatible agents can discover OpenCrabs, send it tasks, and get results back — all via standard JSON-RPC 2.0. The agent can also proactively call remote A2A agents using the built-in `a2a_send` tool.

### Enabling A2A

Add to `~/.opencrabs/config.toml`:

```toml
[a2a]
enabled = true
bind = "127.0.0.1"   # Loopback only (default) — use "0.0.0.0" to expose
port = 18790          # Gateway port
# api_key = "your-secret"  # Optional Bearer token auth for incoming requests
# allowed_origins = ["http://localhost:3000"]  # CORS (empty = blocked)
```

### Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/.well-known/agent.json` | GET | Agent Card — discover skills, capabilities, supported content types |
| `/a2a/v1` | POST | JSON-RPC 2.0 — `message/send`, `message/stream` (SSE), `tasks/get`, `tasks/cancel` |
| `/a2a/health` | GET | Health check |

### `a2a_send` Tool

The agent has a built-in `a2a_send` tool to communicate with remote A2A agents:

| Action | Description |
|--------|-------------|
| `discover` | Fetch a remote agent's Agent Card (no approval needed) |
| `send` | Send a task message to a remote agent |
| `get` | Check status of a remote task |
| `cancel` | Cancel a running remote task |

The tool supports optional `api_key` for authenticated endpoints and `context_id` for multi-turn conversations.

### Connecting Two Agents

**VPS agent** (`config.toml`):
```toml
[a2a]
enabled = true
bind = "0.0.0.0"
port = 18790
api_key = "shared-secret"
```

**Local agent** — connect via SSH tunnel (recommended, no ports to open):
```bash
ssh -L 18791:127.0.0.1:18790 user@your-vps
```

Now the local agent can reach the VPS agent at `http://127.0.0.1:18791`. The agent will use `a2a_send` with that URL automatically.

### Quick Start Examples

```bash
# Discover the agent
curl http://127.0.0.1:18790/.well-known/agent.json | jq .

# Send a message (creates a task)
curl -X POST http://127.0.0.1:18790/a2a/v1 \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-secret" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "message/send",
    "params": {
      "message": {
        "role": "user",
        "parts": [{"text": "What tools do you have?"}]
      }
    }
  }'

# Poll a task by ID
curl -X POST http://127.0.0.1:18790/a2a/v1 \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-secret" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tasks/get","params":{"id":"TASK_ID"}}'

# Cancel a running task
curl -X POST http://127.0.0.1:18790/a2a/v1 \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-secret" \
  -d '{"jsonrpc":"2.0","id":3,"method":"tasks/cancel","params":{"id":"TASK_ID"}}'
```

### Bee Colony Debate

OpenCrabs supports multi-agent structured debate via the **Bee Colony** protocol — based on [ReConcile (ACL 2024)](https://arxiv.org/abs/2309.13007) confidence-weighted voting. Multiple "bee" agents argue across configurable rounds, each enriched with knowledge context from QMD memory search, then converge on a consensus answer with confidence scores.

### Security & Persistence

- **Loopback only** by default — binds to `127.0.0.1`, not `0.0.0.0`
- **Bearer token auth** — set `api_key` to require `Authorization: Bearer <key>` on all JSON-RPC requests
- **CORS locked down** — no cross-origin requests unless `allowed_origins` is explicitly set
- **Task persistence** — active tasks survive restarts via SQLite
- For public exposure, use a reverse proxy (nginx/Caddy) with TLS + the `api_key` auth

---

## 🚀 Quick Start

### Option 1: Download Binary (just run it)

Grab a pre-built binary from [GitHub Releases](https://github.com/adolfousier/opencrabs/releases) — available for Linux (amd64/arm64), macOS (amd64/arm64), and Windows.

```bash
# Download, extract, run
tar xzf opencrabs-linux-amd64.tar.gz
./opencrabs
```

The onboarding wizard handles everything on first run.

> **Note:** `/rebuild` works even with pre-built binaries — it auto-clones the source to `~/.opencrabs/source/` on first use, then builds and hot-restarts. For active development or adding custom tools, Option 2 gives you the source tree directly.

### Option 2: Install via Cargo

```bash
cargo install opencrabs
```

> **Linux (Debian/Ubuntu):** Install system deps first: `sudo apt-get install build-essential pkg-config clang libclang-dev libasound2-dev libssl-dev cmake`
>
> **Large build:** The build can use 8GB+ in `/tmp`. If you run out of space: `CARGO_TARGET_DIR=~/.cargo/target cargo install opencrabs`

#### Feature Flags

All features are enabled by default. To customize, use `--no-default-features` and pick what you need:

```bash
# Install with only Telegram and Discord
cargo install opencrabs --no-default-features --features "telegram,discord"

# Everything except browser automation
cargo install opencrabs --no-default-features --features "telegram,whatsapp,discord,slack,trello,local-stt,local-tts"
```

| Feature | Crate | Description |
|---------|-------|-------------|
| `telegram` | teloxide | Telegram bot channel |
| `whatsapp` | whatsapp-rust | WhatsApp channel via multi-device API |
| `discord` | serenity | Discord bot channel |
| `slack` | slack-morphism | Slack bot channel (Socket Mode) |
| `trello` | — | Trello webhook channel |
| `local-stt` | rwhisper | On-device speech-to-text (requires CMake + C++ compiler) |
| `local-tts` | opusic-sys | On-device text-to-speech |
| `browser` | chromey | Browser automation via CDP (Chrome, Brave, Edge, Arc, Vivaldi, Opera — not Firefox) |

### Option 3: Build from Source (full control)

Required for `/rebuild`, adding custom tools, or modifying the agent.

**Prerequisites:**
- **Rust stable (1.91+)** — [Install Rust](https://rustup.rs/). The project includes a `rust-toolchain.toml` that selects the correct toolchain automatically
- **An API key** from at least one supported provider
- **SQLite** (bundled via sqlx)
- **macOS:** Xcode CLI Tools + `brew install cmake pkg-config` (requires macOS 15+)
- **Linux (Debian/Ubuntu):** `sudo apt-get install build-essential pkg-config clang libclang-dev libasound2-dev libssl-dev cmake`
- **Linux (Fedora/RHEL):** `sudo dnf install gcc gcc-c++ make pkg-config openssl-devel cmake`
- **Linux (Arch):** `sudo pacman -S base-devel pkg-config openssl cmake`

> **One-liner setup:** `bash <(curl -sL https://raw.githubusercontent.com/adolfousier/opencrabs/main/src/scripts/setup.sh)` — detects your platform, installs all dependencies, and sets up Rust.

```bash
# Clone
git clone https://github.com/adolfousier/opencrabs.git
cd opencrabs

# Build & run (development)
cargo run --bin opencrabs

# Or build release and run directly
cargo build --release
./target/release/opencrabs
```

> **Linux on older CPUs (Sandy Bridge / AVX1-only, no AVX2):** The local STT and embedding engine require at minimum AVX instructions. If your CPU has AVX but not AVX2 (e.g. Intel Sandy Bridge, Ivy Bridge — roughly 2011–2012), you must build with:
> ```bash
> RUSTFLAGS="-C target-cpu=native" cargo run --bin opencrabs
> # or for release:
> RUSTFLAGS="-C target-cpu=native" cargo build --release
> ```
> CPUs without AVX at all are not supported for local STT/embedding. API STT mode works on any machine.

> **API Keys:** OpenCrabs uses `keys.toml` instead of `.env` for API keys. The onboarding wizard will help you set it up, or edit `~/.opencrabs/keys.toml` directly. Keys are handled at runtime — no OS environment pollution.

> **First run?** The onboarding wizard will guide you through provider setup, workspace, and more. See [Onboarding Wizard](#-onboarding-wizard).

### Option 3: Docker (sandboxed)

Run OpenCrabs in an isolated container. Build takes ~15min (Rust release + LTO).

```bash
# Clone and run
git clone https://github.com/adolfousier/opencrabs.git
cd opencrabs

# Run with docker compose
# API keys are mounted from keys.toml on host
docker compose -f src/docker/compose.yml up --build
```

Config, workspace, and memory DB persist in a Docker volume across restarts. API keys in `keys.toml` are mounted into the container at runtime — never baked into the image.

### CLI Commands

```bash
# Interactive TUI (default)
cargo run --bin opencrabs
cargo run --bin opencrabs -- chat

# Onboarding wizard (first-time setup)
cargo run --bin opencrabs -- onboard
cargo run --bin opencrabs -- chat --onboard   # Force wizard before chat

# Non-interactive single command
cargo run --bin opencrabs -- run "What is Rust?"
cargo run --bin opencrabs -- run --format json "List 3 programming languages"
cargo run --bin opencrabs -- run --format markdown "Explain async/await"

# Configuration
cargo run --bin opencrabs -- init              # Initialize config
cargo run --bin opencrabs -- config            # Show current config
cargo run --bin opencrabs -- config --show-secrets

# Database
cargo run --bin opencrabs -- db init           # Initialize database
cargo run --bin opencrabs -- db stats          # Show statistics

# Profiles — multi-instance isolation
cargo run --bin opencrabs -- profile create hermes
cargo run --bin opencrabs -- profile list
cargo run --bin opencrabs -- -p hermes         # Run as "hermes" profile
cargo run --bin opencrabs -- profile migrate --from default --to hermes
cargo run --bin opencrabs -- profile export hermes -o backup.tar.gz
cargo run --bin opencrabs -- profile import backup.tar.gz
cargo run --bin opencrabs -- profile delete hermes

# Debug mode
cargo run --bin opencrabs -- -d                # Enable file logging
cargo run --bin opencrabs -- -d run "analyze this"

# Log management
cargo run --bin opencrabs -- logs status
cargo run --bin opencrabs -- logs view
cargo run --bin opencrabs -- logs view -l 100
cargo run --bin opencrabs -- logs clean
cargo run --bin opencrabs -- logs clean -d 3
```

> **Tip:** After `cargo build --release`, run the binary directly: `./target/release/opencrabs`

### Make It Available System-Wide

After downloading or building, add the binary to your PATH so you can run `opencrabs` from any project directory:

```bash
# Symlink (recommended — always points to latest build)
sudo ln -sf $(pwd)/target/release/opencrabs /usr/local/bin/opencrabs

# Or copy
sudo cp target/release/opencrabs /usr/local/bin/
```

Then from any project:
```bash
cd /your/project
opencrabs
```

Use `/cd` inside OpenCrabs to switch working directory at runtime without restarting.

**Output formats** for non-interactive mode: `text` (default), `json`, `markdown`

---

## 🧙 Onboarding Wizard

First-time users are guided through a 9-step setup wizard that appears automatically after the splash screen.

### How It Triggers

- **Automatic:** When no `~/.opencrabs/config.toml` exists and no API keys are set in `keys.toml`
- **CLI:** `cargo run --bin opencrabs -- onboard` (or `opencrabs onboard` after install)
- **Chat flag:** `cargo run --bin opencrabs -- chat --onboard` to force the wizard before chat
- **Slash command:** Type `/onboard` in the chat to re-run it anytime

### The 9 Steps

| Step | Title | What It Does |
|------|-------|-------------|
| 1 | **Mode Selection** | QuickStart (sensible defaults) vs Advanced (full control) |
| 2 | **Model & Auth** | Pick provider (Anthropic, OpenAI, GitHub Copilot, Gemini, OpenRouter, Minimax, z.ai GLM, Custom) → enter token/key or sign in via OAuth → model list fetched live from API → select model. Auto-detects existing keys from `keys.toml` |
| 3 | **Workspace** | Set brain workspace path (default `~/.opencrabs/`) → seed template files (SOUL.md, IDENTITY.md, etc.) |
| 4 | **Gateway** | Configure HTTP API gateway: port, bind address, auth mode |
| 5 | **Channels** | Toggle messaging integrations (Telegram, Discord, WhatsApp, Slack, Trello) |
| 6 | **Voice** | Choose STT mode: **Off** / **API** (Groq Whisper) / **Local** (whisper.cpp). Choose TTS mode: **Off** / **API** (OpenAI TTS) / **Local** (Piper TTS). Local modes show model/voice picker with download progress |
| 7 | **Image Handling** | Enable Gemini image generation and/or vision analysis — uses a separate Google AI key |
| 8 | **Daemon** | Install background service (systemd on Linux, LaunchAgent on macOS) |
| 9 | **Health Check** | Verify API key, config, workspace — shows pass/fail summary |
| 10 | **Brain Personalization** | Tell the agent about yourself and how you want it to behave → AI generates personalized brain files (SOUL.md, IDENTITY.md, USER.md, etc.) |

**QuickStart mode** skips steps 4-8 with sensible defaults. **Advanced mode** lets you configure everything.

Type `/onboard:voice` or `/onboard:image` in chat to jump directly to Voice or Image setup anytime.

#### Local STT (whisper.cpp)

Run speech-to-text on-device with zero API cost. Included by default in prebuilt binaries and `cargo install opencrabs`.

In `/onboard:voice`, select **Local** mode, pick a model size, and press Enter to download. Models are stored at `~/.local/share/opencrabs/models/whisper/`.

> **Building from source:** Local STT requires CMake and a C++ compiler (for whisper.cpp). To exclude it: `cargo install opencrabs --no-default-features --features telegram,whatsapp,discord,slack,trello`

| Model | Size | Quality |
|-------|------|---------|
| Tiny | ~75 MB | Fast, lower accuracy |
| Base | ~142 MB | Good balance |
| Small | ~466 MB | High accuracy |
| Medium | ~1.5 GB | Best accuracy |

Config (`config.toml`):
```toml
[voice]
stt_enabled = true
stt_mode = "local"              # "api" (default) or "local"
local_stt_model = "local-base"  # local-tiny, local-base, local-small, local-medium
tts_enabled = true
tts_mode = "local"              # "api" (default) or "local"
local_tts_voice = "ryan"        # ryan, amy, lessac, kristin, joe, cori
```

#### Brain Personalization (Step 10)

Two input fields: **About You** (who you are) and **Your OpenCrabs** (how the agent should behave). The LLM uses these plus the 6 workspace template files to generate personalized brain files.

- **First run:** Empty fields, static templates as reference → LLM generates → writes to workspace
- **Re-run:** Fields pre-populated with truncated preview of existing `USER.md` / `IDENTITY.md` → edit to regenerate or `Esc` to skip
- **Regeneration:** LLM receives the **current workspace files** (not static templates), so any manual edits you made are preserved as context
- **Overwrite:** Only files with new AI-generated content are overwritten; untouched files keep their current state
- No extra persistence files — the brain files themselves are the source of truth

### Wizard Navigation

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Navigate between fields |
| `Up` / `Down` | Scroll through lists |
| `Enter` | Confirm / next step |
| `Space` | Toggle checkboxes |
| `Esc` | Go back one step |

---

## 🔑 API Keys (keys.toml)

OpenCrabs uses `~/.opencrabs/keys.toml` as the **single source** for all API keys, bot tokens, and search keys. No `.env` files, no OS keyring, no environment variables for secrets. Keys are loaded at runtime and can be modified by the agent.

```toml
# ~/.opencrabs/keys.toml — chmod 600!

# LLM Providers
[providers.anthropic]
api_key = "sk-ant-api03-YOUR_KEY"

[providers.openai]
api_key = "sk-YOUR_KEY"

[providers.github]
api_key = "gho_..."                  # OAuth token (auto-saved by onboarding wizard)

[providers.openrouter]
api_key = "sk-or-YOUR_KEY"

[providers.minimax]
api_key = "your-minimax-key"

[providers.zhipu]
api_key = "your-zhipu-key"               # Get from open.bigmodel.cn

[providers.gemini]
api_key = "AIza..."                  # Get from aistudio.google.com

[providers.custom.your_name]
api_key = "your-key"                 # not required for local LLMs

# Image Generation & Vision (independent of main chat provider)
[image]
api_key = "AIza..."                  # Same Google AI key as providers.gemini (can reuse)

# Messaging Channels — tokens/secrets only (config.toml holds allowed_users, allowed_channels, etc.)
[channels.telegram]
token = "123456789:ABCdef..."

[channels.discord]
token = "your-discord-bot-token"

[channels.slack]
token = "xoxb-your-bot-token"
app_token = "xapp-your-app-token"   # Required for Socket Mode

[channels.trello]
app_token = "your-trello-api-key"   # API Key from trello.com/power-ups/admin
token = "your-trello-api-token"     # Token from the authorization URL

# Web Search
[providers.web_search.exa]
api_key = "your-exa-key"

[providers.web_search.brave]
api_key = "your-brave-key"

# Voice (STT/TTS)
# STT API mode (default): uses Groq Whisper
[providers.stt.groq]
api_key = "your-groq-key"

# STT Local mode: no API key needed — runs whisper.cpp on device
# Set stt_mode = "local" and local_stt_model in config.toml

# TTS API mode (default): uses OpenAI TTS
[providers.tts.openai]
api_key = "your-openai-key"

# TTS Local mode: no API key needed — runs Piper TTS on device
# Set tts_mode = "local" and local_tts_voice in config.toml
```

> **Note:** Anthropic OAuth tokens (`sk-ant-oat`) are no longer supported for third-party apps as of Feb 2026 ([anthropics/claude-code#28091](https://github.com/anthropics/claude-code/issues/28091)). Use console API keys (`sk-ant-api03-*`) instead.

> **Trello note:** `app_token` holds the Trello **API Key** and `token` holds the Trello **API Token** — `app_token` is the app-level credential and `token` is the user-level credential. Board IDs are configured via `board_ids` in `config.toml`.

> **Security:** Always `chmod 600 ~/.opencrabs/keys.toml` and add `keys.toml` to `.gitignore`.

---

## 🔐 Secret Sanitization & Redaction

OpenCrabs automatically redacts API keys and tokens from all outputs — conversation history, TUI display, tool approval dialogs, and external channel delivery (Telegram, Discord, Slack, WhatsApp). Secrets never persist to the database or appear in logs.

### How It Works

**Three layers of defense:**

1. **Prefix-based detection** — Keys with recognized prefixes are caught instantly (`sk-proj-...` → `sk-proj-[REDACTED]`)
2. **Hex token detection** — Contiguous hex strings of 32+ chars are redacted
3. **Mixed alphanumeric detection** — Opaque tokens of 28+ chars containing both letters and digits are caught as a safety net

**Structural redaction** also applies to:
- JSON fields named `authorization`, `api_key`, `token`, `secret`, `password`, etc.
- Inline patterns in bash commands (`bearer ...`, `x-api-key: ...`, `api_key=...`)
- URL passwords (`https://user:PASSWORD@host` → `https://user:[REDACTED]@host`)

### Supported Key Prefixes

OpenCrabs recognizes **60+ industry-standard key formats** out of the box:

| Category | Prefixes |
|----------|----------|
| **AI / LLM** | `sk-proj-` (OpenAI), `sk-ant-` (Anthropic), `sk-or-v1-` (OpenRouter), `sk-` (generic), `gsk_` (Groq), `nvapi-` (NVIDIA), `AIzaSy` (Google), `pplx-` (Perplexity), `hf_` (HuggingFace), `r8_` (Replicate) |
| **Cloud** | `AKIA` / `ASIA` (AWS), `DefaultEndpointsProtocol=` (Azure), `ya29.` (Google OAuth) |
| **Payments** | `sk_live_` / `sk_test_` / `pk_live_` / `pk_test_` / `rk_live_` / `rk_test_` (Stripe), `sq0atp-` / `sq0csp-` (Square) |
| **Git / DevOps** | `ghp_` / `gho_` / `ghu_` / `ghs_` / `github_pat_` (GitHub), `glpat-` / `gloas-` (GitLab), `npm_` (npm), `pypi-AgEIcHlwaS` (PyPI) |
| **Communication** | `xoxb-` / `xoxp-` / `xapp-` / `xoxs-` (Slack), `SG.` (SendGrid), `xkeysib-` (Brevo) |
| **SaaS** | `shpat_` / `shpca_` / `shppa_` / `shpss_` (Shopify), `ntn_` (Notion), `lin_api_` (Linear), `aio_` (Airtable), `phc_` (PostHog) |
| **Infrastructure** | `sntrys_` (Sentry), `dop_v1_` (DigitalOcean), `tskey-` (Tailscale), `tvly-` (Tavily), `hvs.` / `vault:v1:` (HashiCorp Vault) |
| **Auth / Crypto** | `eyJ` (JWT), `whsec_` (webhooks), `EAA` (Facebook/Meta), `ATTA` (Trello), `AGE-SECRET-KEY-` (age encryption) |

### Best Practice: Use Prefixed Keys

If you build or configure services that integrate with OpenCrabs, **use industry-standard key prefixes**. Keys with recognized prefixes are caught by the first and most reliable layer of redaction. Opaque tokens (random alphanumeric strings with no prefix) rely on the generic safety-net regex, which has a higher threshold to avoid false positives.

**Good:** `av_38947394723jkhkrjkhdfiuo83489732` — prefix makes it instantly recognizable
**Risky:** `38947394723jkhkrjkhdfiuo83489732` — still caught by mixed-alnum regex, but no prefix context in `[REDACTED]` output

### Memory Safety

API keys stored via `SecretString` are:
- **Zeroized on drop** — memory is overwritten when the value goes out of scope (not left for GC)
- **Never serialized** — `Debug`, `Display`, and `Serialize` all output `[REDACTED]`
- **Never logged** — `expose_secret()` is the only way to access the raw value

---

## 🏠 Using Local LLMs

OpenCrabs works with any OpenAI-compatible local inference server for **100% private, zero-cost** operation.

### LM Studio (Recommended)

1. Download and install [LM Studio](https://lmstudio.ai/)
2. Download a model (e.g., `qwen2.5-coder-7b-instruct`, `Mistral-7B-Instruct`, `Llama-3-8B`)
3. Start the local server (default port 1234)
4. Add to `config.toml` — no API key needed:

```toml
[providers.custom.lm_studio]
enabled = true
base_url = "http://localhost:1234/v1"
default_model = "qwen2.5-coder-7b-instruct"   # Must EXACTLY match LM Studio model name
models = ["qwen2.5-coder-7b-instruct", "llama-3-8B", "mistral-7B-instruct"]
```

> **Critical:** The `default_model` value must exactly match the model name shown in LM Studio's Local Server tab (case-sensitive).

### Ollama

```bash
ollama pull mistral
```

Add to `config.toml` — no API key needed:
```toml
[providers.custom.ollama]
enabled = true
base_url = "http://localhost:11434/v1"
default_model = "mistral"
models = ["mistral", "llama3", "codellama"]
```

### Multiple Local Providers

Want both LM Studio and Ollama configured? Use named providers and switch via `/models`:

```toml
[providers.custom.lm_studio]
enabled = true
base_url = "http://localhost:1234/v1"
default_model = "qwen2.5-coder-7b-instruct"
models = ["qwen2.5-coder-7b-instruct", "llama-3-8B", "mistral-7B-instruct"]

[providers.custom.ollama]
enabled = false
base_url = "http://localhost:11434/v1"
default_model = "mistral"
models = ["mistral", "llama3", "codellama"]
```

The name after `custom.` is just a label you choose. The first one with `enabled = true` is used. Switch anytime via `/models` or `/onboard`.

### Recommended Models

| Model | RAM | Best For |
|-------|-----|----------|
| Qwen-2.5-7B-Instruct | 16 GB | Coding tasks |
| Mistral-7B-Instruct | 16 GB | General purpose, fast |
| Llama-3-8B-Instruct | 16 GB | Balanced performance |
| DeepSeek-Coder-6.7B | 16 GB | Code-focused |
| TinyLlama-1.1B | 4 GB | Quick responses, lightweight |

**Tips:**
- Start with Q4_K_M quantization for best speed/quality balance
- Set context length to 8192+ in LM Studio settings
- Use `Ctrl+N` to start a new session if you hit context limits
- GPU acceleration significantly improves inference speed

### Cloud vs Local Comparison

| Aspect | Cloud (Anthropic) | Local (LM Studio) |
|--------|-------------------|-------------------|
| Privacy | Data sent to API | 100% private |
| Cost | Per-token pricing | Free after download |
| Speed | 1-2s (network) | 2-10s (hardware-dependent) |
| Quality | Excellent (Claude 4.x) | Good (model-dependent) |
| Offline | Requires internet | Works offline |

See [LM_STUDIO_GUIDE.md](src/docs/guides/LM_STUDIO_GUIDE.md) for detailed setup and troubleshooting.

---

## 📝 Configuration

### Configuration Files

OpenCrabs uses three config files — all **hot-reloaded at runtime** (no restart needed):

| File | Purpose | Secret? |
|------|---------|---------|
| `~/.opencrabs/config.toml` | Provider settings, models, channels, allowed users | No — safe to commit |
| `~/.opencrabs/keys.toml` | API keys, bot tokens | **Yes** — `chmod 600`, never commit |
| `~/.opencrabs/commands.toml` | User-defined slash commands | No |
| `~/.opencrabs/tools.toml` | Runtime-defined agent tools (HTTP, shell) | No |

Changes to any of these files are picked up automatically within ~300ms while OpenCrabs is running. The active LLM provider, channel allowlists, approval policy, and slash command autocomplete all update without restart.

### Profiles (Multi-Instance)

Run multiple isolated OpenCrabs instances from the same installation. Each profile gets its own config, memory, sessions, skills, and gateway service.

```bash
# Create a new profile
opencrabs profile create hermes

# Run as a specific profile
opencrabs -p hermes

# Or set via environment variable
OPENCRABS_PROFILE=hermes opencrabs daemon

# List all profiles
opencrabs profile list

# Migrate config and brain files from one profile to another
# Copies: *.md, *.toml (config, keys, tools, commands), memory/ directory
# Skips: database, sessions, logs, locks
opencrabs profile migrate --from default --to hermes
opencrabs profile migrate --from default --to hermes --force  # overwrite existing files

# Export/import for sharing or backup
opencrabs profile export hermes -o hermes-backup.tar.gz
opencrabs profile import hermes-backup.tar.gz

# Delete a profile
opencrabs profile delete hermes
```

**Directory layout:**

```
~/.opencrabs/                    # "default" profile (backward compatible)
├── config.toml
├── keys.toml
├── opencrabs.db
├── memory/
├── profiles.toml                # profile registry
├── locks/                       # token-lock files
└── profiles/
    └── hermes/                  # named profile — fully isolated
        ├── config.toml
        ├── keys.toml
        ├── opencrabs.db
        ├── memory/
        └── *.md
```

**Token-lock isolation:** Two profiles cannot use the same bot token (Telegram, Discord, Slack, Trello). On startup, OpenCrabs acquires PID-based locks for each channel credential. If another profile already holds that token, the channel refuses to start and notifies the user. Stale locks from crashed processes are automatically cleaned up.

**Migration workflow:** Use `profile migrate` to clone your configuration into a new profile, then customize the brain files (SOUL.md, IDENTITY.md) to give the new instance its own personality. The original profile's database and sessions stay untouched.

Search order for `config.toml`:
1. `~/.opencrabs/config.toml` (primary)
2. `~/.config/opencrabs/config.toml` (legacy fallback)
3. `./opencrabs.toml` (current directory override)

---

## 🛠️ Configuration (config.toml)

Full annotated example — the onboarding wizard writes this for you, but you can edit it directly:

```toml
# ~/.opencrabs/config.toml

[agent]
approval_policy = "auto-always"  # auto-always (default) | auto-session | ask
working_directory = "~/projects" # default working dir for Bash/file tools

# ── Channels ──────────────────────────────────────────────────────────────────

[channels.telegram]
enabled = true
allowed_users = ["123456789"]    # Telegram user IDs (get yours via /start)
respond_to = "all"               # all | mention | dm_only

[channels.discord]
enabled = true
allowed_users = ["637291214508654633"]  # Discord user IDs
allowed_channels = ["1473207147025137778"]
respond_to = "mention"           # all | mention | dm_only

[channels.slack]
enabled = true
allowed_users = ["U066SGWQZFG"]  # Slack user IDs
allowed_channels = ["C0AEY3C2P9V"]
respond_to = "mention"           # all | mention | dm_only

[channels.whatsapp]
enabled = true
allowed_phones = ["+1234567890"] # E.164 format

[channels.trello]
enabled = true
board_ids = ["your-board-id"]    # From the board URL

# ── Providers ─────────────────────────────────────────────────────────────────

[providers.anthropic]
enabled = true
default_model = "claude-sonnet-4-6"

[providers.gemini]
enabled = false

[providers.openai]
enabled = false
default_model = "gpt-5-nano"

[providers.zhipu]
enabled = false
default_model = "glm-4.7"
endpoint_type = "coding"                 # "api" (General) or "coding" (Coding API)

# ── Image ─────────────────────────────────────────────────────────────────────

[image.generation]
enabled = true
model = "gemini-3.1-flash-image-preview"

[image.vision]
enabled = true
model = "gemini-3.1-flash-image-preview"

# ── Cron Defaults ────────────────────────────────────────────────────────────

[cron]
default_provider = "minimax"      # Provider for cron jobs that don't specify one
default_model = "MiniMax-M2.7"    # Model for cron jobs that don't specify one
```

> API keys go in `keys.toml`, not here. See [API Keys (keys.toml)](#-api-keys-keystoml).

---

## 📋 Commands (commands.toml)

User-defined slash commands — the agent writes these autonomously via the `config_manager` tool, or you can edit directly:

```toml
# ~/.opencrabs/commands.toml

[[commands]]
name = "/deploy"
description = "Deploy to staging server"
action = "prompt"
prompt = "Run ./deploy.sh staging and report the result."

[[commands]]
name = "/standup"
description = "Generate a daily standup summary"
action = "prompt"
prompt = "Summarize my recent git commits and open tasks for a standup. Be concise."

[[commands]]
name = "/rebuild"
description = "Build and restart OpenCrabs from source"
action = "prompt"
prompt = 'Run `RUSTFLAGS="-C target-cpu=native" cargo build --release` in /srv/rs/opencrabs. If it succeeds, ask if I want to restart now.'
```

Commands appear instantly in autocomplete (type `/`) after saving — no restart needed. The `action` field supports:
- `"prompt"` — sends the prompt text to the agent for execution
- `"system"` — displays the text inline as a system message

---

## 🔌 Dynamic Tools (tools.toml)

Runtime-defined tools that the agent can call autonomously — no rebuild, no restart. Unlike slash commands (`commands.toml`) which are user-triggered shortcuts, dynamic tools appear in the LLM's tool list and the agent decides when to use them.

```toml
# ~/.opencrabs/tools.toml

[[tools]]
name = "health_check"
description = "Check if a service is healthy"
executor = "http"
method = "GET"
url = "https://{{host}}/health"
headers = { "Authorization" = "Bearer {{token}}" }
timeout_secs = 10
requires_approval = false

[[tools.params]]
name = "host"
type = "string"
description = "Hostname to check"
required = true

[[tools.params]]
name = "token"
type = "string"
description = "API bearer token"
required = true

[[tools]]
name = "deploy_staging"
description = "Deploy the current branch to staging"
executor = "shell"
command = "cd ~/project && ./deploy.sh {{branch}} staging"
timeout_secs = 120
requires_approval = true

[[tools.params]]
name = "branch"
type = "string"
description = "Git branch to deploy"
required = true
```

**Executor types:**

| Executor | Fields | Description |
|----------|--------|-------------|
| `http` | `method`, `url`, `headers` | Makes an HTTP request. Template variables (`{{param}}`) are substituted in the URL, headers, and body |
| `shell` | `command` | Runs a shell command. Template variables substituted in the command string |

**Fields:**

| Field | Required | Default | Description |
|-------|----------|---------|-------------|
| `name` | Yes | | Tool name (used by the agent to call it) |
| `description` | Yes | | What the tool does (shown to the LLM) |
| `executor` | Yes | | `http` or `shell` |
| `enabled` | No | `true` | Whether the tool is active |
| `requires_approval` | No | `true` | Whether the user must approve each call |
| `timeout_secs` | No | `30` | Execution timeout in seconds |
| `params` | No | `[]` | Parameter definitions with name, type, description, required |

**Management via agent:** The `tool_manage` meta-tool lets the agent create, remove, enable/disable, and reload tools at runtime. Tell it *"add a tool that checks my server health"* and it writes the definition to `tools.toml` automatically.

**Commands vs Tools:**

| | Commands (`commands.toml`) | Tools (`tools.toml`) |
|---|---|---|
| **Triggered by** | User types `/command` | Agent decides autonomously |
| **Appears in** | Autocomplete menu | LLM tool list |
| **Use case** | Shortcuts, workflows | Integrations, automations |
| **Action** | Sends a prompt to the agent | Executes HTTP request or shell command directly |

See [`tools.toml.example`](tools.toml.example) for a complete reference.

---

### Example: Hybrid Setup (Local + Cloud)

Keep multiple providers configured — enable the one you want to use, disable the rest.
Switch anytime by toggling `enabled` or using `/onboard`.

In `config.toml`:
```toml
# Local LLM — currently active
[providers.custom.lm_studio]
enabled = true
base_url = "http://localhost:1234/v1"
default_model = "qwen2.5-coder-7b-instruct"
models = ["qwen2.5-coder-7b-instruct", "llama-3-8B"]

# Cloud API — disabled, enable when you need it
[providers.anthropic]
enabled = false
default_model = "claude-opus-4-6"
```

In `keys.toml`:
```toml
[providers.anthropic]
api_key = "sk-ant-api03-YOUR_KEY"
```

### Operational Environment Variables

All API keys and secrets are stored in `keys.toml` — **not** in environment variables. The only env vars OpenCrabs uses are operational:

| Variable | Description |
|----------|-------------|
| `DEBUG_LOGS_LOCATION` | Custom log directory path (default: `.opencrabs/logs/`) |
| `OPENCRABS_BRAIN_PATH` | Custom brain workspace path (default: `~/.opencrabs/`) |

---

## 💰 Pricing Customization (usage_pricing.toml)

OpenCrabs tracks real token costs per model using a centralized pricing table at `~/.opencrabs/usage_pricing.toml`. It's written automatically on first run with sensible defaults.

**Why it matters:**
- `/usage` shows real costs grouped by model across all sessions
- Old sessions with stored tokens but zero cost get estimated costs (shown as `~$X.XX` in yellow)
- Unknown models show `$0.00` instead of silently ignoring them

**Customizing prices:**

```toml
# ~/.opencrabs/usage_pricing.toml
# Edit live — changes take effect on next /usage open, no restart needed.

[providers.anthropic]
entries = [
  { prefix = "claude-sonnet-4",  input_per_m = 3.0,  output_per_m = 15.0 },
  { prefix = "claude-opus-4",    input_per_m = 5.0,  output_per_m = 25.0 },
  { prefix = "claude-haiku-4",   input_per_m = 1.0,  output_per_m = 5.0  },
]

[providers.minimax]
entries = [
  { prefix = "minimax-m2.5",     input_per_m = 0.30, output_per_m = 1.20 },
]

# Add any provider — prefix is matched case-insensitively as a substring
[providers.my_custom_model]
entries = [
  { prefix = "my-model-v1",      input_per_m = 1.00, output_per_m = 3.00 },
]
```

A full example with all built-in providers (Anthropic, OpenAI, MiniMax, Google, DeepSeek, Meta) is available at [`usage_pricing.toml.example`](./usage_pricing.toml.example) in the repo root.

---

## 🔧 Tool System

OpenCrabs includes 30+ built-in tools. The AI can use these during conversation:

#### File & Code
| Tool | Description |
|------|-------------|
| `read_file` | Read file contents with syntax awareness |
| `write_file` | Create or modify files |
| `edit_file` | Precise text replacements in files |
| `bash` | Execute shell commands — **any CLI tool on your system works** |
| `ls` | List directory contents |
| `glob` | Find files matching patterns |
| `grep` | Search file contents with regex |
| `execute_code` | Run code in various languages |
| `notebook_edit` | Edit Jupyter notebooks |
| `parse_document` | Extract text from PDF, DOCX, HTML |

#### Search & Web
| Tool | Description |
|------|-------------|
| `web_search` | Search the web (DuckDuckGo, always available, no key needed) |
| `exa_search` | Neural web search via EXA AI (free via MCP, no API key needed; set key in `keys.toml` for higher rate limits) |
| `brave_search` | Web search via Brave Search (set key in `keys.toml` — free $5/mo credits at brave.com/search/api) |
| `http_request` | Make HTTP requests |
| `memory_search` | Hybrid semantic search across past memory logs — FTS5 keyword + vector embeddings (768-dim, local GGUF model) combined via RRF. No API key needed, runs offline |

#### Image
| Tool | Description |
|------|-------------|
| `generate_image` | Generate images via Google Gemini — auto-sent as native images on all channels |
| `analyze_image` | Analyze images (local files or URLs) via vision model — uses Gemini vision or provider's `vision_model` |

#### Channel Tools
| Tool | Description |
|------|-------------|
| `telegram_send` | 19 actions: send, reply, edit, delete, pin, forward, send_photo, send_document, polls, buttons, admin ops |
| `discord_send` | 17 actions: send, reply, react, edit, delete, pin, threads, embeds, roles, kick, ban, send_file |
| `slack_send` | 17 actions: send, reply, react, edit, delete, pin, blocks, topics, members, send_file |
| `trello_send` | 22 actions: cards, comments, checklists, labels, members, attachments, board management, search |
| `channel_search` | Search captured message history across all channels (Telegram, Discord, Slack, WhatsApp) |

#### Agent & System
| Tool | Description |
|------|-------------|
| `task_manager` | Manage agent tasks |
| `plan` | Create structured execution plans |
| `config_manager` | Read/write config.toml and commands.toml at runtime (change settings, add/remove commands, reload config) |
| `session_context` | Access session information |
| `cron_manage` | Schedule recurring jobs — create, list, enable/disable, delete. Deliver results to any channel |
| `a2a_send` | Send tasks to remote A2A-compatible agents via JSON-RPC 2.0 |
| `tool_manage` | Manage runtime tools — list, add, remove, enable, disable, reload (`tools.toml`) |
| `evolve` | Download latest release binary from GitHub and hot-restart (no Rust toolchain needed) |
| `rebuild` | Build from source (`cargo build --release`) and hot-restart |

#### Browser Automation (feature: `browser`)

Auto-detects your default Chromium-based browser and uses its native profile (cookies, logins, extensions). Supports **Chrome, Brave, Edge, Arc, Vivaldi, Opera, and Chromium**. If your default browser is Firefox or another non-Chromium browser, OpenCrabs falls back to the first available Chromium browser on your system. If no Chromium browser is found, a fresh Chrome instance is launched.

| Tool | Description |
|------|-------------|
| `browser_navigate` | Navigate to a URL, returns page title and final URL after redirects |
| `browser_click` | Click an element by CSS selector |
| `browser_type` | Type text into an element (by CSS selector or focused element) |
| `browser_screenshot` | Capture page screenshot (full page or element), returns base64 PNG |
| `browser_eval` | Execute JavaScript in page context and return the result |
| `browser_content` | Get page HTML or text-only content, optionally scoped by CSS selector |
| `browser_wait` | Wait for a CSS selector to appear (polls every 200ms, default 10s timeout) |

> **Why no Firefox?** Browser automation uses Chrome DevTools Protocol (CDP). Firefox dropped CDP support entirely — it now uses WebDriver BiDi, which is a different protocol. All Chromium-based browsers speak CDP natively.

#### Multi-Agent Orchestration

OpenCrabs supports spawning specialized sub-agents that run autonomously in isolated sessions. Each sub-agent gets its own context, tool registry, and cancel token.

| Tool | Description |
|------|-------------|
| `spawn_agent` | Spawn a typed child agent to handle a sub-task autonomously in the background |
| `wait_agent` | Wait for a spawned sub-agent to complete and return its output (configurable timeout) |
| `send_input` | Send follow-up input/instructions to a running sub-agent |
| `close_agent` | Terminate a running sub-agent and clean up resources |
| `resume_agent` | Resume a completed or failed sub-agent with a new prompt (preserves prior context) |

**Agent Types** — when spawning, an `agent_type` parameter selects a specialized role:

| Type | Role | Tools |
|------|------|-------|
| `general` | Full-capability agent (default) | All parent tools minus recursive/dangerous |
| `explore` | Fast codebase navigation — read-only | `read_file`, `glob`, `grep`, `ls` |
| `plan` | Architecture planning — read + analysis | `read_file`, `glob`, `grep`, `ls`, `bash` |
| `code` | Implementation — full write access | All parent tools minus recursive/dangerous |
| `research` | Web search + documentation lookup | `read_file`, `glob`, `grep`, `ls`, `web_search`, `http_client` |

Sub-agents never have access to recursive tools (`spawn_agent`, `resume_agent`, `wait_agent`, `send_input`, `close_agent`) or dangerous system tools (`rebuild`, `evolve`).

**Subagent Configuration** — optionally override the provider/model for all spawned sub-agents in `config.toml`:

```toml
[agent]
subagent_provider = "openrouter"   # provider for child agents (omit to inherit parent)
subagent_model = "qwen/qwen3-235b" # model override for child agents (omit to inherit parent)
```

This lets you run a powerful model for the main session while using a cheaper/faster model for sub-tasks.

### System CLI Tools

OpenCrabs can leverage **any CLI tool installed on your system** via `bash`. Common integrations:

| Tool | Purpose | Example |
|------|---------|---------|
| `gh` | GitHub CLI — issues, PRs, repos, releases, actions | `gh issue list`, `gh pr create` |
| `gog` | Google CLI — Gmail, Calendar (OAuth) | `gog gmail search "is:unread"`, `gog calendar events` |
| `docker` | Container management | `docker ps`, `docker compose up` |
| `ssh` | Remote server access | `ssh user@host "command"` |
| `node` | Run JavaScript/TypeScript tools | `node script.js` |
| `python3` | Run Python scripts and tools | `python3 analyze.py` |
| `ffmpeg` | Audio/video processing | `ffmpeg -i input.mp4 output.gif` |
| `curl` | HTTP requests (fallback when `http_request` insufficient) | `curl -s api.example.com` |

Any tool on your `$PATH` works. If it runs in your terminal, OpenCrabs can use it.

---


## ⌨️ Keyboard Shortcuts

### Global

| Shortcut | Action |
|----------|--------|
| `Ctrl+C` | First press clears input, second press (within 3s) quits |
| `Ctrl+N` | New session |
| `Ctrl+L` | List/switch sessions |
| `Ctrl+K` | Clear current session |
| `Page Up/Down` | Scroll chat history |
| `Mouse Scroll` | Scroll chat history |
| `Escape` | Clear input / close overlay |

### Chat Mode

| Shortcut | Action |
|----------|--------|
| `Enter` | Send message |
| `Ctrl+J` | New line (vim — also Alt+Enter / Shift+Enter on supported terminals) |
| `←` / `→` | Move cursor one character |
| `↑` / `↓` | Navigate lines (multiline), jump to start/end (single-line), then history |
| `Ctrl+←` / `Ctrl+→` | Jump by word |
| `Home` / `End` | Start / end of current line |
| `Delete` | Delete character after cursor |
| `Ctrl+W` | Delete word before cursor (vim) |
| `Ctrl+U` | Delete to start of line (vim) |
| `Left-click` | Select/highlight a message |
| `Right-click` | Copy message to clipboard |
| `Escape` ×2 | Abort in-progress request |
| `/help` | Open help dialog |
| `/model` | Show current model |
| `/models` | Switch model (fetches live from provider API) |
| `/usage` | Token/cost stats — shows current session + all-time breakdown grouped by model with estimated costs for historical sessions |
| `/onboard` | Run setup wizard (full flow) |
| `/onboard:provider` | Jump to provider/API key setup |
| `/onboard:workspace` | Jump to workspace settings |
| `/onboard:channels` | Jump to channel config |
| `/onboard:voice` | Jump to voice STT/TTS setup |
| `/onboard:image` | Jump to image handling setup |
| `/onboard:gateway` | Jump to API gateway settings |
| `/onboard:brain` | Jump to brain/persona setup |
| `/doctor` | Run connection health check |
| `/sessions` | Open session manager |
| `/approve` | Tool approval policy selector (approve-only / session / yolo) |
| `/compact` | Compact context (summarize + trim for long sessions) |
| `/rebuild` | Build from source & hot-restart — streams live compiler output to chat, auto exec() restarts on success (no prompt), auto-clones repo if no source tree found |
| `/whisper` | Voice-to-text — speak anywhere, pastes to clipboard |
| `/cd` | Change working directory (directory picker) |
| `/settings` or `S` | Open Settings screen (provider, approval, commands, paths) |
| `/stop` | Abort in-progress agent operation (channels only — TUI uses `Escape` x2) |

### Channel Commands (Telegram, Discord, Slack, WhatsApp)

When connected via messaging channels, the following slash commands are available directly in chat. These are the channel equivalents of TUI commands — type them as regular messages.

| Command | Action |
|---------|--------|
| `/help` | List available channel commands |
| `/usage` | Session token & cost stats (current session + all-time breakdown by model) |
| `/models` | Switch AI model — shows platform-native buttons (Telegram inline keyboard, Discord buttons, Slack Block Kit). WhatsApp shows a plain text list |
| `/stop` | Abort the current agent operation immediately — cancels streaming, tool execution, and any pending approvals. Equivalent to double-Escape in the TUI |

Model switching via `/models` changes the model within the current provider and takes effect immediately (no restart needed). The selection persists to `config.toml`.

Any message that isn't a recognized command is forwarded to the AI agent as normal.

### Sessions Mode

Each session shows its provider/model badge (e.g. `[anthropic/claude-sonnet-4-6]`) and token count. Sessions processing in the background show a spinner; sessions with unread responses show a green dot.

| Shortcut | Action |
|----------|--------|
| `↑` / `↓` | Navigate sessions |
| `Enter` | Load selected session (auto-restores its provider + model) |
| `R` | Rename session |
| `D` | Delete session |
| `Esc` | Back to chat |

### Tool Approval (Inline)

When the AI requests a tool that needs permission, an inline approval prompt appears in chat. Approvals are session-aware: background sessions auto-approve tool calls so they don't block, and switching sessions never loses a pending approval.

| Shortcut | Action |
|----------|--------|
| `↑` / `↓` | Navigate approval options |
| `Enter` | Confirm selected option |
| `D` / `Esc` | Deny the tool request |
| `V` | Toggle parameter details |

**Approval options (TUI and all channels):**

| Option | Effect |
|--------|--------|
| **Yes** | Approve this single tool call |
| **Always (session)** | Auto-approve all tools for this session (resets on restart) |
| **YOLO (permanent)** | Auto-approve all tools permanently, persists to `config.toml` |
| **No** | Deny this tool call |

Use `/approve` to change your approval policy at any time (persisted to `config.toml`):

| Policy | Description |
|--------|-------------|
| **Approve-only** | Prompt before every tool execution. Use this if you want to review each action the agent takes. Set with `/approve` → "Approve-only (always ask)" |
| **Allow all (session)** | Auto-approve all tools for the current session only, resets on restart |
| **Yolo mode** | Execute everything without approval (default for new users). Set with `/approve` → "Yolo mode" |

> **Note:** New installations default to Yolo mode so the agent can work autonomously out of the box. If you prefer to review each tool call, run `/approve` and select **Approve-only (always ask)**.

---

## 🔍 Debug and Logging

OpenCrabs uses a **conditional logging system** — no log files by default.

```bash
# Stack traces on crash
RUST_BACKTRACE=1 opencrabs

# Verbose console output (no file logging)
RUST_LOG=debug opencrabs

# Debug mode: rolling file logs to ~/.opencrabs/logs/, auto-cleanup after 7 days
opencrabs -d

# Both: file logs + verbose console output
RUST_LOG=debug opencrabs -d
```

All four work the same whether installed via `cargo install opencrabs` or built from source.

```bash
# Log management
opencrabs logs status    # Check logging status
opencrabs logs view      # View recent entries
opencrabs logs clean     # Clean old logs
opencrabs logs clean -d 3  # Clean logs older than 3 days
```

**When debug mode (`-d`) is enabled:**
- Log files created in `~/.opencrabs/logs/`
- DEBUG level with thread IDs, file names, line numbers
- Daily rolling rotation, noisy crates (hyper, h2, reqwest) suppressed to warn

**When disabled (default):**
- No log files created
- Only warnings and errors to stderr
- Clean workspace

---

## 🧠 Brain System & 3-Tier Memory

OpenCrabs's brain is **dynamic and self-sustaining**. Instead of a hardcoded system prompt, the agent assembles its personality, knowledge, and behavior from workspace files that can be edited between turns.

### Brain Workspace

The brain reads markdown files from `~/.opencrabs/`:

```
~/.opencrabs/                  # Home — everything lives here
├── SOUL.md                    # Personality, tone, hard behavioral rules
├── IDENTITY.md                # Agent name, vibe, style, workspace path
├── USER.md                    # Who the human is, how to work with them
├── AGENTS.md                  # Workspace rules, memory system, safety policies
├── TOOLS.md                   # Environment-specific notes (SSH hosts, API accounts)
├── MEMORY.md                  # Long-term curated context (never touched by auto-compaction)
├── SECURITY.md                # Security policies and access controls
├── BOOT.md                    # Startup checklist (optional, runs on launch)
├── HEARTBEAT.md               # Periodic task definitions (optional)
├── BOOTSTRAP.md               # First-run onboarding wizard (deleted after setup)
├── config.toml                # App configuration (provider, model, approval policy)
├── keys.toml                  # API keys (provider, channel, STT/TTS)
├── commands.toml              # User-defined slash commands
├── tools.toml                 # Runtime-defined agent tools (HTTP, shell)
├── opencrabs.db               # SQLite — sessions, messages, plans
└── memory/                    # Daily memory logs (auto-compaction summaries)
    └── YYYY-MM-DD.md          # One per day, multiple compactions stack
```

Brain files are re-read **every turn** — edit them between messages and the agent immediately reflects the changes. Missing files are silently skipped; a hardcoded brain preamble is always present.

### 3-Tier Memory Architecture

| Tier | Location | Purpose | Managed By |
|------|----------|---------|------------|
| **1. Brain MEMORY.md** | `~/.opencrabs/MEMORY.md` | Durable, curated knowledge loaded into system brain every turn | You (the user) |
| **2. Daily Memory Logs** | `~/.opencrabs/memory/YYYY-MM-DD.md` | Auto-compaction summaries with structured breakdowns of each session | Auto (on compaction) |
| **3. Hybrid Memory Search** | `memory_search` tool (FTS5 + vector) | Hybrid semantic search — BM25 keyword + vector embeddings (768-dim, local GGUF) combined via Reciprocal Rank Fusion. No API key, zero cost, runs offline | Agent (via tool call) |

**How it works:**
1. When context hits 70%, auto-compaction summarizes the conversation into a structured breakdown (current task, decisions, files modified, errors, next steps)
2. The summary is saved to a daily log at `~/.opencrabs/memory/2026-02-15.md` (multiple compactions per day stack in the same file)
3. The summary is shown to you in chat so you see exactly what was remembered
4. The file is indexed in the background into the FTS5 database so the agent can search past logs with `memory_search`
5. Brain `MEMORY.md` is **never touched** by auto-compaction — it stays as your curated, always-loaded context

#### Hybrid Memory Search (FTS5 + Vector Embeddings)

Memory search combines two strategies via **Reciprocal Rank Fusion (RRF)** for best-of-both-worlds recall:

1. **FTS5 keyword search** — BM25-ranked full-text matching with porter stemming
2. **Vector semantic search** — 768-dimensional embeddings via a local GGUF model (embeddinggemma-300M, ~300 MB)

The embedding model downloads automatically on first TUI launch (~300 MB, one-time) and runs entirely on CPU. **No API key, no cloud service, no per-query cost, works offline.** If the model isn't available yet (first launch, still downloading), search gracefully falls back to FTS-only.

```
┌─────────────────────────────────────┐
│  ~/.opencrabs/memory/               │
│  ├── 2026-02-15.md                  │  Markdown files (daily logs)
│  ├── 2026-02-16.md                  │
│  └── 2026-02-17.md                  │
└──────────────┬──────────────────────┘
               │ index on startup +
               │ after each compaction
               ▼
┌─────────────────────────────────────────────────┐
│  memory.db  (SQLite WAL mode)                   │
│  ┌───────────────────────┐ ┌──────────────────┐ │
│  │ documents + FTS5      │ │ vector embeddings│ │
│  │ (BM25, porter stem)   │ │ (768-dim, cosine)│ │
│  └───────────┬───────────┘ └────────┬─────────┘ │
└──────────────┼──────────────────────┼───────────┘
               │ MATCH query          │ cosine similarity
               ▼                      ▼
┌─────────────────────────────────────────────────┐
│  Reciprocal Rank Fusion (k=60)                  │
│  Merges keyword + semantic results              │
└─────────────────────┬───────────────────────────┘
                      ▼
┌─────────────────────────────────────────────────┐
│  Hybrid-ranked results with snippets            │
└─────────────────────────────────────────────────┘
```

**Why local embeddings instead of OpenAI/cloud?**

| | Local (embeddinggemma-300M) | Cloud API (e.g. OpenAI) |
|---|---|---|
| **Cost** | Free forever | ~$0.0001/query, adds up |
| **Privacy** | 100% local, nothing leaves your machine | Data sent to third party |
| **Latency** | ~2ms (in-process, no network) | 100-500ms (HTTP round-trip) |
| **Offline** | Works without internet | Requires internet |
| **Setup** | Automatic, no API key needed | Requires API key + billing |
| **Quality** | Excellent for code/session recall (768-dim) | Slightly better for general-purpose |
| **Size** | ~300 MB one-time download | N/A |

### User-Defined Slash Commands

Tell OpenCrabs in natural language: *"Create a /deploy command that runs deploy.sh"* — and it writes the command to `~/.opencrabs/commands.toml` via the `config_manager` tool:

```toml
[[commands]]
name = "/deploy"
description = "Deploy to staging server"
action = "prompt"
prompt = "Run the deployment script at ./src/scripts/deploy.sh for the staging environment."
```

Commands appear in autocomplete alongside built-in commands. After each agent response, `commands.toml` is automatically reloaded — no restart needed. Legacy `commands.json` files are auto-migrated on first load.

### Self-Sustaining Architecture

OpenCrabs can modify its own source code, build, test, and hot-restart itself — triggered by the agent via the `rebuild` tool or by the user via `/rebuild`:

```
/rebuild          # User-triggered: build → restart prompt
rebuild tool      # Agent-triggered: build → ProgressEvent::RestartReady → restart prompt
```

**How it works:**

1. The agent edits source files using its built-in tools (read, write, edit, bash)
2. `SelfUpdater::build()` runs `cargo build --release` asynchronously
3. On success, a `ProgressEvent::RestartReady` is emitted → bridged to `TuiEvent::RestartReady`
4. The TUI switches to **RestartPending** mode — user presses Enter to confirm
5. `SelfUpdater::restart(session_id)` replaces the process via Unix `exec()`
6. The new binary starts with `opencrabs chat --session <uuid>` — resuming the same conversation
7. A hidden wake-up message is sent to the agent so it greets the user and continues where it left off

**Two trigger paths:**

| Path | Entry point | Signal |
|------|-------------|--------|
| **Agent-triggered** | `rebuild` tool (called by the agent after editing source) | `ProgressCallback` → `RestartReady` |
| **User-triggered** | `/rebuild` slash command | `TuiEvent::RestartReady` directly |

**Key details:**

- The running binary is in memory — source changes on disk don't affect it until restart
- If the build fails, the agent stays running and can read compiler errors to fix them
- Session persistence via SQLite means no conversation context is lost across restarts
- After restart, the agent auto-wakes with session context — no user input needed
- Brain files (`SOUL.md`, `MEMORY.md`, etc.) are re-read every turn, so edits take effect immediately without rebuild
- User-defined slash commands (`commands.toml`) also auto-reload after each agent response
- Hot restart is Unix-only (`exec()` syscall); on Windows the build/test steps work but restart requires manual relaunch

**Modules:**
- `src/brain/self_update.rs` — `SelfUpdater` struct with `auto_detect()`, `build()`, `test()`, `restart()`
- `src/brain/tools/rebuild.rs` — `RebuildTool` (agent-callable, emits `ProgressEvent::RestartReady`)

### Self-Improving Agent

OpenCrabs learns from experience through three local mechanisms — no data ever leaves your machine:

**1. Procedural memory — custom commands from experience**
When the agent completes a complex workflow, overcomes errors, or follows user corrections, it can save that workflow as a reusable slash command via `config_manager add_command`. Next session, the command appears in autocomplete and the agent knows it exists.

**2. Episodic memory — lessons learned**
The agent writes important knowledge to `~/.opencrabs/` brain files as it works:
- `MEMORY.md` — infrastructure details, troubleshooting patterns, architecture decisions
- `USER.md` — your preferences, communication style, project context
- `memory/YYYY-MM-DD.md` — daily logs of integrations, fixes, and decisions
- Custom files (e.g., `DEPLOY.md`) — domain-specific knowledge

**3. Cross-session recall — hybrid search**
The `memory_search` and `session_search` tools use hybrid FTS5 + vector semantic search (Reciprocal Rank Fusion) to find relevant context from past sessions and memory files. Local embeddings via `embeddinggemma-300M` — no API calls needed.

**Key difference from cloud-based "self-improving" agents:** Your memory files, commands, and brain files are 100% local and belong to you. With local models (LM Studio, Ollama), everything stays on your machine. With cloud providers (Anthropic, MiniMax, OpenRouter), conversations go through their APIs — but these providers are privacy-first by default per their ToS, and you can opt out of logging and training data in their settings. Either way, your self-improvement data (skills, memory, commands) never leaves your machine.

---

## ⏰ Cron Jobs & Heartbeats

OpenCrabs runs as a daemon on your machine — a persistent terminal agent that's always on. This makes scheduled tasks and background jobs native and trivial.

### Cron Jobs — Scheduled Isolated Sessions

Cron jobs run as isolated sessions in the background. Each job gets its own session, provider, model, and context — completely independent from your main chat.

```bash
# Add a cron job via CLI
opencrabs cron add \
  --name "morning-briefing" \
  --cron "0 9 * * *" \
  --tz "America/New_York" \
  --prompt "Check my email, calendar, and weather. Send a morning briefing." \
  --deliver telegram:123456789

# List all jobs
opencrabs cron list

# Enable/disable
opencrabs cron enable morning-briefing
opencrabs cron disable morning-briefing

# Remove
opencrabs cron remove morning-briefing
```

The agent can also create, list, and manage cron jobs autonomously via the `cron_manage` tool — from any channel:

> "Set up a cron job that checks my Trello board every 2 hours and pings me on Telegram if any card is overdue"

| Option | Default | Description |
|--------|---------|-------------|
| `--cron` | required | Standard cron expression (e.g. `"0 9 * * *"`) |
| `--tz` | `UTC` | Timezone for the schedule |
| `--prompt` | required | The instruction to execute |
| `--provider` | `[cron]` default or current | Override provider (e.g. `anthropic`, `gemini`, `minimax`) |
| `--model` | `[cron]` default or current | Override model |
| `--thinking` | `off` | Thinking mode: `off`, `on`, `budget` |
| `--auto-approve` | `true` | Auto-approve tool calls (isolated sessions) |
| `--deliver` | none | Channel to deliver results (e.g. `telegram:123456`, `discord:789`, `slack:C0123`) |

**Provider priority:** per-job `--provider` > `[cron] default_provider` in config.toml > session's active provider. Set a global default for cron jobs to route them to a cheaper provider while keeping your interactive session on a premium one:

```toml
[cron]
default_provider = "minimax"
default_model = "MiniMax-M2.7"
```

### Heartbeats — Proactive Background Checks

When running as a daemon, OpenCrabs can perform periodic heartbeat checks. Configure `HEARTBEAT.md` in your workspace (`~/.opencrabs/HEARTBEAT.md`) with a checklist of things to monitor:

```markdown
# Heartbeat Checklist
- Check for urgent unread emails
- Check calendar for events in the next 2 hours
- If anything needs attention, message me on Telegram
- Otherwise, reply HEARTBEAT_OK
```

The heartbeat prompt is loaded into the agent's brain every turn. When the heartbeat fires, the agent reads `HEARTBEAT.md` and acts on it — checking email, calendar, notifications, or whatever you've configured.

### Heartbeat vs Cron

| | Heartbeat | Cron Job |
|---|-----------|----------|
| **Timing** | Periodic (every N minutes) | Exact schedule (cron expression) |
| **Session** | Main session (shared context) | Isolated session (independent) |
| **Context** | Has conversation history | Fresh context each run |
| **Use case** | Batch periodic checks | Standalone scheduled tasks |
| **Model** | Current session model | Configurable per job |
| **Cost** | Single turn per cycle | Full session per run |

**Rule of thumb:** Use heartbeats for lightweight monitoring that benefits from conversation context. Use cron jobs for standalone tasks that need exact timing, different models, or isolation.

### Autostart on Boot

To keep OpenCrabs always running, set it to start automatically with your system.

> **Profile-aware setup:** For named profiles, use `opencrabs -p <name> service install` instead of manual configuration. It generates the correct service name (`com.opencrabs.daemon.<name>` on macOS, `opencrabs-<name>.service` on Linux), includes the `-p` flag in the daemon args, and isolates log paths per profile. Multiple profiles can run as simultaneous daemon services.

The examples below show manual setup for the **default profile**. For named profiles, replace `daemon` with `-p <name> daemon` in the command arguments.

#### Linux (systemd)

```bash
mkdir -p ~/.config/systemd/user

cat > ~/.config/systemd/user/opencrabs.service << 'EOF'
[Unit]
Description=OpenCrabs AI Agent
After=network.target

[Service]
ExecStart=%h/.cargo/bin/opencrabs daemon
Restart=on-failure
RestartSec=5
Environment=OPENCRABS_HOME=%h/.opencrabs

[Install]
WantedBy=default.target
EOF

systemctl --user daemon-reload
systemctl --user enable opencrabs
systemctl --user start opencrabs

# Check status
systemctl --user status opencrabs

# View logs
journalctl --user -u opencrabs -f
```

> Replace `%h/.cargo/bin/opencrabs` with the actual path to your binary if you installed it elsewhere (e.g. `/usr/local/bin/opencrabs`).

#### macOS (launchd)

```bash
cat > ~/Library/LaunchAgents/com.opencrabs.agent.plist << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.opencrabs.agent</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/opencrabs</string>
        <string>daemon</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/tmp/opencrabs.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/opencrabs.err</string>
</dict>
</plist>
EOF

launchctl load ~/Library/LaunchAgents/com.opencrabs.agent.plist

# Check status
launchctl list | grep opencrabs

# Stop and unload
launchctl unload ~/Library/LaunchAgents/com.opencrabs.agent.plist
```

> Update the path in `ProgramArguments` to match your install. For cargo installs: `~/.cargo/bin/opencrabs`. For source builds: `/path/to/target/release/opencrabs`.

#### Windows (Task Scheduler)

1. Press `Win + R`, type `taskschd.msc`, hit Enter
2. Click **Create Basic Task** in the right panel
3. Name: `OpenCrabs`, Description: `OpenCrabs AI Agent`
4. Trigger: **When I log on**
5. Action: **Start a program**
6. Program: `C:\Users\<you>\.cargo\bin\opencrabs.exe` (or wherever your binary lives)
7. Arguments: `daemon`
8. Check **Open the Properties dialog** before finishing
9. In Properties > Settings, check **If the task fails, restart every 1 minute**

Or via PowerShell:

```powershell
$action = New-ScheduledTaskAction -Execute "$env:USERPROFILE\.cargo\bin\opencrabs.exe" -Argument "daemon"
$trigger = New-ScheduledTaskTrigger -AtLogon
$settings = New-ScheduledTaskSettingsSet -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1)
Register-ScheduledTask -TaskName "OpenCrabs" -Action $action -Trigger $trigger -Settings $settings -Description "OpenCrabs AI Agent"
```

> All platforms: cron jobs, heartbeats, and channel listeners (Telegram, Discord, Slack, WhatsApp) work in daemon mode. The TUI is not needed for background operation.

---

## 🏗️ Architecture

```
Presentation Layer
    ↓
CLI (Clap) + TUI (Ratatui + Crossterm)
    ↓
Brain Layer (Dynamic system brain, user commands, config management, self-update)
    ↓
Application Layer
    ↓
Service Layer (Session, Message, Agent, Plan)
    ↓
Data Access Layer (SQLx + SQLite)
    ↓
Integration Layer (LLM Providers, LSP)
```

**Key Technologies:**

| Component | Crate |
|-----------|-------|
| Async Runtime | Tokio |
| Terminal UI | Ratatui + Crossterm |
| CLI Parsing | Clap (derive) |
| Database | SQLx (SQLite) |
| Serialization | Serde + TOML |
| HTTP Client | Reqwest |
| Syntax Highlighting | Syntect |
| Markdown | pulldown-cmark |
| LSP Client | Tower-LSP |
| Provider Registry | Crabrace |
| Memory Search | qmd (FTS5 + vector embeddings) |
| Error Handling | anyhow + thiserror |
| Logging | tracing + tracing-subscriber |
| Security | zeroize |

---

## 📁 Project Structure

```
opencrabs/
├── src/
│   ├── main.rs           # Entry point
│   ├── lib.rs            # Library root (crate root — required by Rust)
│   ├── error/            # Error types (OpenCrabsError, ErrorCode)
│   ├── logging/          # Conditional logging system
│   ├── app/              # Application lifecycle
│   ├── brain/            # Intelligence layer — LLM providers, agent, tools, brain system
│   │   ├── agent/        # Agent service + context management
│   │   ├── provider/     # Provider implementations (Anthropic, GitHub Copilot, OpenAI-Compatible: OpenRouter, Minimax, z.ai GLM, Custom)
│   │   ├── tools/        # Tool system (read, write, bash, glob, grep, memory_search, etc.)
│   │   ├── tokenizer.rs  # Token counting (tiktoken-based)
│   │   ├── prompt_builder.rs  # BrainLoader — assembles system brain from workspace files
│   │   ├── commands.rs   # CommandLoader — user-defined slash commands (TOML)
│   │   └── self_update.rs # SelfUpdater — build, test, hot-restart via exec()
│   ├── channels/         # Messaging integrations + voice (feature-gated)
│   │   ├── factory.rs    # ChannelFactory — shared factory for channel agent services
│   │   ├── telegram/     # Telegram bot (agent, handler)
│   │   ├── whatsapp/     # WhatsApp Web client (agent, handler, store)
│   │   ├── discord/      # Discord bot (agent, handler)
│   │   ├── slack/        # Slack bot via Socket Mode (agent, handler)
│   │   ├── trello/       # Trello board poller (agent, client, handler, models)
│   │   └── voice/        # STT (Groq Whisper / whisper.cpp) + TTS (OpenAI / Piper)
│   ├── cli/              # Command-line interface (Clap)
│   ├── config/           # Configuration (config.toml + keys.toml)
│   ├── db/               # Database layer (SQLx + SQLite)
│   ├── services/         # Business logic (Session, Message, File, Plan)
│   ├── memory/           # Memory search (FTS5 + vector embeddings via qmd)
│   ├── tui/              # Terminal UI (Ratatui)
│   │   ├── onboarding.rs     # 8-step onboarding wizard (state + logic)
│   │   ├── onboarding_render.rs  # Wizard rendering
│   │   ├── splash.rs     # Splash screen
│   │   ├── app.rs        # App state + event handling
│   │   ├── render.rs     # Main render dispatch
│   │   └── runner.rs     # TUI event loop
│   ├── utils/            # Utilities (retry, etc.)
│   ├── migrations/       # SQLite migrations
│   ├── tests/            # 1,424 tests (see TESTING.md)
│   ├── benches/          # Criterion benchmarks
│   ├── assets/           # Icons, screenshots, visual assets
│   ├── scripts/          # Build and setup scripts
│   └── docs/             # Documentation templates
├── Cargo.toml
├── config.toml.example
├── keys.toml.example
└── LICENSE.md
```

---

## 🛠️ Development

### Build from Source

```bash
# Development build
cargo build

# Release build (optimized, LTO, stripped)
cargo build --release

# Small release build
cargo build --profile release-small

# Run tests (1,424 tests across 60+ modules)
cargo test --all-features
# See TESTING.md for full test coverage documentation

# Run benchmarks
cargo bench

# Format + lint
cargo fmt
cargo clippy -- -D warnings
```

### Feature Flags

| Feature | Description |
|---------|-------------|
| `telegram` | Telegram bot integration (default: enabled) |
| `whatsapp` | WhatsApp Web integration (default: enabled) |
| `discord` | Discord bot integration (default: enabled) |
| `slack` | Slack bot integration (default: enabled) |
| `trello` | Trello board polling + card management (default: enabled) |
| `local-stt` | Local speech-to-text via rwhisper (candle-based, pure Rust) |
| `local-tts` | Local text-to-speech via Piper (requires python3) |
| `profiling` | Enable pprof flamegraph profiling (Unix only) |

### Performance

| Metric | Value |
|--------|-------|
| Binary size | 34 MB (release, stripped, LTO) |
| RAM idle (RSS) | 57 MB |
| RAM active (100 msgs) | ~20 MB |
| Startup time | < 50 ms |
| Database ops | < 10 ms (session), < 5 ms (message) |
| Embedding engine | embeddinggemma-300M (~300 MB, local GGUF, auto-downloaded) |

#### Memory Search (qmd — FTS5 + Vector Embeddings)

Hybrid semantic search: FTS5 BM25 keyword matching + 768-dim vector embeddings combined via Reciprocal Rank Fusion. Embedding model runs locally — **no API key, zero cost, works offline**.


Benchmarked with `cargo bench --bench memory` on release builds:

| Operation | Time | Notes |
|-----------|------|-------|
| Store open | 1.81 ms | Cold start (create DB + schema) |
| Index file | 214 µs | Insert content + document |
| Hash skip | 19.5 µs | Already indexed, unchanged — fast path |
| FTS search (10 docs) | 397 µs | 2-term BM25 query |
| FTS search (50 docs) | 2.57 ms | Typical user corpus |
| FTS search (100 docs) | 9.22 ms | |
| FTS search (500 docs) | 88.1 ms | Large corpus |
| Vector search (10 docs) | 247 µs | 768-dim cosine similarity |
| Vector search (50 docs) | 1.02 ms | 768-dim cosine similarity |
| Vector search (100 docs) | 2.04 ms | 768-dim cosine similarity |
| Hybrid RRF (50 docs) | 3.49 ms | FTS + vector → Reciprocal Rank Fusion |
| Insert embedding | 301 µs | Single 768-dim vector |
| Bulk reindex (50 files) | 11.4 ms | From cold, includes store open |
| Deactivate document | 267 µs | Prune a single entry |

**Benchmarks** (release build, in-memory SQLite, criterion):

| Operation | Time |
|---|---|
| Index 50 files (first run) | 11.4 ms |
| Per-file index | 214 µs |
| Hash skip (unchanged file) | 19.5 µs |
| FTS search (10 docs) | 397 µs |
| FTS search (50 docs) | 2.57 ms |
| FTS search (100 docs) | 9.2 ms |
| Vector search (10 docs, 768-dim) | 247 µs |
| Vector search (50 docs, 768-dim) | 1.02 ms |
| Vector search (100 docs, 768-dim) | 2.04 ms |
| Hybrid RRF (FTS + vector, 50 docs) | 3.49 ms |
| Insert embedding | 301 µs |
| Deactivate document | 267 µs |

---

## 🐛 Platform Notes

> **Automated setup:** Run `src/scripts/setup.sh` to detect your platform and install everything automatically.

### Linux

```bash
# Debian/Ubuntu
sudo apt-get install build-essential pkg-config libssl-dev cmake

# Fedora/RHEL
sudo dnf install gcc gcc-c++ make pkg-config openssl-devel cmake

# Arch
sudo pacman -S base-devel pkg-config openssl cmake
```

#### Older CPUs (Sandy Bridge / AVX-only)

The default release binary requires AVX2 (Haswell 2013+). If you have an older CPU with only AVX support (Sandy Bridge/Ivy Bridge, 2011-2012), build from source with:

```bash
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

Pre-built `*-compat` binaries are also available on the [releases page](https://github.com/adolfousier/opencrabs/releases) for AVX-only CPUs. If your CPU lacks AVX entirely (pre-2011), vector embeddings are disabled and search falls back to FTS-only keyword matching.

### macOS

Requires **macOS 15 (Sequoia)** or later.

```bash
# Install build dependencies
brew install cmake pkg-config
```

#### Metal GPU crash on macOS 14 (Sonoma) or older

If you see this error when running OpenCrabs:

```
dyld: Symbol not found: _OBJC_CLASS_$_MTLResidencySetDescriptor
```

This happens because `llama.cpp` (used for local embeddings) compiles with Metal GPU support and unconditionally links Metal frameworks that require macOS 15+. There is currently no way to disable Metal at build time through the Rust `llama-cpp-sys-2` crate.

**Fix:** Update to macOS 15 (Sequoia) or later.

### Windows

Requires CMake, NASM, and Visual Studio Build Tools for native crypto dependencies:

```bash
# Option 1: Install build tools
# - CMake (add to PATH)
# - NASM (add to PATH)
# - Visual Studio Build Tools ("Desktop development with C++")

# Option 2: Use WSL2 (recommended)
sudo apt-get install build-essential pkg-config libssl-dev
```

See [BUILD_NOTES.md](src/docs/guides/BUILD_NOTES.md) for detailed troubleshooting.

---

## 🔧 Troubleshooting

### Agent Hallucinating Tool Calls

If the agent starts sending tool call approvals that don't render in the UI — meaning it believes it executed actions that never actually ran — the session context has become corrupted.

**Fix:** Start a new session.

1. Press `/` and type `sessions` (or navigate to the Sessions panel)
2. Press **N** to create a new session
3. Continue your work in the fresh session

This reliably resolves the issue. A fix is coming in a future release.

### Windows Defender Blocking OpenCrabs

Windows Defender (or other antivirus software) may flag `opencrabs.exe` as suspicious because it's an unsigned binary that executes shell commands and makes network requests. This is a false positive.

**Fix — Add an exclusion:**

1. Open **Windows Security** → **Virus & threat protection**
2. Scroll to **Virus & threat protection settings** → **Manage settings**
3. Scroll to **Exclusions** → **Add or remove exclusions**
4. Click **Add an exclusion** → **File** → select `opencrabs.exe`

**Or via PowerShell (admin):**
```powershell
Add-MpPreference -ExclusionPath "C:\path\to\opencrabs.exe"
```

If SmartScreen blocks the first run, click **More info** → **Run anyway**.

---

## 🧩 Companion Tools

### WhisperCrabs — Voice-to-Text

[WhisperCrabs](https://github.com/adolfousier/whispercrabs) is a floating voice-to-text tool. Click to record, click to stop, transcribes, copies to clipboard.

- **Local** (whisper.cpp, on-device) or **API** transcription
- Fully controllable via D-Bus — start/stop recording, switch providers, view history
- Works as an OpenCrabs tool: use D-Bus to control WhisperCrabs from the agent

### SocialCrabs — Social Media Automation

[SocialCrabs](https://github.com/adolfousier/socialcrabs) automates social media via **CLI + GraphQL** with human-like behavior simulation. Twitter/X, Instagram, LinkedIn. No browser needed for read operations.

**Setup:**
```bash
git clone https://github.com/adolfousier/socialcrabs.git
cd socialcrabs && npm install && npm run build

# Add cookies from browser DevTools to .env (auth_token + ct0 for Twitter)
# See SocialCrabs README for per-platform credential setup

node dist/cli.js session login x          # Authenticate Twitter/X
node dist/cli.js session login ig         # Authenticate Instagram
node dist/cli.js session login linkedin   # Authenticate LinkedIn
node dist/cli.js session status           # Check all sessions
```

**Usage with OpenCrabs:** Just ask naturally. OpenCrabs calls SocialCrabs CLI commands via `bash` automatically:

> "Check my Twitter mentions" / "Search LinkedIn for AI founders" / "Post this to X"

Read operations run automatically. Write operations (tweet, like, follow, comment, DM) always ask for your approval first.

**Twitter/X commands:**
```bash
node dist/cli.js x whoami                     # Check logged-in account
node dist/cli.js x mentions -n 5              # Your mentions
node dist/cli.js x home -n 5                  # Your timeline
node dist/cli.js x search "query" -n 10       # Search tweets
node dist/cli.js x read <tweet-url>           # Read a specific tweet
node dist/cli.js x tweet "Hello world"        # Post a tweet
node dist/cli.js x reply <tweet-url> "text"   # Reply to tweet
node dist/cli.js x like <tweet-url>           # Like a tweet
node dist/cli.js x follow <username>          # Follow a user
```

**Instagram commands:**
```bash
node dist/cli.js ig like <post-url>
node dist/cli.js ig comment <post-url> "text"
node dist/cli.js ig dm <username> "message"
node dist/cli.js ig follow <username>
node dist/cli.js ig followers <username> -n 10
node dist/cli.js ig posts <username> -n 3
```

**LinkedIn commands:**
```bash
node dist/cli.js linkedin like <post-url>
node dist/cli.js linkedin comment <post-url> "text"
node dist/cli.js linkedin connect <profile-url>
node dist/cli.js linkedin search "query" -n 10
node dist/cli.js linkedin engage --query="query"   # Full engagement session
```

**Features:** Human-like behavior (randomized delays, natural typing), session persistence across restarts, built-in rate limiting, anti-detection, research-first workflow (scrape targets first, distribute engagement over time).

---

## ⚠️ Disclaimers

### Development Status

OpenCrabs is under active development. While functional, it may contain bugs or incomplete features.

### Token Cost Responsibility

**You are responsible for monitoring and managing your own API usage and costs.**

- API costs from cloud providers (Anthropic, OpenAI, etc.) are your responsibility
- Set billing alerts with your provider
- Consider local LLMs for cost-free operation
- Use the built-in cost tracker to monitor spending

### Support

Cloud API issues, billing questions, and account problems should be directed to the respective providers. OpenCrabs provides the tool; you manage your API relationships.

---

## 🤝 Contributing

Contributions welcome! Please read [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

```bash
# Setup
git clone https://github.com/adolfousier/opencrabs.git
cd opencrabs
cargo build
cargo test
# Make changes, then submit a PR
```

---

## 📄 License

**MIT License** — See [LICENSE.md](LICENSE.md) for details.

---

## 🙏 Acknowledgments

- **[Claude Code](https://github.com/anthropics/claude-code)** — Inspiration
- **[Crabrace](https://crates.io/crates/crabrace)** — Provider registry
- **[Ratatui](https://ratatui.rs/)** — Terminal UI framework
- **[Anthropic](https://anthropic.com/)** — Claude API

---

## 📞 Support

- **Issues:** [GitHub Issues](https://github.com/adolfousier/opencrabs/issues)
- **Discussions:** [GitHub Discussions](https://github.com/adolfousier/opencrabs/discussions)
- **Docs:** [Documentation](https://github.com/adolfousier/opencrabs/tree/main/src/docs)

---

## Star History Chart

[![Star History Chart](https://api.star-history.com/svg?repos=adolfousier/opencrabs&type=date&legend=top-left)](https://www.star-history.com/#adolfousier/opencrabs)

## ✨ Stay Tuned

---

**Built with Rust 🦀 by [Adolfo Usier](https://github.com/adolfousier)**

---

[Latest Release](https://github.com/adolfousier/opencrabs/releases/tag/v0.2.81)
