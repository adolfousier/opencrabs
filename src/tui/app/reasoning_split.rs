//! Split persisted assistant content back into its chronological segments.
//!
//! The tool loop persists a turn one iteration at a time, each append being a
//! `<!-- reasoning -->` block followed by that iteration's visible text. Reload
//! used to split only on TOOL markers, so a turn that ran no tools collapsed
//! every iteration into ONE assistant message: think, text, think, text became
//! a single row holding all the text and one merged `details` blob.
//!
//! That single row is then the turn's last non-empty assistant message, which
//! makes it the final answer, which the per-turn fold must always show. A
//! reasoning-heavy turn therefore rendered as a wall after a restart while the
//! live view, where each iteration was its own row, had folded it away.
//!
//! Splitting on the reasoning markers restores the live layout, and it works on
//! rows already in the database because it changes no storage format.
//!
//! A marker counts only at the start of a line, which is how the persist
//! path writes every one of them. The model quoting a marker mid-sentence
//! (it reads its own history through `session_search`) used to close a real
//! block early and turn the rest of the reasoning into the answer (#1587).

/// One chronological piece of a persisted assistant message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Segment {
    /// A `<!-- reasoning -->` block's contents.
    Reasoning(String),
    /// Visible text between blocks.
    Text(String),
    /// A phantom-blocked section's contents (#1172): narration the detector
    /// refused to deliver, persisted under paired markers so a misjudged
    /// deliverable stays recoverable. Its own reasoning markers are folded
    /// in, so the whole section is one collapsed row and never the answer.
    Blocked(String),
}

use super::ledger_scan::find_at_line_start;

const OPEN: &str = "<!-- reasoning -->";
const CLOSE: &str = "<!-- /reasoning -->";
const BLOCKED_OPEN: &str = "<!-- phantom_blocked=1 -->";
const BLOCKED_CLOSE: &str = "<!-- /phantom_blocked=1 -->";

/// Heading a collapsed blocked section carries, so an expanded row says what
/// it is instead of reading like an answer that was hidden by mistake.
pub(crate) const BLOCKED_LABEL: &str = "Blocked narration (phantom tool calls, never delivered):";

/// Split `content` into text, reasoning and blocked segments, in order.
///
/// Empty pieces are dropped, so a turn whose iterations were pure reasoning
/// yields only `Reasoning` segments and never a blank row. An unclosed block
/// takes the rest of the input as reasoning: the alternative is dumping a
/// half-written chain into the visible answer, which is the exact failure this
/// exists to prevent.
///
/// Phantom-blocked sections are cut out first (#1584). The split used to know
/// only the reasoning markers, so a blocked section came apart into its
/// opener line, its reasoning, and its narration plus closer as the last text
/// segment, which is the final answer by position. The 21K-character wall the
/// self-heal had drained from the live view then rendered in full on the next
/// reload, closing marker and all. An unclosed opener folds to the end of the
/// row for the same reason an unclosed reasoning block does.
pub(crate) fn split_segments(content: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut rest = content;

    while let Some(start) = find_at_line_start(rest, BLOCKED_OPEN, 0) {
        push_reasoning_segments(&mut out, &rest[..start]);
        let after_open = &rest[start + BLOCKED_OPEN.len()..];
        let (inner, after) = match find_at_line_start(after_open, BLOCKED_CLOSE, 0) {
            Some(end) => (&after_open[..end], &after_open[end + BLOCKED_CLOSE.len()..]),
            None => (after_open, ""),
        };
        let folded = fold_blocked(inner);
        if !folded.is_empty() {
            out.push(Segment::Blocked(folded));
        }
        rest = after;
    }
    push_reasoning_segments(&mut out, rest);
    out
}

/// Flatten a blocked section into one string: its reasoning and narration in
/// order, markers gone, so the collapsed row shows what the model produced.
fn fold_blocked(inner: &str) -> String {
    let mut parts = Vec::new();
    push_reasoning_segments(&mut parts, inner);
    parts
        .into_iter()
        .map(|s| match s {
            Segment::Reasoning(r) | Segment::Text(r) | Segment::Blocked(r) => r,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Append `content` to `out` as alternating text and reasoning segments.
fn push_reasoning_segments(out: &mut Vec<Segment>, content: &str) {
    let mut rest = content;

    while let Some(start) = find_at_line_start(rest, OPEN, 0) {
        push_text(out, &rest[..start]);
        let after_open = &rest[start + OPEN.len()..];
        match find_at_line_start(after_open, CLOSE, 0) {
            Some(end) => {
                let inner = after_open[..end].trim();
                if !inner.is_empty() {
                    out.push(Segment::Reasoning(inner.to_string()));
                }
                rest = &after_open[end + CLOSE.len()..];
            }
            None => {
                let inner = after_open.trim();
                if !inner.is_empty() {
                    out.push(Segment::Reasoning(inner.to_string()));
                }
                rest = "";
                break;
            }
        }
    }

    push_text(out, rest);
}

/// Push visible text, minus any line that is nothing but a marker: an
/// orphaned close the model quoted, or the half of a pair a truncated write
/// left behind. Such a line is never content, and shown raw it reads as a
/// leak (#1587).
fn push_text(out: &mut Vec<Segment>, text: &str) {
    let kept: Vec<&str> = text
        .lines()
        .filter(|line| {
            let t = line.trim();
            !(t == OPEN || t == CLOSE || t == BLOCKED_OPEN || t == BLOCKED_CLOSE)
        })
        .collect();
    let joined = kept.join("\n");
    let trimmed = joined.trim();
    if !trimmed.is_empty() {
        out.push(Segment::Text(trimmed.to_string()));
    }
}

/// Whether the text segment at `i` is intermediate narration rather than the
/// turn's answer.
///
/// A text segment with reasoning after it is one the model kept thinking past,
/// so it cannot be the answer however much it reads like one. Only text with no
/// further reasoning behind it stays visible; the rest renders collapsed.
///
/// This matters because the model sometimes restates its reasoning through the
/// content channel, where it is persisted unwrapped and is otherwise
/// indistinguishable from the answer (#760). Position is the one signal that
/// stays exact, so it is the one used here: no guessing at which prose looks
/// like thinking.
///
/// A blocked section counts for nothing here: it is never the answer itself,
/// and the reasoning it folded in must not demote the real answer before it.
pub(crate) fn is_intermediate(segments: &[Segment], i: usize) -> bool {
    segments
        .iter()
        .rposition(|s| matches!(s, Segment::Reasoning(_)))
        .is_some_and(|last| i < last)
}
