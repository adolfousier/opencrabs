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
