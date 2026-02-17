# MEMORY.md — Long-Term Memory

## ⚡ Memory Search — USE FIRST! (Token Savings)
- **Always use `memory_search` as FIRST PASS** before reading full files
- ~500 tokens for search vs ~15,000 tokens for reading full files
- Only use `memory_get` or `Read` if search doesn't provide enough context

## 🏠 Workspace
- **Workspace directory:** `~/.opencrabs/` — This is where EVERYTHING lives
- **Path tip:** Always run `echo $HOME` or `ls ~/.opencrabs/` first to confirm the resolved path before file operations.
- **What's inside:** Config (`config.toml`), memories (`memory/`), identities, tools, security, agent instructions, env files, sessions, and all state
- **Custom code lives HERE** — skills (`skills/`), plugins (`plugins/`), scripts (`scripts/`) — never in the repo
- **Repo = upstream code.** `git pull` is always safe. Your workspace is never touched by upgrades.
- **NEVER search for these paths.** They are HERE. Memorized. Done.

## Identity
- **Name:** *(filled during bootstrap)*
- **Born:** *(first session date)*
- **Human:** *(filled during bootstrap)*

## Key Context
*(Add important context here as you learn it — servers, accounts, projects, preferences)*

## Integrations
*(Track what's connected and working)*
- Example: Telegram ✅ (text + voice)
- Example: Discord ✅ (#channel-name)

## Troubleshooting
*(Document problems and fixes so future-you doesn't waste time)*

### Stale State Files = Silent Failures
**Pattern:** Something stops working mysteriously with no errors.
**Fix:** Clear state files (session JSON, update offsets, temp files), restart clean.
**Rule:** When debugging silent failures, always check for state files first.

## Lessons Learned
*(Add hard-won knowledge here)*
- **Don't give up too early** — dig deeper before declaring something unfixable
- **Path resolution:** Always verify `$HOME` before file operations
- **State files:** Many silent failures are caused by stale cached state — clear and restart
- **Rust-first policy:** When searching for new integrations, tools, or adding new features, always prioritize Rust-based crates over wrappers or other languages. Performance matters — native Rust keeps the stack lean and fast.

## Personality Notes
*(What you've learned about working with your human)*
