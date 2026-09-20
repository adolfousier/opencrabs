//! Auto-compaction runs in the background, and the two things that makes
//! dangerous are covered here.
//!
//! 1. **The gap.** A summariser that blocks the turn cannot miss anything:
//!    nothing arrives while it runs. A backgrounded one leaves a gap, and
//!    everything in that gap is the most recent work the agent did. The swap
//!    has to keep it.
//! 2. **The ceiling.** When the context outgrows its headroom the turn waits
//!    for the summary instead of discarding it. Cancelling to reclaim room is
//!    what left a truncated context with no marker and looped two sessions on
//!    reload (2026-05-05), so the predicate that decides to wait is asserted
//!    directly rather than inferred.

use crate::brain::agent::context::{AgentContext, CompactionScope};
use crate::brain::agent::service::AgentService;
use crate::brain::agent::service::compaction::{BudgetPhase, must_wait_for_compaction};
use crate::brain::provider::{ContentBlock, Message, Role};

fn ctx(messages: Vec<Message>) -> AgentContext {
    let mut c = AgentContext::new(uuid::Uuid::nil(), 200_000);
    for m in messages {
        c.add_message(m);
    }
    c
}

fn tool_use(id: &str) -> Message {
    Message {
        role: Role::Assistant,
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: "bash".into(),
            input: serde_json::json!({ "command": "ls" }),
        }],
    }
}

fn tool_result(id: &str, body: &str) -> Message {
    Message {
        role: Role::User,
        content: vec![ContentBlock::ToolResult {
            tool_use_id: id.into(),
            content: body.into(),
            is_error: None,
        }],
    }
}

fn text_of(msg: &Message) -> String {
    msg.content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.clone()),
            ContentBlock::ToolResult { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// --- the gap ---

#[test]
fn work_done_during_the_summariser_call_survives_the_swap() {
    // Snapshot taken after two messages; the turn kept going and produced
    // three more while the summary was being written.
    let mut context = ctx(vec![
        Message::user("old question"),
        Message::assistant("old answer"),
        Message::user("new question"),
        tool_use("tu_1"),
        tool_result("tu_1", "the command output"),
    ]);

    AgentService::apply_compaction_summary_after(
        &mut context,
        CompactionScope::FullWindow,
        "SUMMARY BODY",
        2,
    );

    let rendered: Vec<String> = context.messages.iter().map(text_of).collect();
    assert!(
        rendered[0].contains("SUMMARY BODY"),
        "summary is not the anchor: {rendered:?}"
    );
    assert!(
        rendered.iter().any(|t| t.contains("new question")),
        "the turn's own question was deleted: {rendered:?}"
    );
    assert!(
        rendered.iter().any(|t| t.contains("the command output")),
        "a tool result the summary never saw was deleted: {rendered:?}"
    );
}

#[test]
fn the_summarised_prefix_does_not_survive_as_messages() {
    let mut context = ctx(vec![
        Message::user("old question"),
        Message::assistant("old answer"),
        Message::user("new question"),
    ]);

    AgentService::apply_compaction_summary_after(
        &mut context,
        CompactionScope::FullWindow,
        "SUMMARY BODY",
        2,
    );

    // Summary plus the one message the summariser never saw. The pair it did
    // see survives only as prose quoted inside the summary, which is the
    // whole point of compacting them.
    assert_eq!(
        context.messages.len(),
        2,
        "compaction kept what it just summarised as live messages"
    );
    assert_eq!(text_of(&context.messages[1]), "new question");
}

#[test]
fn a_delta_opening_on_tool_results_drops_the_orphans() {
    // The snapshot cut between an assistant tool_use and its results. The
    // summary lands as a user message, so those results now have no call to
    // belong to and the provider rejects the shape outright.
    let mut context = ctx(vec![
        Message::user("question"),
        tool_use("tu_1"),
        tool_result("tu_1", "orphaned output"),
        Message::assistant("carrying on"),
    ]);

    AgentService::apply_compaction_summary_after(
        &mut context,
        CompactionScope::FullWindow,
        "SUMMARY BODY",
        2,
    );

    assert!(
        !AgentContext::is_orphaned_tool_result_msg(&context.messages[1]),
        "an orphaned tool result was left directly after the summary"
    );
    let rendered: Vec<String> = context.messages.iter().map(text_of).collect();
    assert!(
        rendered.iter().any(|t| t.contains("carrying on")),
        "everything after the orphan was thrown away too: {rendered:?}"
    );
}

#[test]
fn an_all_orphan_delta_leaves_just_the_summary() {
    let mut context = ctx(vec![
        Message::user("question"),
        tool_use("tu_1"),
        tool_result("tu_1", "one"),
        tool_result("tu_1", "two"),
    ]);

    AgentService::apply_compaction_summary_after(
        &mut context,
        CompactionScope::FullWindow,
        "SUMMARY BODY",
        2,
    );

    assert_eq!(context.messages.len(), 1);
    assert!(text_of(&context.messages[0]).contains("SUMMARY BODY"));
}

#[test]
fn no_delta_behaves_like_a_blocking_compaction() {
    let mut context = ctx(vec![
        Message::user("question"),
        Message::assistant("answer"),
    ]);
    let len = context.messages.len();

    AgentService::apply_compaction_summary_after(
        &mut context,
        CompactionScope::FullWindow,
        "SUMMARY BODY",
        len,
    );

    assert_eq!(context.messages.len(), 1);
    assert!(text_of(&context.messages[0]).contains("SUMMARY BODY"));
}

#[test]
fn a_snapshot_that_outlived_its_context_still_applies() {
    // The context is rebuilt from the database every turn, so an index from a
    // previous turn addresses a vector that no longer exists. There is no
    // delta to recover, but the summary must still land rather than panic.
    let mut context = ctx(vec![Message::user("only message")]);

    AgentService::apply_compaction_summary_after(
        &mut context,
        CompactionScope::FullWindow,
        "SUMMARY BODY",
        99,
    );

    assert_eq!(context.messages.len(), 1);
    assert!(text_of(&context.messages[0]).contains("SUMMARY BODY"));
}

#[test]
fn the_budget_counts_the_delta_it_kept() {
    let mut context = ctx(vec![
        Message::user("old question"),
        Message::assistant("old answer"),
        Message::user("a considerably longer message that carries real weight in the budget"),
    ]);

    AgentService::apply_compaction_summary_after(
        &mut context,
        CompactionScope::FullWindow,
        "SUMMARY BODY",
        2,
    );

    let recomputed: usize = context
        .messages
        .iter()
        .map(|m| context.estimate_message_tokens(m))
        .sum::<usize>()
        + context
            .system_brain
            .as_deref()
            .map(AgentContext::estimate_tokens)
            .unwrap_or(0);
    assert_eq!(
        context.token_count, recomputed,
        "kept messages are in the context but not in its budget"
    );
}

// --- the ceiling ---

#[test]
fn a_turn_about_to_answer_always_waits() {
    // Answering from a context one swap away from replacement is the case the
    // owner called out: the reply would be composed against history that is
    // about to stop existing.
    for usage in [10.0, 66.0, 79.9, 95.0] {
        assert!(
            must_wait_for_compaction(BudgetPhase::TurnStart, usage),
            "turn start ran ahead of the summariser at {usage}%"
        );
    }
}

#[test]
fn the_tool_loop_runs_ahead_below_the_ceiling() {
    for usage in [10.0, 66.0, 79.9] {
        assert!(
            !must_wait_for_compaction(BudgetPhase::MidLoop, usage),
            "the loop blocked at {usage}%, which is the stall we removed"
        );
    }
}

#[test]
fn the_tool_loop_waits_at_the_ceiling() {
    for usage in [80.0, 91.0, 140.0] {
        assert!(
            must_wait_for_compaction(BudgetPhase::MidLoop, usage),
            "the loop kept growing the context at {usage}%"
        );
    }
}
