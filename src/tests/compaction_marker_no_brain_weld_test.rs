//! The compaction marker carries the summary and nothing else (#1676).
//!
//! `apply_scoped_compaction_summary` used to weld a `[RECOVERED BRAIN
//! CONTEXT]` block (SOUL.md + USER.md + AGENTS.md, ~64KB on a real install)
//! onto the in-memory marker. The marker persisted to the DB was built from
//! the summary alone, so the two diverged by the size of the brain files: the
//! `/compact` confirmation reported 55,809 tokens and the next turn, reloading
//! the lean marker from the DB, settled at 39,935. The weld was a duplicate to
//! begin with — `prompt_builder` injects those three files into the system
//! prompt on every turn and `system_brain` survives compaction untouched — and
//! on the background path the welded context is reused for the rest of the
//! turn, so the duplicate was billed rather than discarded.

use crate::brain::agent::context::{AgentContext, COMPACTION_MARKER_PREFIX, CompactionScope};
use crate::brain::agent::service::AgentService;
use crate::brain::provider::{ContentBlock, Message};

const SUMMARY: &str = "## Summary\n\nThe user asked about token accounting.";

fn ctx_with_history() -> AgentContext {
    let mut c = AgentContext::new(uuid::Uuid::nil(), 200_000);
    for i in 0..6 {
        c.add_message(Message::user(format!("turn {i}")));
        c.add_message(Message::assistant(format!("reply {i}")));
    }
    c
}

fn text_of(msg: &Message) -> String {
    msg.content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn the_marker_body_is_the_summary_verbatim() {
    let mut c = ctx_with_history();
    AgentService::apply_scoped_compaction_summary(&mut c, CompactionScope::FullWindow, SUMMARY);

    let marker = text_of(&c.messages[0]);
    assert!(marker.starts_with(COMPACTION_MARKER_PREFIX));
    assert!(
        marker.ends_with(SUMMARY),
        "nothing may be appended after the summary: {marker}"
    );
    assert!(
        !marker.contains("RECOVERED BRAIN CONTEXT"),
        "brain files are in the system prompt every turn; welding them here \
         duplicates them and desyncs the in-memory marker from the DB one"
    );
    assert!(
        !marker.contains("No brain files found"),
        "the absent-brain-files placeholder went with the weld"
    );
}

#[test]
fn the_marker_costs_only_what_the_summary_costs() {
    let mut c = ctx_with_history();
    AgentService::apply_scoped_compaction_summary(&mut c, CompactionScope::FullWindow, SUMMARY);

    let marker = text_of(&c.messages[0]);
    assert!(
        marker.len() < SUMMARY.len() + 512,
        "marker is summary + one banner line, not summary + 64KB of brain files: \
         {} bytes",
        marker.len()
    );
}

#[test]
fn every_scope_keeps_the_body_clean() {
    for scope in [
        CompactionScope::FullWindow,
        CompactionScope::DeltaSinceMarker,
        CompactionScope::SegmentConsolidation,
    ] {
        let mut c = ctx_with_history();
        AgentService::apply_scoped_compaction_summary(&mut c, scope, SUMMARY);
        let welded = c
            .messages
            .iter()
            .any(|m| text_of(m).contains("RECOVERED BRAIN CONTEXT"));
        assert!(!welded, "{scope:?} welded brain files onto the marker");
    }
}
