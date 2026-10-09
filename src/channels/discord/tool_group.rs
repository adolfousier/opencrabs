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
use std::time::{Duration, Instant};
use uuid::Uuid;

use super::DiscordState;
use crate::channels::background_work::FlowOutcome;

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
    /// Turn-start anchor for the live `🕒` segment and the settled `⏱` one
    /// (#1841). Stamped at first insert and preserved across updates so the
    /// clock never restarts mid-turn.
    pub started_at: Instant,
    /// Post-delivery status (#1841). `None` while the turn is live; stamped
    /// once by [`DiscordState::settle_tool_group`] and preserved by every
    /// later upsert. `elapsed` freezes at settle so toggling Expand later
    /// never grows the clock.
    pub settled: Option<SettledStatus>,
}

/// Frozen post-delivery chrome (#1841): the Discord twin of Slack's
/// `SettledStatus`. The clock stops at settle and the ctx budget line moves
/// into the flow group, so the chrome owns it (the answer-message footer
/// goes away in #1842, leaving the settled line as the single home).
#[derive(Debug, Clone)]
pub(crate) struct SettledStatus {
    pub elapsed: Duration,
    pub ctx: Option<String>,
    /// Turn ended with background work still alive (#1987): the settled line
    /// reads `⏳ {verb}` instead of the finished icon. The completion path
    /// narrows this as work drains and clears it (flip to finished) once
    /// both registries are empty.
    pub waiting: Option<String>,
    /// Terminal state of a turn that ended anywhere but normally (#1911):
    /// `None` means the turn delivered its answer, so the line keeps the
    /// plain counts chrome. `Some` stamps the settled outcome's own icon and
    /// verb (`⏱ Timed out`, `❌ Failed`, `❌ Cancelled`) from the shared
    /// [`FlowOutcome`], the same vocabulary Telegram's settled header reads.
    /// Set by the Cancelled/Err delivery arms (#1987) so the group stops
    /// claiming success and the flow ticker's clock ends with the turn
    /// instead of running to its 30-minute orphan cap.
    pub outcome: Option<FlowOutcome>,
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

/// `M:SS` elapsed clock (`H:MM:SS` past an hour), the Discord twin of
/// Telegram's flow clock. The glyph lives with the caller so the live and
/// settled segments can differ (`🕒` rolls, `⏱️` freezes at settle).
fn clock(elapsed: Duration) -> String {
    let (h, m, s) = (
        elapsed.as_secs() / 3600,
        (elapsed.as_secs() % 3600) / 60,
        elapsed.as_secs() % 60,
    );
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Longest activity segment in the live flow line (#1844). Display-only
/// cap: the line stays a glance, not a transcript.
const ACTIVITY_MAX_CHARS: usize = 100;

fn activity_segment(group: &GroupState) -> Option<String> {
    let text = latest_activity(group)?;
    let clipped: String = text.chars().take(ACTIVITY_MAX_CHARS).collect();
    let out = if text.chars().count() > ACTIVITY_MAX_CHARS {
        format!("{clipped}…")
    } else {
        clipped
    };
    (!out.is_empty()).then_some(out)
}

/// Live activity preview for the flow line (#1844): what the agent is doing
/// right now, the Discord twin of Slack's `latest_activity` (#1809) and
/// Telegram's `latest_activity_preview` (telegram/flow.rs). Same
/// priorities: latest human-readable narration note, then line-start `#`
/// comments from the latest bash command, then the latest tool label +
/// context. Discord keeps notes in their own `Vec` (`GroupState.notes`),
/// not as entries.
fn latest_activity(group: &GroupState) -> Option<String> {
    if let Some(text) = group
        .notes
        .iter()
        .rev()
        .find_map(|n| crate::channels::telegram::flow::human_readable_preview(n))
    {
        return Some(text);
    }
    if let Some(comments) = group.entries.iter().rev().find_map(|e| {
        if e.name == "bash" {
            crate::channels::telegram::flow::extract_status_from_text(&e.context)
        } else {
            None
        }
    }) {
        return Some(comments);
    }
    group
        .entries
        .iter()
        .rev()
        .map(|e| {
            let ctx = e.context.trim_start();
            if ctx.is_empty() {
                e.name.clone()
            } else {
                // Contexts are stored with a leading space (` (arg0)`): trim,
                // then join with one canonical space (the #1809 double-space fix).
                format!("{} {ctx}", e.name)
            }
        })
        .next()
}

fn summary_line(group: &GroupState) -> String {
    let n = group.entries.len();
    let failed = group
        .entries
        .iter()
        .filter(|e| e.status == Some(false))
        .count();
    let counts = format!("**{n} tool call{}**", if n == 1 { "" } else { "s" });
    match &group.settled {
        Some(s) => {
            // Settled chrome (#1841): frozen clock, ctx budget as the last
            // word before it, mirroring the Telegram settled header order.
            // A settled turn has no running tools: the icon reads final
            // states only, ❌ when something failed, ✅ otherwise, never
            // the live "N running" tail (an entry left statusless at
            // settle is done, not running). #1987 puts two overrides ahead
            // of that default: a waiting verb (⏳, the turn ended with
            // background work alive) and a terminal outcome (#1911, the
            // Cancelled/Err arms settle the group so its clock stops). The
            // outcome prints its own icon and verb, so a timed-out turn now
            // reads `⏱ Timed out` where a crashed one reads `❌ Failed`,
            // matching the Telegram settled header instead of collapsing
            // both into one cross.
            let mut line = if let Some(verb) = &s.waiting {
                format!("⏳ {verb} · {counts}")
            } else if let Some(outcome) = s.outcome {
                let (icon, verb) = outcome.icon_verb();
                format!("{icon} {verb} · {counts}")
            } else if failed > 0 {
                format!("❌ {counts} · {failed} failed")
            } else {
                format!("✅ {counts}")
            };
            if let Some(ctx) = &s.ctx {
                line.push_str(&format!(" · {ctx}"));
            }
            line.push_str(&format!(" · ⏱️ {}", clock(s.elapsed)));
            line
        }
        None => {
            // Live line (#1844): activity text leads when there is any
            // (latest note > bash # comments > tool label), bare counts
            // shape otherwise (zero-entry turn-start shell). Settled arm
            // above stays clean.
            let running = group.entries.iter().filter(|e| e.status.is_none()).count();
            let (icon, tail) = if running > 0 {
                ("⚙️", format!(" · {running} running"))
            } else if failed > 0 {
                ("❌", format!(" · {failed} failed"))
            } else {
                ("✅", String::new())
            };
            let clock = format!("🕒 {}", clock(group.started_at.elapsed()));
            match activity_segment(group) {
                Some(activity) => format!("{icon} {activity} · {counts}{tail} · {clock}"),
                None => format!("{icon} {counts}{tail} · {clock}"),
            }
        }
    }
}

/// Discord hard-caps message content at 2000 chars (#1949): anything
/// longer is rejected on the wire, and a rejected toggle response leaves
/// the interaction unresolved — the client then blames a timeout. Every
/// render path (live edits, settle, Expand responses) fits this cap by
/// construction.
pub(crate) const CONTENT_MAX_CHARS: usize = 2000;

/// Room reserved for the omission marker so the clamped body stays under
/// [`CONTENT_MAX_CHARS`] once the marker is appended.
const OMIT_MARKER_RESERVE: usize = 80;

/// Keep newest rows of an expansion that overshoots the cap. Walks rows
/// backwards (the freshest activity is what people expand for), restores
/// chronology, and states how many rows were dropped — never silently.
fn clamp_rows(summary: String, rows: Vec<String>) -> String {
    let budget = CONTENT_MAX_CHARS - OMIT_MARKER_RESERVE;
    let mut used = summary.chars().count() + 1;
    let mut kept: Vec<String> = Vec::new();
    let mut dropped = 0usize;
    for row in rows.into_iter().rev() {
        let cost = row.chars().count() + 1;
        if used + cost > budget {
            dropped += 1;
            continue;
        }
        used += cost;
        kept.push(row);
    }
    let mut out = format!(
        "{summary}\n{}",
        kept.into_iter().rev().collect::<Vec<_>>().join("\n")
    );
    if dropped > 0 {
        out.push_str(&format!(
            "\n_{dropped} omitted to fit Discord's message limit_"
        ));
    }
    out
}

/// Final wire guard: hard-cut at the cap on a char boundary. Entry rows
/// and notes are clipped upstream, so this only exists for pathological
/// callers — an oversized message must never reach the API.
fn hard_clip(body: String) -> String {
    if body.chars().count() <= CONTENT_MAX_CHARS {
        return body;
    }
    let cut: String = body.chars().take(CONTENT_MAX_CHARS - 1).collect();
    format!("{cut}…")
}

/// Message body for the group in its current display state.
pub(crate) fn render_content(group: &GroupState) -> String {
    // The bare entry line is a LIVE-only shortcut. Once settled, the
    // final-state chrome (⏳ waiting verb, outcome icon+verb, ✅/❌ counts,
    // ctx, frozen clock) must render even for single-tool turns. #1987:
    // a lone aborted call otherwise kept showing a plain entry row and
    // hid the Cancelled/Error/Waiting state from the user entirely.
    let tools_part = if group.entries.len() == 1 && !group.expanded && group.settled.is_none() {
        let e = &group.entries[0];
        format!("{} **{}**{}", entry_icon(e.status), e.name, e.context)
    } else if group.expanded {
        let lines: Vec<String> = group
            .entries
            .iter()
            .map(|e| format!("{} **{}**{}", entry_icon(e.status), e.name, e.context))
            .collect();
        clamp_rows(summary_line(group), lines)
    } else {
        summary_line(group)
    };
    let mut body = if group.notes.is_empty() {
        tools_part.clone()
    } else {
        format!("{tools_part}\n{}", notes_block(&group.notes))
    };
    // Notes ride after the rows; if the whole body still overshoots, drop
    // the oldest notes first (the newest is what the ticker just wrote),
    // and hard-cut as the last resort.
    let mut notes: Vec<String> = group.notes.clone();
    while body.chars().count() > CONTENT_MAX_CHARS && notes.len() > 1 {
        notes.remove(0);
        body = format!("{tools_part}\n{}", notes_block(&notes));
    }
    hard_clip(body)
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
                group.started_at = existing.started_at;
                group.settled = existing.settled.clone();
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

    /// Stamp the post-delivery status (#1841): freeze the clock at now and
    /// record the ctx budget line for the settled chrome. A `None` ctx keeps
    /// whatever a previous settle stamped, so a re-settle never clears the
    /// budget. `waiting` (Some verb, #1987) settles the line to the ⏳ form
    /// because the turn ended with background work alive; `outcome` (Some,
    /// #1911) settles it to that state's own icon and verb, so the Cancelled
    /// and Err delivery arms stop the ticker's clock with the turn and the
    /// group never claims success. `None` is the normal delivery: the line
    /// keeps the plain ✅/❌ counts chrome. Returns the updated state, or
    /// None when the message has no stored group (aged out of retention).
    pub(crate) async fn settle_tool_group(
        &self,
        message_id: u64,
        ctx: Option<String>,
        waiting: Option<String>,
        outcome: Option<FlowOutcome>,
    ) -> Option<GroupState> {
        let mut guard = self.tool_groups.lock().await;
        let (_, map) = &mut *guard;
        let group = map.get_mut(&message_id)?;
        let prev_ctx = group.settled.as_ref().and_then(|s| s.ctx.clone());
        group.settled = Some(SettledStatus {
            elapsed: group.started_at.elapsed(),
            ctx: ctx.or(prev_ctx),
            waiting,
            outcome,
        });
        Some(group.clone())
    }

    /// Narrow or clear the waiting line of a settled group (#1987): the
    /// background-completion path recomputes both registries and either
    /// replaces the verb (some work left) or clears it (flip to finished).
    /// The frozen clock and ctx budget are untouched. Returns the updated
    /// state, or None when the group aged out of retention.
    pub(crate) async fn refresh_waiting_line(
        &self,
        message_id: u64,
        waiting: Option<String>,
    ) -> Option<GroupState> {
        let mut guard = self.tool_groups.lock().await;
        let (_, map) = &mut *guard;
        let group = map.get_mut(&message_id)?;
        group.settled.as_mut()?.waiting = waiting;
        Some(group.clone())
    }

    /// Remember that this session's turn settled with live background work
    /// (#1987), so the completion path can find the group after the turn's
    /// closures are gone. One group per session; the newest settle wins.
    pub(crate) async fn register_waiting_group(&self, session_id: Uuid, message_id: u64) {
        self.waiting_groups
            .lock()
            .await
            .insert(session_id, message_id);
    }

    /// The message id of a session's waiting group, if one is registered.
    pub(crate) async fn waiting_group_for(&self, session_id: Uuid) -> Option<u64> {
        self.waiting_groups.lock().await.get(&session_id).copied()
    }

    /// Drop a session's waiting-group registration (flipped to finished or
    /// the group aged out).
    pub(crate) async fn clear_waiting_group(&self, session_id: Uuid) -> Option<u64> {
        self.waiting_groups.lock().await.remove(&session_id)
    }

    /// Clone the live or settled group for out-of-loop renderers (#1843):
    /// the flow ticker snapshots under the lock, renders outside it, and
    /// re-snapshots after its edit so the settled line keeps the last word.
    pub(crate) async fn tool_group_snapshot(&self, message_id: u64) -> Option<GroupState> {
        let guard = self.tool_groups.lock().await;
        let (_, map) = &*guard;
        map.get(&message_id).cloned()
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

#[cfg(test)]
mod cap_tests {
    use super::*;

    fn probe(entries: usize, notes: usize, expanded: bool) -> GroupState {
        GroupState {
            entries: (0..entries)
                .map(|i| GroupEntry {
                    name: format!("tool{i}"),
                    context: format!(" (long context line to reach the cap sooner #{i})"),
                    status: Some(true),
                })
                .collect(),
            notes: (0..notes)
                .map(|i| clip_note(&"n".repeat(NOTE_MAX_CHARS + 40 + i)))
                .collect(),
            expanded,
            started_at: Instant::now(),
            settled: Some(SettledStatus {
                elapsed: Duration::from_secs(3),
                ctx: None,
                waiting: None,
                outcome: None,
            }),
        }
    }

    #[test]
    fn huge_expansion_fits_the_cap_and_states_drops() {
        let rendered = render_content(&probe(300, 6, true));
        let chars = rendered.chars().count();
        assert!(chars <= CONTENT_MAX_CHARS, "expansion overshot: {chars}");
        assert!(
            rendered.contains("_") && rendered.contains("omitted to fit"),
            "silent drop: {rendered}"
        );
        assert!(
            rendered.contains("tool299"),
            "the newest entry must survive the clamp"
        );
    }

    #[test]
    fn small_expansion_is_untouched() {
        let rendered = render_content(&probe(3, 0, true));
        assert!(rendered.contains("tool0") && rendered.contains("tool2"));
        assert!(!rendered.contains("omitted to fit"));
    }

    #[test]
    fn collapsed_note_flood_fits_the_cap() {
        let rendered = render_content(&probe(30, 6, false));
        assert!(rendered.chars().count() <= CONTENT_MAX_CHARS);
    }

    #[test]
    fn multibyte_context_never_splits_a_char() {
        let mut g = probe(2, 0, true);
        g.entries[0].context = " (".repeat(3000);
        let rendered = render_content(&g);
        assert!(rendered.chars().count() <= CONTENT_MAX_CHARS);
        assert!(!rendered.ends_with('\u{FFFD}'));
    }

    #[test]
    fn marker_counts_exactly_the_dropped_rows() {
        let rendered = render_content(&probe(250, 0, true));
        // Line 0 is the summary (`✅ **250 tool calls** · …`), which also
        // starts with an entry icon — only the entry rows count as shown.
        let shown = rendered
            .lines()
            .skip(1)
            .filter(|l| l.starts_with(['✅', '❌', '\u{2699}']))
            .count();
        assert!(shown > 0 && shown < 250, "clamp kept {shown} rows");
        assert!(
            rendered.contains(&format!("_{} omitted to fit", 250 - shown)),
            "marker disagrees with visible rows:\n{rendered}"
        );
    }
}
