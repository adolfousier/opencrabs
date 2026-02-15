[![Rust Edition](https://img.shields.io/badge/rust-2024_edition-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-FSL--1.1--MIT-blue.svg)](LICENSE.md)
[![CI](https://github.com/adolfousier/opencrabs/actions/workflows/ci.yml/badge.svg)](https://github.com/adolfousier/opencrabs/actions/workflows/ci.yml)
[![GitHub Stars](https://img.shields.io/github/stars/adolfousier/opencrabs?style=social)](https://github.com/adolfousier/opencrabs)

# OpenCrabs

**The AI buddy that gets things done. Rust-based open-claw inspired orchestration layer for software development.**

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

---

## Table of Contents

- [Screenshots](#-screenshots)
- [Core Features](#-core-features)
- [Supported AI Providers](#-supported-ai-providers)
- [Quick Start](#-quick-start)
- [Onboarding Wizard](#-onboarding-wizard)
- [Authentication Methods](#-authentication-methods)
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
- [Disclaimers](#-disclaimers)
- [Contributing](#-contributing)
- [License](#-license)
- [Acknowledgments](#-acknowledgments)

---

## 📸 Screenshots

![Splash](src/screenshots/splash.png)

![Onboarding](src/screenshots/onboard1.png)

![Provider Auth](src/screenshots/onboard2.png)

![Workspace](src/screenshots/onboard3.png)

![Chat](src/screenshots/opencrabs-ui.png)

---

## 🎯 Core Features

| Feature | Description |
|---------|-------------|
| **Dynamic Brain System** | System brain assembled from workspace MD files — personality, identity, memory, all editable live |
| **Self-Sustaining** | Agent can modify its own source, build, test, and hot-restart via Unix `exec()` |
| **Natural Language Commands** | Tell OpenCrabs to create slash commands — it writes them to `commands.json` autonomously |
| **Built-in Tools** | Read/write files, execute commands, grep, glob, web search, and more |
| **Session Management** | Create, rename, delete sessions with persistent SQLite storage |
| **Syntax Highlighting** | 100+ languages with line numbers via syntect |
| **Local LLM Support** | Run with LM Studio, Ollama, or any OpenAI-compatible endpoint — 100% private |
| **Multi-Provider** | Anthropic Claude (with OAuth), OpenAI, Qwen, and OpenAI-compatible APIs |
| **Session Context** | Persistent conversation memory with SQLite storage |
| **Streaming** | Real-time character-by-character response generation |
| **Cost Tracking** | Per-message token count and cost displayed in header |
| **Context Awareness** | Live context usage indicator with color-coded percentage; auto-compaction when nearing limits |
| **Web Search** | Real-time internet search via EXA AI (neural) or Brave Search APIs — activate with an API key |
| **Plan Mode** | Structured task decomposition with review workflow |
| **Multi-line Input** | Paste entire functions, send with Ctrl+Enter |
| **Markdown Rendering** | Rich text formatting with code blocks and headings |
| **Debug Logging** | Conditional file logging with `-d` flag, clean workspace by default |

---

## 🌐 Supported AI Providers

### Anthropic Claude

**Models:** `claude-opus-4-6`, `claude-sonnet-4-5-20250929`, `claude-haiku-4-5-20251001`, plus legacy Claude 3.x models

**Authentication:**

| Method | Env Variable | Header |
|--------|-------------|--------|
| **OAuth / Claude Max** (recommended) | `ANTHROPIC_MAX_SETUP_TOKEN` | `Authorization: Bearer` + `anthropic-beta: oauth-2025-04-20` |
| Standard API Key | `ANTHROPIC_API_KEY` | `x-api-key` |

OAuth tokens are auto-detected by the `sk-ant-oat` prefix. When `ANTHROPIC_MAX_SETUP_TOKEN` is set, it takes priority over `ANTHROPIC_API_KEY`.

Set a custom model with `ANTHROPIC_MAX_MODEL` (e.g., `claude-opus-4-6`).

**Features:** Streaming, tools, cost tracking, automatic retry with backoff

### OpenAI

**Models:** GPT-4 Turbo, GPT-4, GPT-3.5 Turbo

**Setup:** `export OPENAI_API_KEY="sk-YOUR_KEY"`

Compatible with any OpenAI-compatible API endpoint via `OPENAI_BASE_URL`.

### Qwen (via OpenAI-compatible)

**Setup:** Configure via `QWEN_API_KEY` and `QWEN_BASE_URL`.

### OpenAI-Compatible Local / Cloud APIs

| Provider | Status | Setup |
|----------|--------|-------|
| **LM Studio** | Tested | `OPENAI_BASE_URL="http://localhost:1234/v1"` |
| **Ollama** | Compatible | `OPENAI_BASE_URL="http://localhost:11434/v1"` |
| **LocalAI** | Compatible | `OPENAI_BASE_URL="http://localhost:8080/v1"` |
| OpenRouter | Compatible | `OPENAI_BASE_URL="https://openrouter.ai/api/v1"` |
| Groq | Compatible | `OPENAI_BASE_URL="https://api.groq.com/openai/v1"` |

**Provider priority:** Qwen > OpenAI > Anthropic (fallback). The first provider with a configured API key is used.

---

## 🚀 Quick Start

### Prerequisites

- **Rust (2024 edition)** — [Install Rust](https://rustup.rs/)
- **An API key** from at least one supported provider
- **SQLite** (bundled via sqlx)
- **Linux:** `build-essential`, `pkg-config`, `libssl-dev`, `libchafa-dev`

### Install & Run

```bash
# Clone
git clone https://github.com/adolfousier/opencrabs.git
cd opencrabs

# Set up credentials (pick one)
cp .env.example .env
# Edit .env with your API key(s)

# Build & run (development)
cargo run --bin opencrabs

# Or build release and run directly
cargo build --release
./target/release/opencrabs
```

OpenCrabs auto-loads `.env` via `dotenvy` at startup — no need to manually export variables.

> **First run?** The onboarding wizard will guide you through provider setup, workspace, and more. See [Onboarding Wizard](#-onboarding-wizard).

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

# Keyring (secure OS credential storage)
cargo run --bin opencrabs -- keyring set anthropic YOUR_KEY
cargo run --bin opencrabs -- keyring get anthropic
cargo run --bin opencrabs -- keyring list

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

**Output formats** for non-interactive mode: `text` (default), `json`, `markdown`

---

## 🧙 Onboarding Wizard

First-time users are guided through an 8-step setup wizard that appears automatically after the splash screen.

### How It Triggers

- **Automatic:** When no `~/.config/opencrabs/config.toml` exists and no API keys are set in env/keyring
- **CLI:** `cargo run --bin opencrabs -- onboard` (or `opencrabs onboard` after install)
- **Chat flag:** `cargo run --bin opencrabs -- chat --onboard` to force the wizard before chat
- **Slash command:** Type `/onboard` in the chat to re-run it anytime

### The 8 Steps

| Step | Title | What It Does |
|------|-------|-------------|
| 1 | **Mode Selection** | QuickStart (sensible defaults) vs Advanced (full control) |
| 2 | **Model & Auth** | Pick provider (Anthropic, OpenAI, Gemini, Qwen, Custom) → enter token/key → select model. Auto-detects existing keys from env/keyring |
| 3 | **Workspace** | Set brain workspace path (default `~/opencrabs/brain/workspace/`) → seed template files (SOUL.md, IDENTITY.md, etc.) |
| 4 | **Gateway** | Configure HTTP API gateway: port, bind address, auth mode |
| 5 | **Channels** | Toggle messaging integrations (Telegram, Discord, WhatsApp, Signal, Google Chat, iMessage) |
| 6 | **Daemon** | Install background service (systemd on Linux, LaunchAgent on macOS) |
| 7 | **Health Check** | Verify API key, config, workspace — shows pass/fail summary |
| 8 | **Brain Personalization** | Tell the agent about yourself and your preferred agent vibe → AI generates personalized brain files (SOUL.md, IDENTITY.md, USER.md, etc.) |

**QuickStart mode** skips steps 4-6 with sensible defaults. **Advanced mode** lets you configure everything.

### Wizard Navigation

| Key | Action |
|-----|--------|
| `Tab` / `Shift+Tab` | Navigate between fields |
| `Up` / `Down` | Scroll through lists |
| `Enter` | Confirm / next step |
| `Space` | Toggle checkboxes |
| `Esc` | Go back one step |

---

## 🔑 Authentication Methods

### Option A: OAuth / Claude Max (Recommended for Claude)

```bash
# In .env file:
ANTHROPIC_MAX_SETUP_TOKEN=sk-ant-oat01-YOUR_OAUTH_TOKEN
ANTHROPIC_MAX_MODEL=claude-opus-4-6
```

The `sk-ant-oat` prefix is auto-detected. OpenCrabs will use `Authorization: Bearer` with the `anthropic-beta: oauth-2025-04-20` header.

### Option B: Standard API Key

```bash
# In .env or exported:
ANTHROPIC_API_KEY=sk-ant-api03-YOUR_KEY
OPENAI_API_KEY=sk-YOUR_KEY
```

### Option C: OS Keyring (Secure Storage)

```bash
cargo run -- keyring set anthropic YOUR_API_KEY
# Encrypted by OS (Windows Credential Manager / macOS Keychain / Linux Secret Service)
# Automatically loaded on startup, no plaintext files
```

**Priority:** Keyring > `ANTHROPIC_MAX_SETUP_TOKEN` > `ANTHROPIC_API_KEY` > config file

---

## 🏠 Using Local LLMs

OpenCrabs works with any OpenAI-compatible local inference server for **100% private, zero-cost** operation.

### LM Studio (Recommended)

1. Download and install [LM Studio](https://lmstudio.ai/)
2. Download a model (e.g., `qwen2.5-coder-7b-instruct`, `Mistral-7B-Instruct`, `Llama-3-8B`)
3. Start the local server (default port 1234)
4. Configure OpenCrabs:

```bash
# .env or environment
OPENAI_API_KEY="lm-studio"
OPENAI_BASE_URL="http://localhost:1234/v1"
```

Or via `opencrabs.toml`:

```toml
[providers.openai]
enabled = true
base_url = "http://localhost:1234/v1/chat/completions"
default_model = "qwen2.5-coder-7b-instruct"   # Must EXACTLY match LM Studio model name
```

> **Critical:** The `default_model` value must exactly match the model name shown in LM Studio's Local Server tab (case-sensitive).

### Ollama

```bash
ollama pull mistral
# Configure:
OPENAI_BASE_URL="http://localhost:11434/v1"
OPENAI_API_KEY="ollama"
```

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

### Configuration File (`opencrabs.toml`)

OpenCrabs searches for config in this order:
1. `./opencrabs.toml` (current directory)
2. `~/.config/opencrabs/opencrabs.toml` (Linux/macOS) or `%APPDATA%\opencrabs\opencrabs.toml` (Windows)
3. `~/opencrabs.toml`

Environment variables override config file settings. `.env` files are auto-loaded.

```bash
# Initialize config
cargo run -- init

# Copy the example
cp config.toml.example ~/.config/opencrabs/opencrabs.toml
```

### Example: Hybrid Setup (Local + Cloud)

```toml
[database]
path = "~/.opencrabs/opencrabs.db"

# Local LLM for daily development
[providers.openai]
enabled = true
base_url = "http://localhost:1234/v1/chat/completions"
default_model = "qwen2.5-coder-7b-instruct"

# Cloud API for complex tasks
[providers.anthropic]
enabled = true
default_model = "claude-opus-4-6"
# API key via env var or keyring
```

### Environment Variables

| Variable | Provider | Description |
|----------|----------|-------------|
| `ANTHROPIC_MAX_SETUP_TOKEN` | Anthropic (OAuth) | OAuth Bearer token (takes priority) |
| `ANTHROPIC_MAX_MODEL` | Anthropic | Custom default model |
| `ANTHROPIC_API_KEY` | Anthropic | Standard API key |
| `OPENAI_API_KEY` | OpenAI / Compatible | API key |
| `OPENAI_BASE_URL` | OpenAI / Compatible | Custom endpoint URL |
| `QWEN_API_KEY` | Qwen | API key |
| `QWEN_BASE_URL` | Qwen | Custom endpoint URL |
| `EXA_API_KEY` | EXA AI Search | Optional — enables direct API mode (higher rate limits) |
| `BRAVE_API_KEY` | Brave Search | Web search (free $5/mo credits at brave.com/search/api) |
| `EXA_API_KEY` | EXA AI Search | Neural web search (free credits on signup at exa.ai) |
| `BRAVE_API_KEY` | Brave Search | Web search (free $5/mo credits at brave.com/search/api) |

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
| `exa_search` | Neural web search via EXA AI (free via MCP, no API key needed) |
| `brave_search` | Web search via Brave Search (set `BRAVE_API_KEY` — free $5/mo credits) |
| `execute_code` | Run code in various languages |
| `notebook_edit` | Edit Jupyter notebooks |
| `parse_document` | Extract text from PDF, DOCX, HTML |
| `task_manager` | Manage agent tasks |
| `http_request` | Make HTTP requests |
| `session_context` | Access session information |
| `plan` | Create structured execution plans |

---

## 📋 Plan Mode

Plan Mode breaks complex tasks into structured, reviewable, executable plans.

### Workflow

1. **Request:** Ask the AI to create a plan using the plan tool
2. **AI creates:** Structured tasks with dependencies, complexity estimates, and types
3. **Review:** Press `Ctrl+P` to view the plan in a visual TUI panel
4. **Decide:**
   - `Ctrl+A` — Approve and execute
   - `Ctrl+R` — Reject the plan
   - `Ctrl+I` — Request changes (returns to chat with context)
   - `Esc` — Go back without changes

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
| `/help` | Open help dialog |
| `/model` | Show current model |
| `/models` | Switch model |
| `/usage` | Token/cost stats |
| `/onboard` | Run setup wizard |
| `/sessions` | Open session manager |
| `/approve` | Tool approval policy selector (approve-only / session / yolo) |
| `/compact` | Compact context (summarize + trim for long sessions) |
| `/compact` | Compact context (summarize + trim for long sessions) |

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

Use `/approve` to change your approval policy at any time:

| Policy | Description |
|--------|-------------|
| **Approve-only** | Always ask before executing tools (default) |
| **Allow all (session)** | Auto-approve all tools for the current session |
| **Yolo mode** | Execute everything without approval until reset |

### Plan Mode

| Shortcut | Action |
|----------|--------|
| `Ctrl+P` | View current plan |
| `Ctrl+A` | Approve plan |
| `Ctrl+R` | Reject plan |
| `Ctrl+I` | Request changes |
| `↑` / `↓` | Scroll through plan |

---

## 🔍 Debug and Logging

OpenCrabs uses a **conditional logging system** — no log files by default.

```bash
# Enable debug mode (creates log files)
opencrabs -d
cargo run -- -d

# Logs stored in .opencrabs/logs/ (auto-gitignored)
# Daily rolling rotation, auto-cleanup after 7 days

# Management
opencrabs logs status    # Check logging status
opencrabs logs view      # View recent entries
opencrabs logs clean     # Clean old logs
opencrabs logs clean -d 3  # Clean logs older than 3 days
```

**When debug mode is enabled:**
- Log files created in `.opencrabs/logs/`
- DEBUG level with thread IDs, file names, line numbers
- Daily rolling rotation

**When disabled (default):**
- No log files created
- Only warnings and errors to stderr
- Clean workspace

---

## 🧠 Brain System (v0.1.1)

OpenCrabs's brain is **dynamic and self-sustaining**. Instead of a hardcoded system prompt, the agent assembles its personality, knowledge, and behavior from workspace files that can be edited between turns.

### Brain Workspace

The brain reads markdown files from `~/opencrabs/brain/workspace/` (or `OPENCRABS_BRAIN_PATH` env var):

| File | Purpose |
|------|---------|
| `SOUL.md` | Personality, tone, hard behavioral rules |
| `IDENTITY.md` | Agent name, vibe, style |
| `USER.md` | Who the human is, how to work with them |
| `AGENTS.md` | Workspace rules, memory system, safety policies |
| `TOOLS.md` | Environment-specific notes (SSH hosts, API accounts) |
| `MEMORY.md` | Long-term context, troubleshooting notes, lessons learned |

Files are re-read **every turn** — edit them between messages and the agent immediately reflects the changes. Missing files are silently skipped; a hardcoded brain preamble is always present.

### User-Defined Slash Commands

Tell OpenCrabs in natural language: *"Create a /deploy command that runs deploy.sh"* — and it writes the command to `~/opencrabs/brain/commands.json`:

```json
[
  {
    "name": "/deploy",
    "description": "Deploy to staging server",
    "action": "prompt",
    "prompt": "Run the deployment script at ./scripts/deploy.sh for the staging environment."
  }
]
```

Commands appear in autocomplete alongside built-in commands. After each agent response, `commands.json` is automatically reloaded — no restart needed.

### Self-Sustaining Architecture

OpenCrabs can modify its own source code, build, test, and hot-restart itself via the `/rebuild` command:

```
/rebuild          # Triggers build → test → restart
```

**How it works:**

1. The agent edits source files using its built-in tools (read, write, edit, bash)
2. `SelfUpdater::build()` runs `cargo build --release` asynchronously
3. `SelfUpdater::test()` runs `cargo test` to verify correctness
4. On success, the TUI shows a **RestartPending** confirmation dialog
5. `SelfUpdater::restart(session_id)` replaces the process via Unix `exec()`
6. The new binary starts with `opencrabs chat --session <uuid>` — resuming the same conversation from SQLite

**Key details:**

- The running binary is in memory — source changes on disk don't affect it until restart
- If the build fails, the agent stays running and can read compiler errors to fix them
- Session persistence via SQLite means no conversation context is lost across restarts
- Brain files (`SOUL.md`, `MEMORY.md`, etc.) are re-read every turn, so edits take effect immediately without rebuild
- User-defined slash commands (`commands.json`) also auto-reload after each agent response
- Hot restart is Unix-only (`exec()` syscall); on Windows the build/test steps work but restart requires manual relaunch

**Module:** `src/brain/self_update.rs` — `SelfUpdater` struct with `auto_detect()`, `build()`, `test()`, `restart()`

---

## 🏗️ Architecture

```
Presentation Layer
    ↓
CLI (Clap) + TUI (Ratatui + Crossterm)
    ↓
Brain Layer (Dynamic system brain, user commands, self-update)
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
| Error Handling | anyhow + thiserror + color-eyre |
| Logging | tracing + tracing-subscriber |
| Security | zeroize + keyring |

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
│   ├── brain/            # Dynamic brain system (v0.1.1)
│   │   ├── mod.rs        # Module root
│   │   ├── prompt_builder.rs  # BrainLoader — assembles system brain from workspace files
│   │   ├── commands.rs   # CommandLoader — user-defined slash commands (JSON)
│   │   └── self_update.rs # SelfUpdater — build, test, hot-restart via exec()
│   ├── cli/              # Command-line interface (Clap)
│   ├── config/           # Configuration (TOML + env + keyring)
│   │   └── crabrace.rs   # Provider registry integration
│   ├── db/               # Database layer (SQLx + SQLite)
│   ├── services/         # Business logic (Session, Message, File, Plan)
│   ├── llm/              # LLM integration
│   │   ├── agent/        # Agent service + context management
│   │   ├── provider/     # Provider implementations (Anthropic, OpenAI, Qwen)
│   │   ├── tools/        # Tool system (read, write, bash, glob, grep, etc.)
│   │   └── prompt/       # Prompt engineering
│   ├── tui/              # Terminal UI (Ratatui)
│   │   ├── onboarding.rs     # 7-step onboarding wizard (state + logic)
│   │   ├── onboarding_render.rs  # Wizard rendering
│   │   ├── splash.rs     # Splash screen
│   │   ├── app.rs        # App state + event handling
│   │   ├── render.rs     # Main render dispatch
│   │   └── runner.rs     # TUI event loop
│   ├── lsp/              # LSP integration
│   ├── events/           # Event handling
│   ├── message/          # Message types
│   ├── sync/             # Synchronization utilities
│   ├── macros/           # Rust macros
│   ├── utils/            # Utilities (retry, etc.)
│   ├── migrations/       # SQLite migrations
│   ├── tests/            # Integration tests
│   ├── benches/          # Criterion benchmarks
│   └── docs/             # Documentation + screenshots
├── Cargo.toml
├── config.toml.example
├── .env.example
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
| `openai` | Enable async-openai integration |
| `aws-bedrock` | Enable AWS Bedrock runtime |
| `all-llm` | Enable all LLM provider features |
| `profiling` | Enable pprof flamegraph profiling (Unix only) |

### Performance

| Metric | Value |
|--------|-------|
| Startup time | < 50ms |
| Memory (idle) | ~15 MB |
| Memory (100 messages) | ~20 MB |
| Database ops | < 10ms (session), < 5ms (message) |

---

## 🐛 Platform Notes

### Linux

```bash
sudo apt-get install build-essential pkg-config libssl-dev libchafa-dev
```

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

**FSL-1.1-MIT License**

- **Functional Source License (FSL) 1.1** — First 2 years
- **MIT License** — After 2 years from release

See [LICENSE.md](LICENSE.md) for details.

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

**Built with Rust 🦀 by [Adolfo Usier](https://github.com/adolfousier)**
