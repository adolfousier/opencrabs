# Discord application commands (slash commands)

Issue: [#1850](https://github.com/opencrabs/opencrabs/issues/1850): OpenCrabs has no Discord slash commands.

OpenCrabs projects `commands.toml` onto Discord's native slash-command list for
every guild the bot is in, so a command defined in that file appears in the
client's `/` menu with its description and an argument hint. The menu is the
`commands.toml` catalog plus two built-ins the owner needs from Discord:
`/respond_to` (#2013) and `/cowork` (#2015). Telegram's menu also lists the other
built-ins and skills, and Discord's does not, so a skill typed as plain text still
works through the message path.

## Two different axes: intents and scopes

Both must be right or nothing works, and they fail in different ways.

**Gateway intents** are what events the bot is *told* about. OpenCrabs requests
`GUILD_MESSAGES | DIRECT_MESSAGES | MESSAGE_CONTENT`
(`src/channels/discord/agent.rs`). Intents are set in the Developer Portal under
**Bot → Privileged Gateway Intents**. `MESSAGE_CONTENT` must be enabled there or
the bot receives empty message content and looks deaf. Intents have nothing to do
with slash commands: leaving them alone does not break `/`, and enabling
`applications.commands` is not an intent.

**OAuth2 scopes** are what the bot is *allowed to do*, and they are granted at
invite time, not in the portal. Slash commands need:

```
bot applications.commands
```

If the bot was invited with `bot` only, the gateway connects, messages flow, and
every call to the command-registration endpoint returns **403**. This is the
single most common reason `/` is empty on a working OpenCrabs Discord install:
the bot looks healthy everywhere else.

## Re-inviting with the missing scope

The scope cannot be added from the portal. Re-run the invite URL:

```
https://discord.com/oauth2/authorize?client_id=<APPLICATION_ID>&scope=bot%20applications.commands&permissions=68608
```

Open it, pick the guild, Authorize. Then restart OpenCrabs, or wait: a sync where
every guild refused is **not** remembered as done, so the next reconnect tries
again and the newly granted scope is picked up without touching any config. A
reconnect alone is the reliable path, since Discord fires no new `READY` event for
an existing connection when you re-authorize it.

## How commands are chosen

`src/channels/discord/commands.rs` reads the catalog through the same
`CommandLoader` every channel uses, so entries added at runtime by the agent
appear on Discord too: the config watcher re-plans on every publish. Beside the
catalog, two built-ins are added (`MENU_BUILTINS`, `src/channels/discord/commands.rs`):
`/respond_to` and `/cowork`, both owner-only. A user command of the same name wins.
Telegram's menu also lists the other built-ins and skills, and this projection does
not. Names and descriptions that Discord would reject are
sanitized rather than dropped:

| Rule | Discord limit | What OpenCrabs does |
|------|---------------|---------------------|
| Name charset | `^[\w-]{1,32}$`, lowercase | lowercases, maps illegal characters to `-`, trims edge dashes |
| Name length | 32 characters | truncates |
| Description | 100 characters | truncates |
| Empty description | required for CHAT_INPUT | falls back to the command name |
| Commands per guild | 100 CHAT_INPUT | keeps the head of `commands.toml`, logs the dropped tail |
| Whole tree | 8000 characters | drops the entry that would cross the line, logs it |

Name collisions after sanitizing are **dropped**, not merged, and the log names
both the loser and the winner: silently merging `/foo bar` and `/foo-bar` would
run one command where the user believes they have two.

## How a slash command is executed

The interaction handler rebuilds the invocation as the text the user would have
typed (`/name args`) and routes it through the tool-loop display path that a
tapped suggestion uses (#1852), not the bare interaction router that modal forms
and select menus ride. The difference is not cosmetic: the bare route is a
single completion with no tool loop, which is correct for a form fill (a
synthetic steering prompt) and useless for `/check`, whose whole purpose is to
make the agent run cargo. The handler does **not** call the `slash_command` tool
from the channel layer, so:

- whatever the agent would do with the typed text is what it does here, because
  it is handed the same string;
- arguments survive, because they are inside the rebuilt text;
- history records the command the same way it records a typed message, so
  nothing downstream can tell the difference, which is the point.

Every command is registered with a single optional string argument (`args`),
because the catalog's commands take free-form text rather than typed parameters.

## Who can run a command

Discord shows the command list to **every member of the guild**, so an invoked
command is a new way into the agent and gets the same deny-by-default gate
(OC-02) that `handle_message` applies to typed text:

| Situation | Result |
|-----------|--------|
| No `allowed_users`, no `allowed_roles`, no `bot_owner` | **unconfigured, denies everybody** (this is not "open to all") |
| The configured `bot_owner` | admitted |
| An id in `allowed_users` | admitted |
| A member holding a role in `allowed_roles` | admitted (guild only; a DM has no roles) |
| Anyone else | refused |

The refusal is an ephemeral message, so only the person who tapped it sees it,
plus a `warn` line naming which check failed. Channel scope travels with the
identity check: `allowed_channels` applies, including the parent fallback that
lets an allow-listed forum admit its posts. A command is solicited, so only a
channel's `dm_only` mode blocks it; that mode is read per channel, with the thread →
parent → global fallback (#2014). In a `mention` channel an unmentioned command is
dropped, except the owner's `/respond_to` and `/cowork`, which pass the gate (#2016).

## Per-channel settings and the owner commands

Each Discord channel, or a forum/thread parent, can have its own entry:

```toml
[channels.discord.channels.1473207147025137778]
name = "general"             # display only; access never reads it
respond_to = "all"           # this channel's mode; unset inherits the global respond_to
open = true                  # any member of this channel is admitted (ACL)
```

- **`open`** admits every member of the channel, and its threads and forum posts,
  past `allowed_users`. It never admits anyone while the bot has no
  `allowed_users`, `allowed_roles` or `bot_owner`. DMs and other channels stay locked.
- **`/respond_to`** (owner) typed in a channel or thread shows the mode that applies
  there. With an argument (`all`, `dm_only`, `mention`, `auto`) it writes that
  channel's own `respond_to`. Threads write their own entry, which wins over the parent's.
- **`/cowork`** (owner) in a server channel or thread writes `open = true` and the
  channel's `name`. It refuses in a DM. Members are not registered; `open` admits them.
- Both commands are refused for non-owners before anything is written, and a failed
  write is reported in the channel. Slack and WhatsApp answer `/respond_to` from their
  channel-level setting and do not write it.
- `auto` on Discord behaves as `mention`.

The verdict itself lives in `identity_admitted()` and `holds_allowed_role()` as
pure functions, which is what lets the deny-by-default case have a test: there
is no Discord application in CI, so an inline `if` in the gateway handler would
have shipped unproven.

## Rate limits that shaped the implementation

Discord's application-command limits are real and OpenCrabs stays inside them:

- **200 command creates per day per guild**
  ([docs](https://discord.com/developers/docs/interactions/application-commands#rate-limits)).
  This counts per-command `POST` creates. OpenCrabs uses the **bulk overwrite**
  route (`GuildId::set_commands` → `PUT /applications/<app>/guilds/<guild>/commands`),
  which replaces the whole set in one request and does not consume that budget.
- **5 requests per second per route.** OpenCrabs keeps a comparison key over the
  projected set plus the guild list, so an unchanged `commands.toml` re-read costs
  no API call at all and only a real change re-syncs. Guild membership is part of
  the key on purpose: a server the bot joined since the last sync moves it, so the
  new guild gets its menu without waiting for a config edit.
- Commands are synced on `ready` and on every config-publish. A reconnect
  re-plans, and the key comparison decides whether anything is sent, so a gateway
  that flaps on a short retry loop cannot turn into a registration storm. The
  trade-off is that a guild joined while the process is up is picked up on the
  next reconnect, not instantly.
