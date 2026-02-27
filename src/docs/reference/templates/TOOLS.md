# TOOLS.md - Local Notes

Skills define *how* tools work. This file is for *your* specifics — the stuff that's unique to your setup.

## Custom Skills & Plugins

Your custom implementations live here in the workspace, **never in the repo**:

```
~/.opencrabs/
├── skills/       # Custom skills you create or install
├── plugins/      # Custom plugins and extensions
├── scripts/      # Custom automation scripts
```

This ensures `git pull` on the repo never overwrites your work. See AGENTS.md for the full workspace layout.

### Rust-First Policy
When building custom tools or adding dependencies, **always prioritize Rust-based crates** over wrappers, FFI bindings, or other-language alternatives. Native Rust = lean, safe, fast.

## Tool Parameter Reference

Use these **exact parameter names** when calling tools:

| Tool | Required Params | Optional Params |
|------|----------------|-----------------|
| `ls` | `path` | `recursive` |
| `glob` | `pattern` | `path` |
| `grep` | `pattern` | `path`, `regex`, `case_insensitive`, `file_pattern`, `limit`, `context` |
| `read_file` | `path` | `line_range` |
| `edit_file` | `path`, `operation` | `old_text`, `new_text`, `line` |
| `write_file` | `path`, `content` | — |
| `bash` | `command` | `timeout` |
| `execute_code` | `language`, `code` | — |
| `web_search` | `query` | `n` |
| `http_request` | `method`, `url` | `headers`, `body` |
| `session_search` | `operation` | `query`, `n`, `session_id` |
| `task_manager` | `operation` | `title`, `description`, `task_id`, `status` |
| `plan` | `operation` | `title`, `description`, `task` |
| `session_context` | `operation` | `key`, `value` |

> **Note:** `grep` and `glob` use `pattern` (not `query`). `bash` uses `command` (not `cmd`). File tools use `path` (not `file` or `file_path`).

## What Goes Here

Things like:
- SSH hosts and aliases
- API account details (not secrets — those go in `.env`)
- Camera names and locations
- Preferred voices for TTS
- Speaker/room names
- Device nicknames
- Server IPs and access methods
- Docker container inventories
- Nginx site mappings
- Custom skill/plugin notes and configuration
- Anything environment-specific

## Path Tips
- **Workspace:** `~/.opencrabs/`
- **Path tip:** Always run `echo $HOME` or `ls ~/.opencrabs/` first to confirm the resolved path before file operations.
- OpenCrabs tools operate on the directory you launched from. Use `/cd` to change at runtime, or use `config_manager` with `set_working_directory` to change via natural language.
- **Env files:** `~/.opencrabs/.env` — chmod 600 (owner-only read)

## LLM Provider Configuration

OpenCrabs supports multiple LLM providers simultaneously. Each session can use a different provider + model.

### Adding a New Custom Provider

When a user asks to add a new AI provider (e.g. "add Groq", "connect to my OpenRouter", "add this new API"), offer them two paths:

**Path 1 — You handle it (preferred):**
> "Paste your provider details (base URL, API key, model name) and I'll add it to your config right now."

Then write these two blocks:

**`~/.opencrabs/config.toml`** — add a named section under `[providers.custom]`:
```toml
[providers.custom.groq]          # name can be anything: groq, nvidia, together, etc.
enabled = true                   # set to true to make it active; set others to false
base_url = "https://api.groq.com/openai/v1/chat/completions"
default_model = "llama-3.3-70b-versatile"
models = ["llama-3.3-70b-versatile", "mixtral-8x7b-32768"]
```

**`~/.opencrabs/keys.toml`** — add a matching section with the same label:
```toml
[providers.custom.groq]
api_key = "gsk_..."
```

**Critical rules:**
- The label after `custom.` MUST match exactly in both files (e.g. `custom.groq` ↔ `custom.groq`)
- Only one provider should have `enabled = true` at a time (the active one)
- For local LLMs (Ollama, LM Studio) — `api_key = ""` (empty is fine)
- Use `config_manager` tool with `read_config` / `write_config` to inspect and update these files safely

**Path 2 — User edits manually:**
> "Add this to `~/.opencrabs/config.toml` and the matching key to `~/.opencrabs/keys.toml`"
> Then show them the TOML blocks above filled in with their details.

### Multiple Providers Coexisting

All named providers persist — switching via `/models` just toggles `enabled`:

```toml
[providers.custom.lm_studio]
enabled = false          # currently inactive
base_url = "http://localhost:1234/v1/chat/completions"
default_model = "qwen3-coder"

[providers.custom.groq]
enabled = true           # currently active
base_url = "https://api.groq.com/openai/v1/chat/completions"
default_model = "llama-3.3-70b-versatile"

[providers.custom.nvidia]
enabled = false
base_url = "https://integrate.api.nvidia.com/v1/chat/completions"
default_model = "moonshotai/kimi-k2.5"
```

User can switch between them via `/models` in the TUI — no need to edit files manually each time.

### Per-Session Provider

Each session remembers its own provider + model. When the user switches sessions, the provider auto-restores. No need to `/models` every time.

To run two providers in parallel: open session A → send message → press `Ctrl+N` for new session B → switch provider via `/models` → send another message. Both process simultaneously.

### Provider Priority (new sessions inherit first enabled)

`providers.custom.*` → `providers.minimax` → `providers.openrouter` → `providers.anthropic` → `providers.openai`

The first provider with `enabled = true` (in config file order) is used for new sessions.

## Integrations

### Channel Connections
OpenCrabs can connect to messaging platforms. Configure in `~/.opencrabs/config.toml`:

- **Telegram** — Create a bot via @BotFather, add token to config `[channels.telegram]`
- **Discord** — Create a bot at discord.com/developers (enable MESSAGE CONTENT intent), add token to config `[channels.discord]`
- **WhatsApp** — Link via QR code pairing, configure `[channels.whatsapp]` with allowed phone numbers
- **Slack** — Create an app at api.slack.com/apps (enable Socket Mode), add tokens to config `[channels.slack]`

API keys go in `~/.opencrabs/keys.toml` (chmod 600). Channel settings go in `config.toml`.

### WhisperCrabs — Voice-to-Text (D-Bus)
[WhisperCrabs](https://github.com/adolfousier/whispercrabs) is a floating voice-to-text tool. Fully controllable via D-Bus.

**What it does:** Click to record → click to stop → transcribes → text copied to clipboard. Sound plays when ready.

**D-Bus control (full access):**
- Start/stop recording
- Switch between local (whisper.cpp) and API transcription
- Set API keys and endpoint URLs
- View transcription history
- Trigger settings dialog

**Setup:** Download binary, launch, configure via right-click menu or D-Bus commands.

**As an OpenCrabs tool:** When user asks to transcribe voice or set up voice input, use D-Bus to control WhisperCrabs — check if running, start recording, configure provider, etc.

### SocialCrabs — Social Media Automation
[SocialCrabs](https://github.com/adolfousier/socialcrabs) is a web-based social media automation tool with human-like behavior simulation (Playwright).

**Supported platforms:** Twitter/X, Instagram, LinkedIn

**Interfaces:**
- **CLI** — `node dist/cli.js <platform> <command>`
- **REST API** — port 3847
- **WebSocket** — port 3848
- **SDK** — TypeScript/JavaScript programmatic access

**Setup:**
1. Clone the repo and install dependencies
2. Add platform cookies/credentials to `.env` (see SocialCrabs README)
3. Run `node dist/cli.js session login <platform>` to authenticate

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
node dist/cli.js linkedin search <query>
node dist/cli.js linkedin engage --query=<query>   # Full engagement session
```

**Key features:** Human-like behavior (randomized delays, natural typing), session persistence, built-in rate limiting, anti-detection, research-first workflow.

**As an OpenCrabs tool:** When user asks to post, engage, or monitor social media, use SocialCrabs CLI commands. Read operations (search, mentions, timeline) are safe. Write operations (tweet, like, follow, comment) **require explicit user approval**.

### Agent-to-Agent (A2A) Gateway
OpenCrabs exposes an A2A Protocol HTTP gateway for peer-to-peer agent communication.

**What it does:** Other A2A-compatible agents can send tasks via JSON-RPC 2.0. OpenCrabs processes them using its full tool suite and returns results.

**Endpoints:**
- `GET /.well-known/agent.json` — Agent Card discovery (skills, capabilities)
- `POST /a2a/v1` — JSON-RPC 2.0 (`message/send`, `tasks/get`, `tasks/cancel`)
- `GET /a2a/health` — Health check

**Setup:** Enable in `~/.opencrabs/config.toml`:
```toml
[a2a]
enabled = true
bind = "127.0.0.1"
port = 18790
```

**Bee Colony Debate:** Multi-agent structured debate with knowledge-enriched context from QMD memory search. Configurable rounds, confidence-weighted consensus based on ReConcile (ACL 2024).

## Why Separate?

Skills are shared. Your setup is yours. Keeping them apart means you can update skills without losing your notes, and share skills without leaking your infrastructure.

---

Add whatever helps you do your job. This is your cheat sheet.
