//! Inbox data service — pulls pending RSI proposals into a uniform
//! list of `McInboxItem` rows for the inbox panel.
//!
//! Stateless wrapper around `ProposalsStore`. Re-reads on every call so
//! the panel reflects what's actually on disk (the RSI background loop
//! and the user-facing agent can both touch the inbox concurrently —
//! see `rsi_proposals` module docs).

use super::types::{McInboxItem, McInboxKind};
use crate::brain::rsi_proposals::{
    CommandProposal, ProposalsStore, SkillProposal, ToolProposal,
};

/// Read every pending tool + command + skill proposal, sorted newest-first.
pub fn list() -> Vec<McInboxItem> {
    list_with_store(&ProposalsStore::new())
}

/// Same as [`list`] but takes an explicit store — used by tests so
/// they can point at a tmpdir without touching `~/.opencrabs/`.
pub fn list_with_store(store: &ProposalsStore) -> Vec<McInboxItem> {
    let mut items: Vec<McInboxItem> = store
        .list_tool_proposals()
        .into_iter()
        .map(item_from_tool)
        .chain(
            store
                .list_command_proposals()
                .into_iter()
                .map(item_from_command),
        )
        .chain(
            store
                .list_skill_proposals()
                .into_iter()
                .map(item_from_skill),
        )
        .collect();
    items.sort_by_key(|i| std::cmp::Reverse(i.created_at));
    items
}

fn item_from_tool(p: ToolProposal) -> McInboxItem {
    let summary = match (&p.def.command, &p.def.url) {
        (Some(cmd), _) => format!("shell: {cmd}"),
        (_, Some(url)) => format!("{} {}", p.def.method.as_deref().unwrap_or("GET"), url),
        _ => "(no command / url)".to_string(),
    };
    McInboxItem {
        id: p.id,
        label: p.def.name,
        summary,
        kind: McInboxKind::ProposedTool,
        source: p.proposer,
        created_at: p.created_at,
        detail: None,
    }
}

fn item_from_command(p: CommandProposal) -> McInboxItem {
    // Strip `prompt`/`system` action label out of the summary; the kind
    // badge already says "command", and the prompt body is the useful
    // bit a user wants to glance at.
    let summary = if p.command.prompt.is_empty() {
        format!("({})", p.command.action)
    } else {
        p.command.prompt.clone()
    };
    McInboxItem {
        id: p.id,
        label: p.command.name,
        summary,
        kind: McInboxKind::ProposedCommand,
        source: p.proposer,
        created_at: p.created_at,
        detail: None,
    }
}

fn item_from_skill(p: SkillProposal) -> McInboxItem {
    // For the inbox card the description is the most useful peek ���
    // it's the one-line summary the LLM dispatcher would see anyway,
    // and the multi-line body is too long to fit a card. The kind
    // badge already says "skill" so we don't repeat that here.
    McInboxItem {
        id: p.id,
        label: p.skill.name,
        summary: p.skill.description,
        kind: McInboxKind::ProposedSkill,
        source: p.proposer,
        created_at: p.created_at,
        detail: None,
    }
}
