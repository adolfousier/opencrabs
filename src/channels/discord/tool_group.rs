//! Collapsible tool-call group for Discord (#380), matching the Telegram
//! block and the Slack Block Kit port: ONE message per turn, collapsed to
//! a live summary with an Expand button, toggled in place via component
//! interaction. State lives in [`super::DiscordState`] keyed by message id
//! so the click handler can re-render after the turn's closures are gone.
//! Expansion is per-message: everyone in the channel shares it.
//!
//! With `trace_narration` enabled the same bubble also carries the turn's
//! intermediate narration as dim subtext notes (agent-disco-style live
//! trace): one editable work-log per turn instead of one message per
//! intermediate.

use serenity::builder::{CreateActionRow, CreateButton};
use serenity::model::application::ButtonStyle;

use super::DiscordState;

/// One tool row in a group.
#[derive(Debug, Clone)]
pub(crate) struct GroupEntry {
    pub name: String,
    pub context: String,
    /// None = running, Some(success) = finished.
    pub status: Option<bool>,
}

/// A turn's tool group: contents plus display state.
#[derive(Debug, Clone)]
pub(crate) struct GroupState {
    pub entries: Vec<GroupEntry>,
    /// Narration lines folded into the bubble (live trace). Authoritative
    /// state lives in [`DiscordState`]; only [`DiscordState::append_note`]
    /// and [`DiscordState::drop_note_if`] mutate them —
    /// [`DiscordState::upsert_tool_group`] preserves the stored notes the
    /// way it preserves `expanded`.
    pub notes: Vec<String>,
    pub expanded: bool,
}

/// Keep at most this many narration lines in the bubble (newest win).
pub(crate) const NOTE_CAP: usize = 6;

/// Clip each narration line to this many chars — the bubble stays a glance,
/// not a transcript.
pub(crate) const NOTE_MAX_CHARS: usize = 160;

/// First non-empty line, trimmed to [`NOTE_MAX_CHARS`] — the bubble form of
/// one narration event.
pub(crate) fn clip_note(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let mut out: String = line.chars().take(NOTE_MAX_CHARS).collect();
    if line.chars().count() > NOTE_MAX_CHARS {
        out.push('…');
    }
    out
}

/// Narration lines as Discord subtext (`-# ` renders dim and small).
fn notes_block(notes: &[String]) -> String {
    notes
        .iter()
        .map(|n| format!("-# {n}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn entry_icon(status: Option<bool>) -> &'static str {
    match status {
        None => "⚙️",
        Some(true) => "✅",
        Some(false) => "❌",
    }
}

fn summary_line(entries: &[GroupEntry]) -> String {
    let n = entries.len();
    let running = entries.iter().filter(|e| e.status.is_none()).count();
    let failed = entries.iter().filter(|e| e.status == Some(false)).count();
    let (icon, tail) = if running > 0 {
        ("⚙️", format!(" · {running} running"))
    } else if failed > 0 {
        ("❌", format!(" · {failed} failed"))
    } else {
        ("✅", String::new())
    };
    format!(
        "{icon} **{n} tool call{}**{tail}",
        if n == 1 { "" } else { "s" }
    )
}

/// Message body for the group in its current display state.
pub(crate) fn render_content(group: &GroupState) -> String {
    let tools_part = if group.entries.len() == 1 && !group.expanded {
        let e = &group.entries[0];
        format!("{} **{}**{}", entry_icon(e.status), e.name, e.context)
    } else if group.expanded {
        let lines: Vec<String> = group
            .entries
            .iter()
            .map(|e| format!("{} **{}**{}", entry_icon(e.status), e.name, e.context))
            .collect();
        format!("{}\n{}", summary_line(&group.entries), lines.join("\n"))
    } else {
        summary_line(&group.entries)
    };
    if group.notes.is_empty() {
        tools_part
    } else {
        format!("{tools_part}\n{}", notes_block(&group.notes))
    }
}

/// Toggle components for the group message; empty for single-tool groups
/// (a lone line has nothing extra to reveal).
pub(crate) fn render_components(group: &GroupState, message_id: u64) -> Vec<CreateActionRow> {
    if group.entries.len() < 2 {
        return Vec::new();
    }
    let label = if group.expanded {
        "Collapse ▲"
    } else {
        "Expand ▼"
    };
    vec![CreateActionRow::Buttons(vec![
        CreateButton::new(format!("toolgroup:{message_id}"))
            .label(label)
            .style(ButtonStyle::Secondary),
    ])]
}

impl DiscordState {
    /// Retained tool groups; older ones stop being toggleable (their last
    /// rendered state stays on screen, like Telegram's frozen blocks).
    const TOOL_GROUP_CAP: usize = 20;

    /// Insert or update a group, PRESERVING the stored expanded/collapsed
    /// choice on updates (a completing tool must not snap an expanded group
    /// shut) and the stored narration notes (only `append_note`/`drop_note_if`
    /// mutate those). Returns the stored state so callers render what is kept.
    pub(crate) async fn upsert_tool_group(
        &self,
        message_id: u64,
        mut group: GroupState,
    ) -> GroupState {
        let mut guard = self.tool_groups.lock().await;
        let (order, map) = &mut *guard;
        match map.get(&message_id) {
            Some(existing) => {
                group.expanded = existing.expanded;
                group.notes = existing.notes.clone();
            }
            None => {
                order.push(message_id);
                while order.len() > Self::TOOL_GROUP_CAP {
                    let oldest = order.remove(0);
                    map.remove(&oldest);
                }
            }
        }
        map.insert(message_id, group.clone());
        group
    }

    /// Flip a group's expanded state; None when it aged out of retention.
    pub(crate) async fn toggle_tool_group(&self, message_id: u64) -> Option<GroupState> {
        let mut guard = self.tool_groups.lock().await;
        let (_, map) = &mut *guard;
        let group = map.get_mut(&message_id)?;
        group.expanded = !group.expanded;
        Some(group.clone())
    }

    /// Append one narration line to the stored group, keeping only the
    /// newest [`NOTE_CAP`]. Returns the updated state, or None when the
    /// message has no stored group (aged out of retention).
    pub(crate) async fn append_note(&self, message_id: u64, note: String) -> Option<GroupState> {
        let mut guard = self.tool_groups.lock().await;
        let (_, map) = &mut *guard;
        let group = map.get_mut(&message_id)?;
        group.notes.push(note);
        if group.notes.len() > NOTE_CAP {
            group.notes.remove(0);
        }
        Some(group.clone())
    }

    /// Remove the LAST narration line matching `pred` — the final-response
    /// dedup drops the trailing note that mirrors the answer, so the trace
    /// does not double-post it as a clip. Returns the updated state, or None
    /// when nothing matched or no group is stored.
    pub(crate) async fn drop_note_if<F>(&self, message_id: u64, pred: F) -> Option<GroupState>
    where
        F: Fn(&str) -> bool,
    {
        let mut guard = self.tool_groups.lock().await;
        let (_, map) = &mut *guard;
        let group = map.get_mut(&message_id)?;
        let idx = group.notes.iter().rposition(|n| pred(n))?;
        group.notes.remove(idx);
        Some(group.clone())
    }
}
