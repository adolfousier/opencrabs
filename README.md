[![Rust](https://img.shields.io/badge/rust-%23000000.svg?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Rust Edition](https://img.shields.io/badge/rust-2024_edition-orange.svg)](https://www.rust-lang.org/)
[![Ratatui](https://img.shields.io/badge/ratatui-%23000000.svg?style=for-the-badge&logo=rust&logoColor=white)](https://ratatui.rs)
[![Docker](https://img.shields.io/badge/docker-%23000000.svg?style=for-the-badge&logo=docker&logoColor=white)](https://docker.com)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE.md)
[![CI](https://github.com/adolfousier/opencrabs/actions/workflows/ci.yml/badge.svg)](https://github.com/adolfousier/opencrabs/actions/workflows/ci.yml)
[![GitHub Stars](https://img.shields.io/github/stars/adolfousier/opencrabs?style=social)](https://github.com/adolfousier/opencrabs)

# 🦀 OpenCrabs

**Rust-based open-claw inspired orchestration layer for software development.**

> A terminal-native AI orchestration agent written in Rust with Ratatui. Inspired by [Open Claw](https://github.com/openclaw/openclaw).

```
    ___                    ___           _
   / _ \ _ __  ___ _ _    / __|_ _ __ _| |__  ___
  | (_) | '_ \/ -_) ' \  | (__| '_/ _` | '_ \(_-<
   \___/| .__/\___|_||_|  \___|_| \__,_|_.__//__/
        |_|

 🦀 Shell Yeah! AI Orchestration at Rust Speed.

```

**Author:** [Adolfo Usier](https://github.com/adolfousier)

⭐ Star us on [GitHub](https://github.com/adolfousier/opencrabs) if you like what you see!

---

## Table of Contents

- [Screenshots](#-screenshots)
- [Core Features](#-core-features)
- [Supported AI Providers](#-supported-ai-providers)
- [Quick Start](#-quick-start)
- [Onboarding Wizard](#-onboarding-wizard)
- [API Keys (keys.toml)](#-api-keys-keystoml)
- [Using Local LLMs](#-using-local-llms)
- [Configuration](#-configuration)
- [Tool System](#-tool-system)
- [Plan Mode](#-plan-mode)
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

[![Demo](src/screenshots/opencrabs-demo.gif)](https://github.com/user-attachments/assets/dfc44f70-52e1-44f7-8aef-b57faf453761)

![Splash](src/screenshots/splash.png)

![Onboarding](src/screenshots/onboard1.png)

![Provider Auth](src/screenshots/onboard2.png)

![Workspace](src/screenshots/onboard3.png)

![Home Base](src/screenshots/onboard4.png)

![Chat](src/screenshots/opencrabs-ui.png)

![Session Usage](src/screenshots/session-usage.png)

![Rebuild Complete](src/screenshots/rebuild-dialog.png)

---

## 🎯 Core Features

### AI & Providers
| Feature | Description |
|---------|-------------|
| **Multi-Provider** | Anthropic Claude, OpenAI, OpenRouter (400+ models), MiniMax, and any OpenAI-compatible API (Ollama, LM Studio, LocalAI). Model lists fetched live from provider APIs — new models available instantly |
| **Real-time Streaming** | Character-by-character response streaming with animated spinner showing model name and live text |
| **Local LLM Support** | Run with LM Studio, Ollama, or any OpenAI-compatible endpoint — 100% private, zero-cost |
| **Cost Tracking** | Per-message token count and cost displayed in header |
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

### Messaging Integrations
| Feature | Description |
|---------|-------------|
| **Telegram Bot** | Full-featured Telegram bot running alongside the TUI — shared session, photo/voice support, allowlisted user IDs |
| **WhatsApp** | Connect via QR code pairing at runtime ("connect my WhatsApp") or from onboarding wizard. Text + image support, shared session with TUI, phone allowlist, session persists across restarts |
| **Discord** | Full Discord bot — text + image + voice, allowlisted users/channels, shared session with TUI |
| **Slack** | Coming soon |

### Terminal UI
| Feature | Description |
|---------|-------------|
| **Cursor Navigation** | Full cursor movement: Left/Right arrows, Ctrl+Left/Right word jump, Home/End, Delete, Backspace at position |
| **Input History** | Persistent command history (`~/.opencrabs/history.txt`), loaded on startup, capped at 500 entries |
| **Inline Tool Approval** | Claude Code-style `❯ Yes / Always / No` selector with arrow key navigation |
| **Inline Plan Approval** | Interactive plan review selector (Approve / Reject / Request Changes / View Plan) |
| **Session Management** | Create, rename, delete sessions with persistent SQLite storage; token counts and context % per session |
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
| **Built-in Tools** | Read/write/edit files, bash, glob, grep, web search (EXA, Brave), plan mode, and more |
| **Plan Mode** | Structured task decomposition with dependency graphs, complexity ratings, and inline approval workflow |
| **Self-Sustaining** | Agent can modify its own source, build, test, and hot-restart via Unix `exec()` |
| **Natural Language Commands** | Tell OpenCrabs to create slash commands — it writes them to `commands.toml` autonomously via the `config_manager` tool |
| **Live Settings** | Agent can read/write `config.toml` at runtime; Settings TUI screen (press `S`) shows current config; approval policy persists across restarts |
| **Web Search** | EXA AI (neural, free via MCP) and Brave Search APIs — keys in `keys.toml` |
| **Debug Logging** | `--debug` flag enables file logging; `DEBUG_LOGS_LOCATION` env var for custom log directory |

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

**Models:** GPT-4 Turbo, GPT-4, GPT-3.5 Turbo

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

**Setup** in `config.toml`:

```toml
[providers.custom]
enabled = true
base_url = "http://localhost:1234/v1"  # or your endpoint
default_model = "qwen2.5-coder-7b-instruct"
# Optional: list your available models — shows up in /models and /onboard
# so you can switch between them without editing config
models = ["qwen2.5-coder-7b-instruct", "llama-3-8B", "mistral-7B-instruct"]
```

> **Local LLMs (Ollama, LM Studio):** No API key needed — just set `base_url` and `default_model`.
>
> **Remote APIs (Groq, Together, etc.):** Add the key in `keys.toml`:
> ```toml
> [providers.custom]
> api_key = "your-api-key"
> ```

> **Note:** `/chat/completions` is auto-appended to base URLs that don't include it.

**Want multiple custom providers?** Use named sections to define as many as you need and switch between them via `/models`:

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

The name after `custom.` is just a label you choose. The first one with `enabled = true` is used. Keys go in `keys.toml` using the same label (e.g. `[providers.custom.lm_studio]`).

**Provider priority:** MiniMax > OpenRouter > Anthropic > OpenAI > Custom. The first provider with `enabled = true` is used. Each provider has its own API key in `keys.toml` — no sharing or confusion.

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

First-time users are guided through an 8-step setup wizard that appears automatically after the splash screen.

### How It Triggers

- **Automatic:** When no `~/.opencrabs/config.toml` exists and no API keys are set in `keys.toml`
- **CLI:** `cargo run --bin opencrabs -- onboard` (or `opencrabs onboard` after install)
- **Chat flag:** `cargo run --bin opencrabs -- chat --onboard` to force the wizard before chat
- **Slash command:** Type `/onboard` in the chat to re-run it anytime

### The 8 Steps

| Step | Title | What It Does |
|------|-------|-------------|
| 1 | **Mode Selection** | QuickStart (sensible defaults) vs Advanced (full control) |
| 2 | **Model & Auth** | Pick provider (Anthropic, OpenAI, Gemini, OpenRouter, Minimax, Custom) → enter token/key → model list fetched live from API → select model. Auto-detects existing keys from `keys.toml` |
| 3 | **Workspace** | Set brain workspace path (default `~/.opencrabs/`) → seed template files (SOUL.md, IDENTITY.md, etc.) |
| 4 | **Gateway** | Configure HTTP API gateway: port, bind address, auth mode |
| 5 | **Channels** | Toggle messaging integrations (Telegram, Discord, WhatsApp, Signal, Google Chat, iMessage) |
| 6 | **Daemon** | Install background service (systemd on Linux, LaunchAgent on macOS) |
| 7 | **Health Check** | Verify API key, config, workspace — shows pass/fail summary |
| 8 | **Brain Personalization** | Tell the agent about yourself and how you want it to behave → AI generates personalized brain files (SOUL.md, IDENTITY.md, USER.md, etc.) |

**QuickStart mode** skips steps 4-6 with sensible defaults. **Advanced mode** lets you configure everything.

#### Brain Personalization (Step 8)

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

[providers.custom]
api_key = "your-key"                 # not required for local LLMs

# Messaging Channels
[channels.telegram]
token = "123456789:ABCdef..."

[channels.discord]
token = "your-discord-bot-token"

[channels.slack]
token = "xoxb-your-bot-token"
app_token = "xapp-your-app-token"

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
[providers.custom]
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
[providers.custom]
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

OpenCrabs uses two config files:
1. `~/.opencrabs/config.toml` — Provider settings, models, channels (safe to commit)
2. `~/.opencrabs/keys.toml` — API keys (chmod 600, NEVER commit!)

Search order for config.toml:
1. `~/.opencrabs/config.toml` (primary)
2. `~/.config/opencrabs/config.toml` (legacy fallback)
3. `./opencrabs.toml` (current directory override)

```bash
# Initialize config
cargo run -- init

# Copy the examples
cp config.toml.example ~/.opencrabs/config.toml
cp keys.toml.example ~/.opencrabs/keys.toml
chmod 600 ~/.opencrabs/keys.toml  # IMPORTANT: Secure the keys file!
```

### Example: Hybrid Setup (Local + Cloud)

Keep multiple providers configured — enable the one you want to use, disable the rest.
Switch anytime by toggling `enabled` or using `/onboard`.

In `config.toml`:
```toml
# Local LLM — currently active
[providers.custom]
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

## 📋 Plan Mode

Plan Mode breaks complex tasks into structured, reviewable, executable plans.

### Workflow

1. **Request:** Ask the AI to create a plan using the plan tool
2. **AI creates:** Structured tasks with dependencies, complexity estimates, and types
3. **Review:** Press `Ctrl+P` to view the plan in a visual TUI panel
4. **Decide:** An inline selector appears with arrow key navigation:
   - **Approve** — Execute the plan
   - **Reject** — Discard the plan
   - **Request Changes** — Returns to chat with context for revisions
   - **View Plan** — Open the full plan panel (`Ctrl+P`)

### Plan States

Plans progress through: **Draft** → **PendingApproval** → **Approved** → **InProgress** → **Completed**

Tasks have 10 types: Research, Edit, Create, Delete, Test, Refactor, Documentation, Configuration, Build, Other

Each task tracks: status (Pending/InProgress/Completed/Skipped/Failed/Blocked), dependencies, complexity (1-5), and timestamps.

### Example

```
You: Use the plan tool to create a plan for implementing JWT authentication.
     Add tasks for: adding dependencies, token generation, validation
     middleware, updating login endpoint, and writing tests.
     Call operation=finalize when done.

OpenCrabs: [Creates plan with 5 tasks, dependencies, complexity ratings]
         ✓ Plan finalized! Press Ctrl+P to review.
```

```
┌─────────────────────────────────────────────────────────────┐
│ 📋 Plan: JWT Authentication                                 │
│ Status: Pending Approval • Tasks: 5 • Complexity: Medium    │
├─────────────────────────────────────────────────────────────┤
│ 1. [⏹] Add jsonwebtoken dependency (⭐⭐)                   │
│ 2. [⏹] Implement token generation (⭐⭐⭐⭐) → depends on #1 │
│ 3. [⏹] Build validation middleware (⭐⭐⭐⭐⭐) → depends on #2│
│ 4. [⏹] Update login endpoint (⭐⭐⭐) → depends on #2       │
│ 5. [⏹] Write integration tests (⭐⭐⭐) → depends on #3, #4 │
├─────────────────────────────────────────────────────────────┤
│ [Ctrl+A] Approve  [Ctrl+R] Reject  [Ctrl+I] Changes  [Esc]│
└─────────────────────────────────────────────────────────────┘
```

**Tip for local LLMs:** Be explicit about tool usage — say "use the plan tool with operation=create" rather than "create a plan".

See [Plan Mode User Guide](src/docs/PLAN_MODE_USER_GUIDE.md) for full documentation.

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
| `/usage` | Token/cost stats |
| `/onboard` | Run setup wizard |
| `/sessions` | Open session manager |
| `/approve` | Tool approval policy selector (approve-only / session / yolo) |
| `/compact` | Compact context (summarize + trim for long sessions) |
| `/rebuild` | Build from source & hot-restart — auto-clones repo if no source tree found |
| `/cd` | Change working directory (directory picker) |
| `/settings` or `S` | Open Settings screen (provider, approval, commands, paths) |

### Sessions Mode

| Shortcut | Action |
|----------|--------|
| `↑` / `↓` | Navigate sessions |
| `Enter` | Load selected session |
| `R` | Rename session |
| `D` | Delete session |
| `Esc` | Back to chat |

### Tool Approval (Inline)

When the AI requests a tool that needs permission, an inline approval prompt appears in chat:

| Shortcut | Action |
|----------|--------|
| `↑` / `↓` | Navigate approval options |
| `Enter` | Confirm selected option |
| `D` / `Esc` | Deny the tool request |
| `V` | Toggle parameter details |

**Approval options:**

| Option | Effect |
|--------|--------|
| **Allow once** | Approve this single tool call |
| **Allow all for this task** | Auto-approve all tools this session (resets on session switch) |
| **Allow all moving forward** | Auto-approve all tools permanently (app lifetime) |

Use `/approve` to change your approval policy at any time (persisted to `config.toml`):

| Policy | Description |
|--------|-------------|
| **Approve-only** | Always ask before executing tools (default) |
| **Allow all (session)** | Auto-approve all tools for the current session |
| **Yolo mode** | Execute everything without approval until reset |

### Plan Approval (Inline)

When a plan is submitted for approval, an inline selector appears in chat:

| Shortcut | Action |
|----------|--------|
| `↑` / `↓` | Navigate approval options (Approve / Reject / Request Changes / View Plan) |
| `Enter` | Confirm selected option |
| `Ctrl+P` | View full plan panel |

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
