[![Rust](https://img.shields.io/badge/rust-%23000000.svg?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Rust Edition](https://img.shields.io/badge/rust-2024_edition-orange.svg)](https://www.rust-lang.org/)
[![Ratatui](https://img.shields.io/badge/ratatui-%23000000.svg?style=for-the-badge&logo=rust&logoColor=white)](https://ratatui.rs)
[![Docker](https://img.shields.io/badge/docker-%23000000.svg?style=for-the-badge&logo=docker&logoColor=white)](https://docker.com)
[![CI](https://github.com/adolfousier/opencrabs/actions/workflows/ci.yml/badge.svg)](https://github.com/adolfousier/opencrabs/actions/workflows/ci.yml)
[![GitHub Stars](https://img.shields.io/github/stars/adolfousier/opencrabs?style=social)](https://github.com/adolfousier/opencrabs)

# 🦀 OpenCrabs

**The autonomous AI agent. Single Rust binary. Every channel.**

> Autonomous multi-channel AI agent built in Rust. Inspired by [Open Claw](https://github.com/openclaw/openclaw).

```
    ___                    ___           _
   / _ \ _ __  ___ _ _    / __|_ _ __ _| |__  ___
  | (_) | '_ \/ -_) ' \  | (__| '_/ _` | '_ \(_-<
   \___/| .__/\___|_||_|  \___|_| \__,_|_.__//__/
        |_|

 🦀 The autonomous AI agent. Single Rust binary. Every channel.

```

**Author:** [Adolfo Usier](https://github.com/adolfousier)

⭐ Star us on [GitHub](https://github.com/adolfousier/opencrabs) if you like what you see!

---

## Table of Contents

- [Screenshots](#-screenshots)
- [Core Features](#-core-features)
- [Supported AI Providers](#-supported-ai-providers)
- [Agent-to-Agent (A2A) Protocol](#-agent-to-agent-a2a-protocol)
- [Quick Start](#-quick-start)
- [Onboarding Wizard](#-onboarding-wizard)
- [API Keys (keys.toml)](#-api-keys-keystoml)
- [Configuration (config.toml)](#-configuration-configtoml)
- [Commands (commands.toml)](#-commands-commandstoml)
- [Using Local LLMs](#-using-local-llms)
- [Configuration](#-configuration)
- [Tool System](#-tool-system)
- [Keyboard Shortcuts](#-keyboard-shortcuts)
- [Debug and Logging](#-debug-and-logging)
- [Architecture](#-architecture)
- [Project Structure](#-project-structure)
- [Development](#-development)
- [Platform Notes](#-platform-notes)
- [Troubleshooting](#-troubleshooting)
- [Disclaimers](#-disclaimers)
- [Contributing](#-contributing)
- [License](#-license)
- [Acknowledgments](#-acknowledgments)

---

## 📸 Screenshots

[![Demo](src/screenshots/opencrabs-demo.gif)](https://github.com/user-attachments/assets/7f45c5f8-acdf-48d5-b6a4-0e4811a9ee23)

---

## 🎯 Core Features

### AI & Providers
| Feature | Description |
|---------|-------------|
| **Multi-Provider** | Anthropic Claude, OpenAI, OpenRouter (400+ models), MiniMax, and any OpenAI-compatible API (Ollama, LM Studio, LocalAI). Model lists fetched live from provider APIs — new models available instantly. Each session remembers its provider + model and restores it on switch |
| **Real-time Streaming** | Character-by-character response streaming with animated spinner showing model name and live text |
| **Local LLM Support** | Run with LM Studio, Ollama, or any OpenAI-compatible endpoint — 100% private, zero-cost |
| **Cost Tracking** | Per-message token count and cost displayed in header; `/usage` shows all-time breakdown grouped by model with real costs + estimates for historical sessions |
| **Context Awareness** | Live context usage indicator showing actual token counts (e.g. `ctx: 45K/200K (23%)`); auto-compaction at 70% with tool overhead budgeting; accurate tiktoken-based counting calibrated against API actuals |
| **3-Tier Memory** | (1) **Brain MEMORY.md** — user-curated durable memory loaded every turn, (2) **Daily Logs** — auto-compaction summaries at `~/.opencrabs/memory/YYYY-MM-DD.md`, (3) **Hybrid Memory Search** — FTS5 keyword search + local vector embeddings (embeddinggemma-300M, 768-dim) combined via Reciprocal Rank Fusion. Runs entirely local — no API key, no cost, works offline |
| **Dynamic Brain System** | System brain assembled from workspace MD files (SOUL, IDENTITY, USER, AGENTS, TOOLS, MEMORY) — all editable live between turns |

### Multimodal Input
| Feature | Description |
|---------|-------------|
| **Image Attachments** | Paste image paths or URLs into the input — auto-detected and attached as vision content blocks for multimodal models |
| **PDF Support** | Attach PDF files by path — native Anthropic PDF support; for other providers, text is extracted locally via `pdf-extract` |
| **Document Parsing** | Built-in `parse_document` tool extracts text from PDF, DOCX, HTML, TXT, MD, JSON, XML |
| **Voice (STT)** | Telegram voice notes transcribed via Groq Whisper (`whisper-large-v3-turbo`) and processed as text. API key in `keys.toml` |
| **Voice (TTS)** | Agent replies to voice notes with audio via OpenAI TTS (`gpt-4o-mini-tts`, `ash` voice); falls back to text if disabled |
| **Attachment Indicator** | Attached images show as `[IMG1:filename.png]` in the input title bar |
| **Image Generation** | Agent generates images via Google Gemini (`gemini-3.1-flash-image-preview` "Nano Banana") using the `generate_image` tool — enabled via `/onboard:image`. Returned as native images/attachments in all channels |

### Messaging Integrations
| Feature | Description |
|---------|-------------|
| **Telegram Bot** | Full-featured Telegram bot — shared session with TUI, photo/voice support, allowed user IDs, allowed chat/group IDs, `respond_to` filter (`all`/`dm_only`/`mention`) |
| **WhatsApp** | Connect via QR code pairing at runtime or from onboarding wizard. Text + image, shared session with TUI, phone allowlist (`allowed_phones`), session persists across restarts |
| **Discord** | Full Discord bot — text + image + voice, allowed user IDs, allowed channel IDs, `respond_to` filter, shared session with TUI. Full proactive control via `discord_send` (17 actions): `send`, `reply`, `react`, `unreact`, `edit`, `delete`, `pin`, `unpin`, `create_thread`, `send_embed`, `get_messages`, `list_channels`, `add_role`, `remove_role`, `kick`, `ban`, `send_file`. Generated images sent as native Discord file attachments |
| **Slack** | Full Slack bot via Socket Mode — allowed user IDs, allowed channel IDs, `respond_to` filter, shared session with TUI. Full proactive control via `slack_send` (17 actions): `send`, `reply`, `react`, `unreact`, `edit`, `delete`, `pin`, `unpin`, `get_messages`, `get_channel`, `list_channels`, `get_user`, `list_members`, `kick_user`, `set_topic`, `send_blocks`, `send_file`. Generated images sent as native Slack file uploads. Bot token + app token from `api.slack.com/apps` (Socket Mode required) |
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
| **Built-in Tools** | Read/write/edit files, bash, glob, grep, web search (DuckDuckGo + EXA default, no key needed; Brave optional), and more |
| **Per-Session Isolation** | Each session is an independent agent with its own provider, model, context, and tool state. Sessions can run tasks in parallel against different providers — ask Claude a question in one session while Kimi works on code in another |
| **Self-Sustaining** | Agent can modify its own source, build, test, and hot-restart via Unix `exec()` |
| **Natural Language Commands** | Tell OpenCrabs to create slash commands — it writes them to `commands.toml` autonomously via the `config_manager` tool |
| **Live Settings** | Agent can read/write `config.toml` at runtime; Settings TUI screen (press `S`) shows current config; approval policy persists across restarts. Default: auto-approve (use `/approve` to change) |
| **Web Search** | DuckDuckGo (built-in, no key needed) + EXA AI (neural, free via MCP) by default; Brave Search optional (key in `keys.toml`) |
| **Debug Logging** | `--debug` flag enables file logging; `DEBUG_LOGS_LOCATION` env var for custom log directory |
| **Agent-to-Agent (A2A)** | HTTP gateway implementing A2A Protocol RC v1.0 — peer-to-peer agent communication via JSON-RPC 2.0. Supports `message/send`, `tasks/get`, `tasks/cancel`. Includes multi-agent debate (Bee Colony) with confidence-weighted consensus. Loopback-only by default; CORS origins must be explicitly configured |

---

## 🌐 Supported AI Providers

### Anthropic Claude

**Models:** `claude-opus-4-6`, `claude-sonnet-4-5-20250929`, `claude-haiku-4-5-20251001`, plus legacy Claude 3.x models

**Setup** in `keys.toml`:
```toml
[providers.anthropic]
api_key = "sk-ant-api03-YOUR_KEY"
```

OAuth tokens (`sk-ant-oat` prefix) are auto-detected — uses `Authorization: Bearer` with `anthropic-beta: oauth-2025-04-20` header automatically.

**Features:** Streaming, tools, cost tracking, automatic retry with backoff

### OpenAI

**Models:** GPT-5 Turbo, GPT-5

**Setup** in `keys.toml`:
```toml
[providers.openai]
api_key = "sk-YOUR_KEY"
```

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

**Models:** `MiniMax-M2.5`, `MiniMax-M2.1`, `MiniMax-Text-01`

**Setup** — get your API key from [platform.minimax.io](https://platform.minimax.io). Add to `keys.toml`:

```toml
[providers.minimax]
api_key = "your-api-key"
```

MiniMax is an OpenAI-compatible provider with competitive pricing. It does not expose a `/models` endpoint, so the model list comes from `config.toml` (pre-configured with available models).

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

**Provider priority:** MiniMax > OpenRouter > Anthropic > OpenAI > Gemini > Custom. The first provider with `enabled = true` is used on new sessions. Each provider has its own API key in `keys.toml` — no sharing or confusion.

**Per-session provider:** Each session remembers which provider and model it was using. Switch to Claude in one session, Kimi in another — when you `/sessions` switch between them, the provider restores automatically. No need to `/models` every time. New sessions inherit the current provider.

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

OpenCrabs includes a built-in A2A gateway — an HTTP server implementing the [A2A Protocol RC v1.0](https://google.github.io/A2A/) for peer-to-peer agent communication. Other A2A-compatible agents can discover OpenCrabs, send it tasks, and get results back — all via standard JSON-RPC 2.0.

### Enabling A2A

Add to `~/.opencrabs/config.toml`:

```toml
[a2a]
enabled = true
bind = "127.0.0.1"   # Loopback only (default)
port = 18790          # Gateway port
# allowed_origins = ["http://localhost:3000"]  # CORS (empty = blocked)
```

No API keys required — A2A is config-only.

### Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/.well-known/agent.json` | GET | Agent Card — discover skills, capabilities, supported content types |
| `/a2a/v1` | POST | JSON-RPC 2.0 — `message/send`, `tasks/get`, `tasks/cancel` |
| `/a2a/health` | GET | Health check |

### Quick Start Examples

```bash
# Discover the agent
curl http://127.0.0.1:18790/.well-known/agent.json | jq .

# Send a message (creates a task)
curl -X POST http://127.0.0.1:18790/a2a/v1 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "message/send",
    "params": {
      "message": {
        "role": "user",
        "parts": [{"kind": "text", "text": "What tools do you have?"}]
      }
    }
  }'

# Poll a task by ID
curl -X POST http://127.0.0.1:18790/a2a/v1 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tasks/get","params":{"id":"TASK_ID"}}'

# Cancel a running task
curl -X POST http://127.0.0.1:18790/a2a/v1 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":3,"method":"tasks/cancel","params":{"id":"TASK_ID"}}'
```

### Bee Colony Debate

OpenCrabs supports multi-agent structured debate via the **Bee Colony** protocol — based on [ReConcile (ACL 2024)](https://arxiv.org/abs/2309.13007) confidence-weighted voting. Multiple "bee" agents argue across configurable rounds, each enriched with knowledge context from QMD memory search, then converge on a consensus answer with confidence scores.

### Security Notes

- **Loopback only** by default — binds to `127.0.0.1`, not `0.0.0.0`
- **CORS locked down** — no cross-origin requests unless `allowed_origins` is explicitly set
- **No authentication** built in — do not expose to public internet without a reverse proxy + auth layer

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

### Option 2: Build from Source (full control)

Required for `/rebuild`, adding custom tools, or modifying the agent.

**Prerequisites:**
- **Rust nightly (2024 edition)** — [Install Rust](https://rustup.rs/), then `rustup toolchain install nightly`. The project includes a `rust-toolchain.toml` that selects nightly automatically
- **An API key** from at least one supported provider
- **SQLite** (bundled via sqlx)
- **Linux:** `build-essential`, `pkg-config`, `libssl-dev`, `libchafa-dev`

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
| 2 | **Model & Auth** | Pick provider (Anthropic, OpenAI, Gemini, OpenRouter, Minimax, Custom) → enter token/key → model list fetched live from API → select model. Auto-detects existing keys from `keys.toml` |
| 3 | **Workspace** | Set brain workspace path (default `~/.opencrabs/`) → seed template files (SOUL.md, IDENTITY.md, etc.) |
| 4 | **Gateway** | Configure HTTP API gateway: port, bind address, auth mode |
| 5 | **Channels** | Toggle messaging integrations (Telegram, Discord, WhatsApp, Slack, Trello) |
| 6 | **Voice** | Enable STT (Groq Whisper) for voice notes transcription from Telegram |
| 7 | **Image Handling** | Enable Gemini image generation and/or vision analysis — uses a separate Google AI key |
| 8 | **Daemon** | Install background service (systemd on Linux, LaunchAgent on macOS) |
| 9 | **Health Check** | Verify API key, config, workspace — shows pass/fail summary |
| 10 | **Brain Personalization** | Tell the agent about yourself and how you want it to behave → AI generates personalized brain files (SOUL.md, IDENTITY.md, USER.md, etc.) |

**QuickStart mode** skips steps 4-8 with sensible defaults. **Advanced mode** lets you configure everything.

Type `/onboard:image` in chat to jump directly to the Image Handling step anytime.

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
api_key = "sk-ant-api03-YOUR_KEY"    # or OAuth: "sk-ant-oat01-..."

[providers.openai]
api_key = "sk-YOUR_KEY"

[providers.openrouter]
api_key = "sk-or-YOUR_KEY"

[providers.minimax]
api_key = "your-minimax-key"

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
[providers.stt.groq]
api_key = "your-groq-key"

[providers.tts.openai]
api_key = "your-openai-key"
```

OAuth tokens (`sk-ant-oat` prefix) are auto-detected — OpenCrabs uses `Authorization: Bearer` with the `anthropic-beta: oauth-2025-04-20` header automatically.

> **Trello note:** `app_token` holds the Trello **API Key** and `token` holds the Trello **API Token** — `app_token` is the app-level credential and `token` is the user-level credential. Board IDs are configured via `board_ids` in `config.toml`.

> **Security:** Always `chmod 600 ~/.opencrabs/keys.toml` and add `keys.toml` to `.gitignore`.

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

Changes to any of these files are picked up automatically within ~300ms while OpenCrabs is running. The active LLM provider, channel allowlists, approval policy, and slash command autocomplete all update without restart.

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
default_model = "gpt-4o"

# ── Image ─────────────────────────────────────────────────────────────────────

[image.generation]
enabled = true
model = "gemini-3.1-flash-image-preview"

[image.vision]
enabled = true
model = "gemini-3.1-flash-image-preview"
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

OpenCrabs includes a built-in tool execution system. The AI can use these tools during conversation:

| Tool | Description |
|------|-------------|
| `read_file` | Read file contents with syntax awareness |
| `write_file` | Create or modify files |
| `edit_file` | Precise text replacements in files |
| `bash` | Execute shell commands |
| `ls` | List directory contents |
| `glob` | Find files matching patterns |
| `grep` | Search file contents with regex |
| `web_search` | Search the web (DuckDuckGo, always available, no key needed) |
| `exa_search` | Neural web search via EXA AI (free via MCP, no API key needed; set key in `keys.toml` for higher rate limits) |
| `brave_search` | Web search via Brave Search (set key in `keys.toml` — free $5/mo credits at brave.com/search/api) |
| `execute_code` | Run code in various languages |
| `notebook_edit` | Edit Jupyter notebooks |
| `parse_document` | Extract text from PDF, DOCX, HTML |
| `task_manager` | Manage agent tasks |
| `http_request` | Make HTTP requests |
| `memory_search` | Hybrid semantic search across past memory logs — FTS5 keyword + vector embeddings (768-dim, local GGUF model) combined via RRF. No API key needed, runs offline |
| `config_manager` | Read/write config.toml and commands.toml at runtime (change settings, add/remove commands, reload config) |
| `session_context` | Access session information |
| `plan` | Create structured execution plans |

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
| `Alt+Enter` / `Shift+Enter` | New line in input |
| `←` / `→` | Move cursor one character |
| `Ctrl+←` / `Ctrl+→` | Jump by word |
| `Home` / `End` | Jump to start/end of input |
| `Delete` | Delete character after cursor |
| `Ctrl+Backspace` / `Alt+Backspace` | Delete word before cursor |
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
# Enable debug mode (creates log files)
opencrabs -d
cargo run -- -d

# Logs stored in ~/.opencrabs/logs/ (user workspace, not in repo)
# Daily rolling rotation, auto-cleanup after 7 days

# Management
opencrabs logs status    # Check logging status
opencrabs logs view      # View recent entries
opencrabs logs clean     # Clean old logs
opencrabs logs clean -d 3  # Clean logs older than 3 days
```

**When debug mode is enabled:**
- Log files created in `~/.opencrabs/logs/`
- DEBUG level with thread IDs, file names, line numbers
- Daily rolling rotation

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
prompt = "Run the deployment script at ./scripts/deploy.sh for the staging environment."
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
│   │   ├── provider/     # Provider implementations (Anthropic, OpenAI-Compatible: OpenRouter, Minimax, Custom)
│   │   ├── tools/        # Tool system (read, write, bash, glob, grep, memory_search, etc.)
│   │   ├── tokenizer.rs  # Token counting (tiktoken-based)
│   │   ├── prompt_builder.rs  # BrainLoader — assembles system brain from workspace files
│   │   ├── commands.rs   # CommandLoader — user-defined slash commands (TOML)
│   │   └── self_update.rs # SelfUpdater — build, test, hot-restart via exec()
│   ├── channels/         # Messaging integrations + voice (feature-gated)
│   │   ├── factory.rs    # ChannelFactory — shared factory for channel agent services
│   │   ├── telegram/     # Telegram bot (agent, handler)
│   │   ├── whatsapp/     # WhatsApp Web client (agent, handler, sqlx_store)
│   │   ├── discord/      # Discord bot (agent, handler)
│   │   ├── slack/        # Slack bot via Socket Mode (agent, handler)
│   │   ├── trello/       # Trello board poller (agent, client, handler, models)
│   │   └── voice/        # STT (Groq Whisper) + TTS (OpenAI)
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
│   ├── tests/            # Integration tests
│   ├── benches/          # Criterion benchmarks
│   └── docs/             # Documentation + screenshots
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

# Run tests
cargo test

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

### Linux

```bash
sudo apt-get install build-essential pkg-config libssl-dev libchafa-dev
```

#### Older CPUs (Sandy Bridge / AVX-only)

The default release binary requires AVX2 (Haswell 2013+). If you have an older CPU with only AVX support (Sandy Bridge/Ivy Bridge, 2011-2012), build from source with:

```bash
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

Pre-built `*-compat` binaries are also available on the [releases page](https://github.com/adolfousier/opencrabs/releases) for AVX-only CPUs. If your CPU lacks AVX entirely (pre-2011), vector embeddings are disabled and search falls back to FTS-only keyword matching.

### macOS

No additional dependencies required.

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

---

### SocialCrabs (Twitter/X, Instagram, LinkedIn)

SocialCrabs is the social media automation template for OpenCrabs. It provides two methods to interact with social platforms:

#### Method 1: CLI + GraphQL (Recommended)
The built-in CLI uses Twitter's GraphQL API directly — fast, reliable, no browser needed.

```bash
cd /path/to/socialcrabs

# Twitter/X
node dist/cli.js x whoami
node dist/cli.js x search "query" -n 10
node dist/cli.js x home -n 20
node dist/cli.js x mentions -n 10
node dist/cli.js x like <tweet-url>
node dist/cli.js x reply <tweet-url> "your reply"
node dist/cli.js x tweet "your post"

# Instagram
node dist/cli.js ig posts <username> -n 3
node dist/cli.js ig like <post-url>
node dist/cli.js ig comment <post-url> "your comment"

# LinkedIn
node dist/cli.js linkedin search "AI founder" -n 10
node dist/cli.js linkedin like <post-url>
node dist/cli.js linkedin comment <post-url> "your comment"
```

#### Method 2: Playwright Browser Automation
Uses Playwright to control a real browser — useful when GraphQL APIs are rate-limited or unavailable.

**Setup:**
```bash
cd /path/to/socialcrabs
npm install
playwright install chromium
```

**Run the HTTP server:**
```bash
node dist/server.js
# API available at http://localhost:3847
```

**Common Playwright Issues:**

| Error | Cause | Fix |
|-------|-------|-----|
| `Target page, context or browser has been closed` | Stale browser process or missing Chromium | `pkill -f chromium; pkill -f pw; rm -rf ~/.cache/ms-playwright/` then `playwright install chromium` |
| `Tweet input not found` | Twitter UI changed, selectors outdated | Use CLI + GraphQL method instead (recommended) |
| `Session expired` | Cookies invalid | Run `node dist/cli.js session login twitter` to get fresh cookies |

**Quick Test:**
```bash
npx playwright screenshot --browser chromium https://x.com /tmp/test.png
```

If this works but SocialCrabs doesn't, the issue is likely selector changes in Twitter's UI.

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
- **Docs:** [src/docs/](src/docs/)

---

## Star History Chart

[![Star History Chart](https://api.star-history.com/svg?repos=adolfousier/opencrabs&type=date&legend=top-left)](https://www.star-history.com/#adolfousier/opencrabs)

## ✨ Stay Tuned

---

**Built with Rust 🦀 by [Adolfo Usier](https://github.com/adolfousier)**
