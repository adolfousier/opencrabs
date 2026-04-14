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
| `brave_search` | `query` | `max_results` |
| `exa_search` | `query` | `max_results`, `search_type` |
| `http_request` | `method`, `url` | `headers`, `body` |
| `session_search` | `operation` | `query`, `n`, `session_id` |
| `task_manager` | `operation` | `title`, `description`, `task_id`, `status` |
| `plan` | `operation` | `title`, `description`, `task` |
| `session_context` | `operation` | `key`, `value` |
| `generate_image` | `prompt` | `filename` |
| `analyze_image` | `image` | `question` |
| `trello_connect` | `api_key`, `api_token`, `boards` | `allowed_users` |
| `trello_send` | `action` | `board_id`, `list_name`, `card_id`, `title`, `description`, `text`, `position`, `pattern`, `member_id`, `label_id`, `due_date`, `due_complete`, `checklist_id`, `item_id`, `complete`, `query`, `read_filter`, `limit`, `file_path` |
| `discord_connect` | `token`, `allowed_users` | `channel_id` |
| `discord_send` | `action` | `message`, `channel_id`, `message_id`, `emoji`, `embed_title`, `embed_description`, `embed_color`, `thread_name`, `user_id`, `role_id`, `limit`, `file_path`, `caption` |
| `telegram_send` | `action` | `message`, `chat_id`, `message_id`, `from_chat_id`, `photo_url`, `document_url`, `latitude`, `longitude`, `poll_question`, `poll_options`, `buttons`, `user_id`, `emoji` |
| `channel_search` | `operation` | `channel`, `chat_id`, `query`, `n` |
| `cron_manage` | `action` | `name`, `cron`, `tz`, `prompt`, `provider`, `model`, `thinking`, `auto_approve`, `deliver_to`, `job_id`, `enabled` |
| `slack_send` | `action` | `message`, `channel_id`, `thread_ts`, `message_ts`, `emoji`, `user_id`, `topic`, `blocks`, `limit`, `file_path`, `caption` |
| `notebook_edit` | `path`, `operation` | `cell_type`, `source`, `position`, `index`, `create_backup` |
| `parse_document` | `path` | `max_chars`, `pages`, `include_metadata` |
| `memory_search` | `query` | `n` |
| `config_manager` | `operation` | `section`, `key`, `value`, `command_name`, `command_description`, `command_prompt`, `command_action`, `path` |
| `tool_manage` | `action` | `name`, `description`, `executor`, `method`, `url`, `headers`, `command`, `params`, `requires_approval`, `timeout_secs` |
| `a2a_send` | `action`, `url` | `message`, `task_id`, `context_id`, `api_key` |
| `whatsapp_send` | `message` | `phone` |
| `whatsapp_connect` | — | `allowed_phones` |
| `browser_navigate` | `url` | `headless` |
| `browser_click` | `selector` | — |
| `browser_type` | `text` | `selector` |
| `browser_screenshot` | — | `selector` |
| `browser_eval` | `script` | — |
| `browser_content` | — | `selector`, `text_only` |
| `browser_wait` | — | `selector`, `timeout_secs`, `delay_secs` |
| `evolve` | — | `check_only` |
| `rebuild` | — | — |
| `feedback_record` | `event_type`, `dimension` | `value`, `metadata` |
| `feedback_analyze` | `query` | `limit` |
| `self_improve` | `action` | `target_file`, `description`, `rationale`, `content` |
| `spawn_agent` | `prompt` | `label` |
| `wait_agent` | `agent_id` | `timeout_secs` |
| `send_input` | `agent_id`, `text` | — |
| `close_agent` | `agent_id` | `remove` |
| `resume_agent` | `agent_id`, `prompt` | — |

> **RSI tools (Recursive Self-Improvement):** `feedback_record` logs observations to the feedback ledger — `event_type` is one of `tool_success`, `tool_failure`, `user_correction`, `provider_error`, `context_compaction`, `improvement_applied`, `pattern_observed`. `dimension` identifies what was observed (tool name, provider name, pattern label). `value` is numeric (1.0 = success, 0.0 = failure). `metadata` is optional free-text context. `feedback_analyze` queries the ledger — `query` is `summary` (overall stats), `tool_stats` (per-tool success/failure rates), `recent` (last N events), or `failures` (recent failures only). `limit` caps result count (default 50). `self_improve` modifies brain files autonomously — `action` is `apply` (edit brain file + log to ~/.opencrabs/rsi/) or `list` (show improvements). `target_file` must be a known brain file. No human approval needed. Changes are logged to `~/.opencrabs/rsi/improvements.md` and archived in `~/.opencrabs/rsi/history/YYYY-MM-DD.md`. Tool executions are auto-recorded to the feedback ledger — you don't need to call `feedback_record` for every tool call.
> **Sub-agent tools:** Use `spawn_agent` to delegate independent sub-tasks to child agents that run in parallel. Each child gets its own session and essential tools (read, write, edit, bash, glob, grep, ls, web_search) with auto-approve. Use `wait_agent` to collect results, `send_input` for follow-up instructions, `close_agent` to cancel, and `resume_agent` to continue a completed agent with new work. Children cannot spawn their own sub-agents (no recursive spawning).
> **Note:** `grep` and `glob` use `pattern` (not `query`). `bash` uses `command` (not `cmd`). File tools use `path` (not `file` or `file_path`).
> **Search tools:** Multiple web search tools are available. Defaults work out of the box; optional tools appear when the user configures API keys:
> - `web_search` — Default search (DuckDuckGo). Always available, no API key needed.
> - `exa_search` — EXA AI neural/semantic search. Available by default via free MCP endpoint (no key needed). Optional: set `EXA_API_KEY` in keys.toml for direct API access with higher rate limits. `search_type` can be `"auto"`, `"neural"`, `"fast"`, `"deep-lite"`, `"deep"`, `"deep-reasoning"`, or `"instant"`.
> - `brave_search` — **Optional.** Brave Search API with privacy focus. Available only when the user sets `[search.brave] enabled = true` in config and provides `BRAVE_API_KEY` in keys.toml. If this tool is in your tool list, prefer it for general web searches.
> When the user asks you to search the web, **check which search tools are available in your tool list**. If `brave_search` or `exa_search` are present, prefer them over `web_search` — they provide better results.
> **Incoming images/files:** When a user sends an image or file from any channel (Telegram, Discord, Slack, WhatsApp), it is downloaded to a temp file and included in the message as `<<IMG:/path/to/file>>`. The file exists at that path — you can read it, pass it to `analyze_image`, attach it to tool calls, or reference it in `bash` commands. The image is also sent to the model as vision content if the provider supports it. Do NOT ask the user to re-send or provide a URL — you already have the file.
> **`generate_image`:** Generate an image from a text prompt using Google Gemini. Returns the saved file path. Automatically sends as a native image on all channels — just include `<<IMG:path>>` in your reply or the channel handler sends it for you. Requires `[image.generation] enabled = true` in config. Run `/onboard:image` to set up.
> **`analyze_image`:** Analyze an image file (local path) or URL. Uses Google Gemini vision when configured (`[image.vision] enabled = true`), otherwise uses the provider's `vision_model` if set. Use when the current model doesn't support vision, the image is a saved file, or the user sends an image. Returns a text description.
> **Provider vision model:** If your default model doesn't support vision but another model on the same provider does, set `vision_model` in your provider config. When you call `analyze_image`, it uses the vision model on the same provider API to describe the image and returns the description as text — the chat model stays the same and gets vision capability via tool call. Gemini vision takes priority when configured. Example: MiniMax M2.7 auto-sets `vision_model = "MiniMax-Text-01"` on first run. Config: `[providers.minimax] vision_model = "MiniMax-Text-01"`.
> **Fallback providers:** If your primary LLM provider goes down, configure fallback providers to automatically retry with alternatives. Any previously configured provider (with API keys already set) can be listed as a fallback. Config: `[providers.fallback] enabled = true` with `providers = ["openrouter", "anthropic"]` (array, tried in order). Supports single (`provider = "openrouter"`) or multiple. Crabs can set this up for the human via config.toml — just add the fallback section and list providers that already have keys configured. At runtime, if the primary provider fails a request, each fallback is tried in sequence until one succeeds.
> **Trello:** `trello_connect` `boards` is an array of board names or IDs. Use `trello_send` for all Trello operations — fall back to `http_request` only if `trello_send` is unavailable. Credentials are handled securely without exposing them in URLs.
> **`trello_send` actions (22):** `add_comment`, `create_card`, `move_card`, `find_cards`, `list_boards`, `get_card`, `get_card_comments`, `update_card`, `archive_card`, `add_member_to_card`, `remove_member_from_card`, `add_label_to_card`, `remove_label_from_card`, `add_checklist`, `add_checklist_item`, `complete_checklist_item`, `list_lists`, `get_board_members`, `search`, `get_notifications`, `mark_notifications_read`, `add_attachment`
> **`add_attachment`:** Upload a local file to a Trello card. Returns the attachment URL — use `![image](url)` in a follow-up `add_comment` to display it inline.
> **Discord:** `discord_connect` `allowed_users` is an array of numeric Discord user IDs. Use `discord_send` for all Discord operations — fall back to `http_request` only if `discord_send` is unavailable. Credentials are handled securely.
> **`discord_send` actions (17):** `send`, `reply`, `react`, `unreact`, `edit`, `delete`, `pin`, `unpin`, `create_thread`, `send_embed`, `get_messages`, `list_channels`, `add_role`, `remove_role`, `kick`, `ban`, `send_file`
> **Guild-required actions:** `list_channels`, `add_role`, `remove_role`, `kick`, `ban` — these need the bot to have received at least one guild message first so the guild_id is available.
> **Telegram:** Use `telegram_send` for all Telegram operations — fall back to `http_request` only if `telegram_send` is unavailable. Credentials handled securely.
> **`telegram_send` actions (19):** `send`, `reply`, `edit`, `delete`, `pin`, `unpin`, `forward`, `send_photo`, `send_document`, `send_location`, `send_poll`, `send_buttons`, `get_chat`, `get_chat_administrators`, `get_chat_member_count`, `get_chat_member`, `ban_user`, `unban_user`, `set_reaction`
> **`channel_search` operations (3):** `list_chats` (show known chats with message counts), `recent` (last N messages in a chat), `search` (find messages by keyword). Telegram Bot API cannot fetch message history due to privacy — OpenCrabs passively captures group messages as they arrive and stores them for later search. Works across all channels (Telegram, Discord, Slack, WhatsApp). If `list_chats` returns empty, it means no messages have been captured yet — ask the user to send a message in the group first, or ask for the chat_id directly and use `telegram_send get_chat` to fetch chat info.
> **`cron_manage` actions (5):** `create` (schedule a new cron job), `list` (show all jobs), `delete` (remove a job by id), `enable` (activate a paused job), `disable` (pause a job without deleting). Jobs run in isolated sessions with configurable provider/model/thinking. Use `deliver_to` to send results to a channel (e.g. `telegram:chat_id`, `discord:channel_id`). Cron expressions follow standard 5-field format (min hour dom mon dow). See AGENTS.md for best practices on heartbeat vs cron.
> **`evolve`:** Download the latest OpenCrabs release binary from GitHub and hot-restart. Use `check_only: true` to check for updates without installing. Works on all platforms (macOS arm64/amd64, Linux arm64/amd64, Windows amd64). No Rust toolchain needed — downloads pre-built binaries. Falls back to legacy asset naming for older releases. Available as `/evolve` slash command on TUI and all channels.
> **`rebuild`:** Build OpenCrabs from source (`cargo build --release`) and hot-restart. Use when you need to build from source (e.g. after editing code). Requires Rust toolchain. Available as `/rebuild` slash command.
> **`notebook_edit`:** Edit Jupyter notebooks (.ipynb). Operations: `add_cell` (insert a new cell), `edit_cell` (modify cell source), `delete_cell` (remove by index), `move_cell` (reorder). `cell_type` is `code` or `markdown`. `position` is insertion index for `add_cell`.
> **`parse_document`:** Extract text from PDF, DOCX, HTML, and other document formats. Returns plain text content. Use `pages` to limit PDF page range (e.g. `"1-5"`). Use `max_chars` to truncate long documents.
> **`memory_search`:** Hybrid semantic search across past memory logs. Combines FTS5 keyword search + vector embeddings (768-dim, local GGUF model) via Reciprocal Rank Fusion. No API key needed, runs entirely offline. `n` controls number of results (default 5).
> **`config_manager`:** Read/write `config.toml` and `commands.toml` at runtime. Operations: `read_config` (read a section or key), `write_config` (set a key), `add_command` (create a slash command), `remove_command` (delete one), `list_commands` (show all), `set_working_directory` (change CWD). Changes are picked up by the config watcher within ~300ms.
> **`tool_manage`:** Manage runtime tools defined in `tools.toml`. Actions: `list` (show all tools), `add` (create new tool), `remove` (delete tool), `enable`/`disable` (toggle), `reload` (re-read tools.toml). Executor is `http` or `shell`. Template variables (`{{param}}`) are substituted in URL/command.
> **`a2a_send`:** Send tasks to remote A2A-compatible agents. Actions: `discover` (fetch Agent Card), `send` (send task message), `get` (check task status), `cancel` (cancel running task). `url` is the agent's base URL. `context_id` links multiple messages in a conversation.
> **`whatsapp_send`:** Send a WhatsApp message. `message` is the text content. `phone` is the recipient phone number (optional — defaults to the current chat).
> **`whatsapp_connect`:** Connect to WhatsApp via QR code pairing. `allowed_phones` filters which numbers can interact with the bot.
> **Browser tools:** Auto-detect and connect to your default Chromium-based browser (Chrome, Brave, Edge, Arc, Vivaldi, Opera, Chromium). Uses native profile (cookies, logins, extensions). `browser_navigate` launches the browser if not connected. `headless: true` runs without visible window. `browser_content` with `text_only: true` strips HTML tags. `browser_wait` polls every 200ms until `selector` appears or `timeout_secs` expires. `browser_eval` runs arbitrary JavaScript and returns the result. All browser tools share one persistent browser session per OpenCrabs instance.
> **Slack:** Always use `slack_send` instead of `http_request` for Slack — credentials handled securely. `thread_ts` and `message_ts` are Slack timestamps (e.g. `1503435956.000247`). Emoji names have no colons (e.g. `thumbsup`).
> **`slack_send` actions (17):** `send`, `reply`, `react`, `unreact`, `edit`, `delete`, `pin`, `unpin`, `get_messages`, `get_channel`, `list_channels`, `get_user`, `list_members`, `kick_user`, `set_topic`, `send_blocks`, `send_file`

## Browser Automation (CDP)

OpenCrabs has **built-in browser control** via Chrome DevTools Protocol — no Node.js, no Playwright, pure Rust. Launches a real Chrome/Chromium instance and controls it programmatically.

| Tool | What it does |
|------|-------------|
| `browser_navigate` | Navigate to URL. Pass `headless: false` for visible window. |
| `browser_click` | Click element by CSS selector. |
| `browser_type` | Type text into input field by selector. |
| `browser_screenshot` | Full-page or element screenshot. Returns file path. |
| `browser_eval` | Execute JavaScript in page context. Returns result. |
| `browser_content` | Extract text/HTML from page or element. |
| `browser_wait` | Wait for selector to appear or fixed delay. |

- **Headless (default):** No visible window. Fast, low resources. Use for automation and scraping.
- **Headed:** Visible Chrome window. Pass `headless: false`. Use for debugging.
- **Compose workflows:** navigate → wait → click → type → screenshot is a typical flow.
- **Screenshots return file paths.** Use `<<IMG:path>>` to send to channels.
- **Requires** Chrome/Chromium installed. Feature-gated: `browser` flag (enabled with `--all-features`).

---

## Dynamic Tools (Runtime)

Define tools in `~/.opencrabs/tools.toml` — callable immediately without restart. Managed via `tool_manage`.

### tool_manage actions

| Action | What |
|--------|------|
| `list` | Show all dynamic tools with enabled/disabled status |
| `add` | Create new tool (persists to tools.toml) |
| `remove` | Delete a tool |
| `enable` / `disable` | Toggle without removing |
| `reload` | Hot-reload from tools.toml |

### Executor Types

- **`http`** — Config: `method`, `url`, `headers`, `body_template`
- **`shell`** — Config: `command`, `args`, `working_dir`

### tools.toml Format

```toml
[[tools]]
name = "check_api_health"
description = "Check if the production API is responding"
executor_type = "http"
enabled = true

[tools.executor_config]
method = "GET"
url = "https://api.example.com/health"

[[tools]]
name = "disk_usage"
description = "Check disk usage on the system"
executor_type = "shell"
enabled = true

[tools.executor_config]
command = "df"
args = ["-h"]
```

Dynamic tools appear in the LLM's tool list alongside compiled tools. The agent can call them autonomously.

### Commands vs Tools

| | `commands.toml` | `tools.toml` |
|---|---|---|
| **Triggered by** | User typing `/command` | Agent deciding to call a tool |
| **Appears in** | Slash command menu | LLM tool list |
| **Use case** | User shortcuts, macros | Agent-callable external integrations |

---

## Profile-Aware Paths

All paths resolve through `opencrabs_home()`:
- **Default profile:** `~/.opencrabs/`
- **Named profile:** `~/.opencrabs/profiles/<name>/`
- **Override:** `OPENCRABS_PROFILE` env var or `-p <name>` CLI flag

Config, keys, brain files, DB, memory, logs — all scoped per profile. Token-lock isolation prevents two profiles from using the same bot credential.

---

## System Commands (macOS, Windows, Linux)

You **can** open system settings panels and manage OS-level tasks directly via `bash`. When users need to grant permissions, open the right panel for them.

**macOS** — Use `open` with URL schemes:
- Full Disk Access: `open "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles"`
- Privacy: `open "x-apple.systempreferences:com.apple.preference.security?Privacy"`
- Network: `open "x-apple.systempreferences:com.apple.NetworkSettings"`
- Bluetooth: `open "x-apple.systempreferences:com.apple.preferences.Bluetooth"`
- Display: `open "x-apple.systempreferences:com.apple.preference.displays"`
- General: `open "x-apple.systempreferences:com.apple.preferences"`
- Open any app: `open -a "AppName"`
- Open files/folders: `open /path/to/file`

**Windows** — Use `start ms-settings:` and `start`:
- Default Apps: `start ms-settings:defaultapps`
- Display: `start ms-settings:display`
- Network: `start ms-settings:network`
- Bluetooth: `start ms-settings:bluetooth`
- Privacy: `start ms-settings:privacy`
- General Settings: `start ms-settings:`
- Open any app: `start "" "C:\path\to\app.exe"`

**Linux (Ubuntu/GNOME)** — Use `gnome-control-center`:
- Network: `gnome-control-center wifi` or `gnome-control-center network`
- Bluetooth: `gnome-control-center bluetooth`
- Display: `gnome-control-center display`
- Privacy: `gnome-control-center privacy`
- Power: `gnome-control-center power`
- General: `gnome-control-center`

**Terminal-native capabilities** — Since you live in a terminal, you can also:
- Manage services: `systemctl status/start/stop/restart <service>`, `brew services`, `launchctl`
- Check system resources: `top`, `htop`, `df -h`, `free -m`, `iostat`, `vmstat`
- Manage processes: `ps aux`, `kill`, `pkill`, `nice`, `renice`
- Network diagnostics: `ping`, `curl`, `dig`, `nslookup`, `netstat`, `ss`, `traceroute`
- File operations: `find`, `locate`, `tar`, `zip`, `rsync`, `scp`, `chmod`, `chown`
- Package management: `brew`, `apt`, `dnf`, `pacman`, `choco`, `winget`, `cargo`, `npm`, `pip`
- Run any installed tool: verify with `--version` or `which`, then use it

Always tell the user exactly what permission to grant before opening a settings panel.

## System CLI Tools

OpenCrabs can leverage any CLI tool installed on the host system via `bash`. Common ones worth knowing about:

| Tool | Purpose | Check |
|------|---------|-------|
| `gh` | GitHub CLI — issues, PRs, repos, releases, actions | `gh --version` |
| `gog` | Google CLI — Gmail, Calendar (OAuth) | `gog --version` |
| `docker` | Container management | `docker --version` |
| `ssh` | Remote server access | `ssh -V` |
| `node` | Run JavaScript/TypeScript tools | `node --version` |
| `python3` | Run Python scripts and tools | `python3 --version` |
| `ffmpeg` | Audio/video processing | `ffmpeg -version` |
| `curl` | HTTP requests (fallback when `http_request` insufficient) | `curl --version` |

Before using any CLI tool, verify it's installed with the check command. Document tool-specific accounts, aliases, and usage notes below in this file.

### GitHub CLI (gh)

Authenticated GitHub CLI. Use `gh` commands instead of raw API calls:

```bash
# Issues
gh issue list
gh issue view <number>
gh issue create --title "Title" --body "Body"
gh issue close <number>
gh issue comment <number> --body "Comment"

# Pull Requests
gh pr list
gh pr view <number>
gh pr create --title "Title" --body "Body"
gh pr merge <number>
gh pr checks <number>

# Repos & Releases
gh release list
gh release create <tag> --title "Title" --notes "Notes"
gh repo view

# Actions / CI
gh run list
gh run view <run-id>
gh run watch <run-id>
```

### Google CLI (gog)

OAuth-authenticated Google Workspace CLI. Supports Gmail and Calendar.

**Setup:** Requires `GOG_KEYRING_PASSWORD` and `GOG_ACCOUNT` env vars. OAuth tokens stored in local keyring.

```bash
export GOG_KEYRING_PASSWORD="<keyring-password>"
export GOG_ACCOUNT="<google-account-email>"

# Calendar
gog calendar events --max 10
gog calendar events --start 2026-01-30 --end 2026-02-01

# Gmail — Read
gog gmail search "newer_than:1d" --max 10
gog gmail search "is:unread" --max 20
gog gmail thread <thread-id>

# Gmail — Send
gog gmail send --to recipient@email.com --subject "Subject" --body "Body"
gog gmail send --to recipient@email.com --subject "Subject" --body "Body" --cc other@email.com
```

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
- CLI tool accounts and configurations (gh, gog, etc.)
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

- **Telegram** — Create a bot via @BotFather, add token to config `[channels.telegram]`. Use `telegram_send` (19 actions) for full proactive control including `get_chat`, `get_chat_administrators`, `get_chat_member_count`, and `get_chat_member`. Use `channel_search` to browse captured message history (Telegram Bot API cannot fetch history, so messages are passively stored as they arrive). Use `telegram_send` for all operations — fall back to `http_request` only if the tool is unavailable.
- **Discord** — Create a bot at discord.com/developers (enable MESSAGE CONTENT intent), add token to config `[channels.discord]`. Use `discord_connect` to set up at runtime, `discord_send` (17 actions) for full proactive control. Use `discord_send` for all operations — fall back to `http_request` only if the tool is unavailable. Use `send_file` to upload images/files; generated images (`<<IMG:path>>`) are automatically sent as native attachments.
- **WhatsApp** — Link via QR code pairing, configure `[channels.whatsapp]` with allowed phone numbers
- **Slack** — Create an app at api.slack.com/apps (enable Socket Mode), add tokens to config `[channels.slack]`. Use `slack_send` (17 actions) for full proactive control. Use `slack_send` for all operations — fall back to `http_request` only if the tool is unavailable. Use `send_file` to upload images/files; generated images (`<<IMG:path>>`) are automatically sent as native Slack file uploads.
- **Trello** — Get API Key + Token at trello.com/power-ups/admin, configure `[channels.trello]`. Tool-only by default — the AI acts on Trello only when explicitly asked via `trello_send`. Opt-in polling via `poll_interval_secs` in config. Use `trello_connect` to set up at runtime, `trello_send` (22 actions) for full proactive card/board management. Use `trello_send` for all operations — fall back to `http_request` only if the tool is unavailable. Use `add_attachment` to upload images to cards, then embed with `![image](url)` in a comment.

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
