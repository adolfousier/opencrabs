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
| `trello_connect` | `api_key`, `api_token`, `boards` | `allowed_users` |
| `trello_send` | `action` | `board_id`, `list_name`, `card_id`, `title`, `description`, `text`, `position`, `pattern`, `member_id`, `label_id`, `due_date`, `due_complete`, `checklist_id`, `item_id`, `complete`, `query`, `read_filter`, `limit` |
| `discord_connect` | `token`, `allowed_users` | `channel_id` |
| `discord_send` | `action` | `message`, `channel_id`, `message_id`, `emoji`, `embed_title`, `embed_description`, `embed_color`, `thread_name`, `user_id`, `role_id`, `limit` |
| `telegram_send` | `action` | `message`, `chat_id`, `message_id`, `from_chat_id`, `photo_url`, `document_url`, `latitude`, `longitude`, `poll_question`, `poll_options`, `buttons`, `user_id`, `emoji` |
| `slack_send` | `action` | `message`, `channel_id`, `thread_ts`, `message_ts`, `emoji`, `user_id`, `topic`, `blocks`, `limit` |

> **Note:** `grep` and `glob` use `pattern` (not `query`). `bash` uses `command` (not `cmd`). File tools use `path` (not `file` or `file_path`).
> **Trello:** `trello_connect` `boards` is an array of board names or IDs. Always use `trello_send` instead of `http_request` for Trello — credentials are handled securely without exposing them in URLs.
> **`trello_send` actions (21):** `add_comment`, `create_card`, `move_card`, `find_cards`, `list_boards`, `get_card`, `get_card_comments`, `update_card`, `archive_card`, `add_member_to_card`, `remove_member_from_card`, `add_label_to_card`, `remove_label_from_card`, `add_checklist`, `add_checklist_item`, `complete_checklist_item`, `list_lists`, `get_board_members`, `search`, `get_notifications`, `mark_notifications_read`
> **Discord:** `discord_connect` `allowed_users` is an array of numeric Discord user IDs. Always use `discord_send` instead of `http_request` for Discord — credentials are handled securely.
> **`discord_send` actions (16):** `send`, `reply`, `react`, `unreact`, `edit`, `delete`, `pin`, `unpin`, `create_thread`, `send_embed`, `get_messages`, `list_channels`, `add_role`, `remove_role`, `kick`, `ban`
> **Guild-required actions:** `list_channels`, `add_role`, `remove_role`, `kick`, `ban` — these need the bot to have received at least one guild message first so the guild_id is available.
> **Telegram:** Always use `telegram_send` instead of `http_request` for Telegram — credentials handled securely.
> **`telegram_send` actions (16):** `send`, `reply`, `edit`, `delete`, `pin`, `unpin`, `forward`, `send_photo`, `send_document`, `send_location`, `send_poll`, `send_buttons`, `get_chat`, `ban_user`, `unban_user`, `set_reaction`
> **Slack:** Always use `slack_send` instead of `http_request` for Slack — credentials handled securely. `thread_ts` and `message_ts` are Slack timestamps (e.g. `1503435956.000247`). Emoji names have no colons (e.g. `thumbsup`).
> **`slack_send` actions (16):** `send`, `reply`, `react`, `unreact`, `edit`, `delete`, `pin`, `unpin`, `get_messages`, `get_channel`, `list_channels`, `get_user`, `list_members`, `kick_user`, `set_topic`, `send_blocks`

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

- **Telegram** — Create a bot via @BotFather, add token to config `[channels.telegram]`. Use `telegram_send` (16 actions) for full proactive control. **Always use `telegram_send` instead of `http_request`** — keeps credentials out of URLs.
- **Discord** — Create a bot at discord.com/developers (enable MESSAGE CONTENT intent), add token to config `[channels.discord]`. Use `discord_connect` to set up at runtime, `discord_send` (16 actions) for full proactive control. **Always use `discord_send` instead of `http_request`** — keeps credentials out of URLs.
- **WhatsApp** — Link via QR code pairing, configure `[channels.whatsapp]` with allowed phone numbers
- **Slack** — Create an app at api.slack.com/apps (enable Socket Mode), add tokens to config `[channels.slack]`. Use `slack_send` (16 actions) for full proactive control. **Always use `slack_send` instead of `http_request`** — keeps credentials out of URLs.
- **Trello** — Get API Key + Token at trello.com/power-ups/admin, configure `[channels.trello]`. Tool-only by default — the AI acts on Trello only when explicitly asked via `trello_send`. Opt-in polling via `poll_interval_secs` in config. Use `trello_connect` to set up at runtime, `trello_send` (21 actions) for full proactive card/board management. **Always use `trello_send` instead of `http_request`** — keeps credentials out of URLs.

API keys go in `~/.opencrabs/keys.toml` (chmod 600). Channel settings go in `config.toml`.

**Trello config example:**
```toml
# keys.toml
[channels.trello]
app_token = "your-api-key"    # ~32-char key from trello.com/power-ups/admin
token = "your-api-token"      # ~64-char token from the authorization URL

# config.toml
[channels.trello]
enabled = true
allowed_channels = ["Board Name", "other-board-id"]  # names or 24-char IDs
allowed_users = []  # Trello member IDs (empty = reply to all)
```

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
