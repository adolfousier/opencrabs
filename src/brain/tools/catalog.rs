//! Tool tiering for lazy schema loading.
//!
//! Every agent turn used to ship ALL ~95 registered tool schemas (~20k tokens)
//! to the provider, even for a "reply yes" turn — they're counted in
//! `input_tokens` on every request. This module splits tools into a small
//! always-injected CORE set and an EXTENDED set the agent pulls on demand via
//! the `tool_search` tool (mirroring how contextual brain files are loaded via
//! `load_brain_file`).
//!
//! When `[agent] lazy_tools` is on, a request carries: CORE ∪ `tool_search` ∪
//! whatever EXTENDED tools the session has activated via `tool_search`. When
//! off, behaviour is unchanged (all tools every request).

/// The discovery tool's name — always injected alongside the core set so the
/// agent can always reach for additional tools.
pub const TOOL_SEARCH_NAME: &str = "tool_search";

/// System-prompt note appended when `lazy_tools` is on. Tells the model it has
/// a core set and must call `tool_search` for anything else, so it doesn't
/// assume a capability is missing and give up. Shared across the TUI and CLI
/// entry points so the guidance stays in one place.
pub const LAZY_TOOLS_PROMPT: &str = "\n\n--- Tool Access ---\n\
    You have a CORE set of tools always available (file read/write/edit, bash, ls/glob/grep, \
    web/exa/memory search, task/context/plan, http, the brain-file loader, config, session). \
    You do NOT see every tool by default. For anything else — browsing or clicking web pages, \
    sending channel messages (Telegram/Discord/Slack/WhatsApp), spawning sub-agents or teams, \
    generating or analyzing images/video, creating real documents (Excel with live \
    formulas, Word, PDF, PowerPoint via generate_document), parsing documents, cron jobs, \
    self-improvement/rebuild/evolve — call \
    `tool_search` FIRST with a short description of what you need. It returns the exact tool's \
    schema and makes it callable for the rest of the session. NEVER say you can't do something \
    before searching for the tool.\n";

/// Fundamental tools injected on EVERY request. Chosen to cover almost any
/// task without a discovery round-trip: file I/O, shell, the common searches,
/// core workflow/orchestration, the brain-file loader, and config/session
/// basics. Everything else is reached through `tool_search`.
///
/// Vision tools (`analyze_image`, `analyze_video`) are included here so they're
/// always available when configured. They're only registered by
/// `register_config_dependent_tools` when a vision backend exists, so they
/// won't appear in the active set if vision isn't configured.
pub const CORE_TOOLS: &[&str] = &[
    // File I/O + shell — the bread and butter
    "read_file",
    "write_file",
    "edit_file",
    "hashline_edit",
    "bash",
    "ls",
    "glob",
    "grep",
    // Search — hit constantly
    "web_search",
    "exa_search",
    "memory_search",
    // Workflow / orchestration
    "session_context",
    "plan",
    "http_request",
    // Loaders + system basics
    "load_brain_file",
    "write_opencrabs_file",
    "config_manager",
    "slash_command",
    "rename_session",
    "suggest_options",
    // Vision — always available when configured (only registered if vision backend exists)
    "analyze_image",
    "analyze_video",
    // The discovery tool itself
    TOOL_SEARCH_NAME,
];

/// True when `name` is part of the always-injected core set (or the discovery
/// tool). Core tools are never gated behind `tool_search`.
pub fn is_core(name: &str) -> bool {
    CORE_TOOLS.contains(&name)
}

/// Built-in EXTENDED tools, grouped for the flat inventory injected into the
/// system prompt (#448). These are real, registered tool names whose SCHEMAS
/// are withheld until `tool_search` activates them — but the model never sees
/// even the names otherwise, so it would claim a capability is missing instead
/// of searching for it (#449). Listing names-only (no schemas) is a low-token
/// nudge: the model can see the tool exists and which name to activate.
///
/// Built-ins only. Dynamic `tools.toml` tools and skills vary per install and
/// are surfaced separately (the "Available Commands & Skills" index), so the
/// rendered block points at those rather than hardcoding them here.
///
/// The names are ground truth (each tool's `name()`), kept adjacent to
/// `CORE_TOOLS` and `is_protected_builtin` so the three move together. When you
/// add a new built-in extended tool, add its name to the right group here so
/// the model keeps seeing it. `tool_inventory_names_are_extended_not_core`
/// (test) fails if any listed name is a core tool or a duplicate, catching the
/// most common drift; a fully registry-backed coverage check would need the DB,
/// so this stays a curated list guarded against the cheap mistakes.
pub const EXTENDED_TOOL_INVENTORY: &[(&str, &[&str])] = &[
    (
        "channels",
        &[
            "telegram_send",
            "telegram_connect",
            "discord_send",
            "discord_connect",
            "slack_send",
            "slack_connect",
            "whatsapp_send",
            "whatsapp_connect",
            "trello_send",
            "trello_connect",
            "cowork_connect",
        ],
    ),
    (
        "browser",
        &[
            "browser_navigate",
            "browser_click",
            "browser_type",
            "browser_eval",
            "browser_content",
            "browser_screenshot",
            "browser_wait",
            "browser_find",
            "browser_close",
        ],
    ),
    (
        "agents",
        &[
            "spawn_agent",
            "wait_agent",
            "send_input",
            "close_agent",
            "resume_agent",
            "session_notify",
            "team_create",
            "team_delete",
            "team_broadcast",
        ],
    ),
    ("media", &["generate_image"]),
    (
        "documents",
        &["generate_document", "parse_document", "pdf_to_images"],
    ),
    (
        "system",
        &[
            "self_improve",
            "rebuild",
            "evolve",
            "tool_manage",
            "feedback_analyze",
            "feedback_record",
        ],
    ),
    (
        "utility",
        &[
            "cron_manage",
            "goal_manage",
            "session_search",
            "channel_search",
            "brave_search",
            "mission_control_report",
            "a2a_send",
            "profile_list",
            "tasks_list",
            "execute_code",
            "notebook_edit",
            "web_scrape",
        ],
    ),
];

/// Interactive-surface-only tool names (#129): unregistered on every headless
/// path by `register_core_agent_tools(headless = true)` (and `ALWAYS_EXCLUDED`
/// for sub-agents), so the headless roster must not advertise them either.
/// Kept adjacent to the registry gate in `cli/tool_setup.rs` — when you gate a
/// tool there, add its name here so the prompt stops promising it.
const HEADLESS_UNADVERTISED: &[&str] = &["suggest_options", "session_notify"];

/// Render the flat tool inventory block (#448): the built-in extended tool
/// names grouped by area, names only. Appended after `LAZY_TOOLS_PROMPT` so the
/// model has a concrete roster to `tool_search` against instead of guessing a
/// tool is absent.
///
/// `headless` (#129, owner-ruled catalog strip, v0.4.98 ruling a) filters the
/// roster to tools the headless surface can actually register: the interactive
/// only pair (`suggest_options`, `session_notify`) is excluded at the registry
/// level, so advertising their names here would promise a capability the
/// surface does not have — the same honesty law the registry gate implements.
pub fn tool_inventory_prompt(headless: bool) -> String {
    let mut out = String::from(
        "\nAVAILABLE EXTENDED TOOLS — you have these too, their schemas just aren't loaded yet. \
         Pick the name and `tool_search` it to activate; NEVER claim one of these is missing \
         without searching first:\n",
    );
    for (category, names) in EXTENDED_TOOL_INVENTORY {
        let names: Vec<&&str> = names
            .iter()
            .filter(|n| !(headless && HEADLESS_UNADVERTISED.contains(&**n)))
            .collect();
        if names.is_empty() {
            continue;
        }
        out.push_str("  ");
        out.push_str(category);
        out.push_str(": ");
        out.push_str(&names.iter().map(|n| **n).collect::<Vec<&str>>().join(", "));
        out.push('\n');
    }
    out.push_str(
        "Plus any project dynamic tools (tools.toml) and skills listed under \
         \"Available Commands & Skills\". Names only here — `tool_search` returns the schema.\n",
    );
    out
}

/// The full tool-access system-prompt section: the `LAZY_TOOLS_PROMPT` guidance
/// followed by the flat inventory (#448/#449). Single source for every prompt
/// assembly site (TUI startup, CLI, and the live brain rebuild) so the guidance
/// and the roster never drift apart.
///
/// `headless` filters the roster (see [`tool_inventory_prompt`]); interactive
/// surfaces pass `false` and get the full roster verbatim.
pub fn tool_access_prompt(headless: bool) -> String {
    format!("{LAZY_TOOLS_PROMPT}{}", tool_inventory_prompt(headless))
}

/// Built-in tools the RSI must never tell the agent to avoid/ban/stop using.
///
/// Banning a built-in removes capability rather than fixing anything, and these
/// tools' "failures" are environmental or recoverable (a channel needing a live
/// bot, a stale-hash retry, a declined prompt), not defects (#236). This is the
/// core set plus the proactive channel, browser, and runtime built-ins that are
/// registered outside `CORE_TOOLS`. Dynamic (`tools.toml`) tools are
/// intentionally absent — those the user defines and can legitimately disable.
pub fn is_protected_builtin(name: &str) -> bool {
    is_core(name)
        || matches!(
            name,
            "telegram_send"
                | "whatsapp_send"
                | "discord_send"
                | "slack_send"
                | "trello_send"
                | "telegram_connect"
                | "whatsapp_connect"
                | "discord_connect"
                | "slack_connect"
                | "trello_connect"
                | "cowork_connect"
                | "cron_manage"
                | "goal_manage"
                | "session_search"
                | "channel_search"
                | "mission_control_report"
                | "a2a_send"
                | "profile_list"
                | "tasks_list"
                | "tool_manage"
                | "execute_code"
                | "notebook_edit"
                | "parse_document"
                | "pdf_to_images"
                | "browser_navigate"
                | "browser_click"
                | "browser_type"
                | "browser_eval"
                | "browser_content"
                | "browser_screenshot"
                | "browser_wait"
                | "browser_find"
                | "browser_close"
        )
}

/// Coarse category for an EXTENDED tool, derived from its name. Used to group
/// `tool_search` results and to catalog the extended set in TOOLS.md. Returns
/// `"core"` for core tools.
pub fn tool_category(name: &str) -> &'static str {
    if is_core(name) {
        return "core";
    }
    match name {
        n if n.starts_with("browser_") => "browser",
        n if n.starts_with("telegram_")
            || n.starts_with("whatsapp_")
            || n.starts_with("discord_")
            || n.starts_with("slack_")
            || n.starts_with("trello_") =>
        {
            "channels"
        }
        n if n.starts_with("spawn_agent")
            || n.starts_with("wait_agent")
            || n.starts_with("send_input")
            || n.starts_with("close_agent")
            || n.starts_with("resume_agent")
            || n.starts_with("session_notify")
            || n.starts_with("team_") =>
        {
            "agents"
        }
        n if n.starts_with("analyze_image")
            || n.starts_with("analyze_video")
            || n.starts_with("generate_image")
            || n.starts_with("provider_vision") =>
        {
            "media"
        }
        n if n.starts_with("feedback_")
            || n == "self_improve"
            || n == "rebuild"
            || n == "evolve"
            || n == "tool_manage" =>
        {
            "system"
        }
        n if n == "generate_document" || n == "parse_document" || n == "pdf_to_images" => {
            "documents"
        }
        n if n == "cron_manage"
            || n == "goal_manage"
            || n == "session_search"
            || n == "channel_search"
            || n == "a2a_send"
            || n == "profile_list"
            || n == "tasks_list" =>
        {
            "utility"
        }
        "web_scrape" => "web",
        _ => "other",
    }
}
