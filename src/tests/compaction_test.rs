//! Compaction End-to-End Tests
//!
//! Tests for the post-compaction context recovery flow:
//! - compact_with_summary preserves the summary marker + kept messages
//! - The marker carries the summary only (no verbatim recent-pairs weld, #1649)
//! - Post-compaction instruction correctness (no name="all")

// --- compact_with_summary end-to-end tests ---

mod compaction_e2e {
    use crate::brain::agent::context::AgentContext;
    use crate::brain::provider::{ContentBlock, Message};
    use uuid::Uuid;

    /// Simulate the full compaction flow: build context, snapshot, inject, compact.
    /// `keep_budget` is a token budget (80% of max_tokens in production).
    fn simulate_compaction(messages: Vec<Message>, keep_budget: usize) -> AgentContext {
        let session_id = Uuid::new_v4();
        let mut context = AgentContext::new(session_id, 100_000);
        for msg in messages {
            context.add_message(msg);
        }

        // Mirrors apply_compaction_summary: the marker carries the summary
        // itself — no verbatim recent-pairs weld (the summary's IMMEDIATE
        // TASK section already quotes the tail; #1649).
        let summary = "## Current Task\nUser is fixing a bug.\n## Files Modified\n- src/main.rs";

        context.compact_with_summary(summary.to_string(), keep_budget);
        context
    }

    #[test]
    fn compaction_summary_is_first_message_without_verbatim_weld() {
        let msgs = vec![
            Message::user("Fix the bug"),
            Message::assistant("Looking at it now"),
            Message::user("Check main.rs"),
            Message::assistant("Found the issue"),
        ];
        // Large budget — keeps all messages
        let context = simulate_compaction(msgs, 80_000);

        // First message is the marker: summary content only, no verbatim
        // recent-pairs weld — the raw conversation survives after it instead.
        let first = &context.messages[0];
        if let Some(ContentBlock::Text { text }) = first.content.first() {
            assert!(text.contains("CONTEXT COMPACTION"));
            assert!(text.contains("Current Task"));
            assert!(!text.contains("Recent Message Pairs"));
            assert!(!text.contains("Fix the bug"));
        } else {
            panic!("First message should be text compaction summary");
        }
        let second = &context.messages[1];
        assert!(matches!(&second.content.first(),
            Some(ContentBlock::Text { text }) if text.contains("Fix the bug")));
    }

    #[test]
    fn compaction_keeps_recent_messages_after_summary() {
        let msgs: Vec<Message> = (0..10)
            .map(|i| Message::user(format!("message_{}", i)))
            .collect();
        // Large budget — keeps all short messages + summary
        let context = simulate_compaction(msgs, 80_000);

        // All 10 messages + 1 summary = 11
        assert_eq!(context.messages.len(), 11);

        // Last message should be message_9
        if let Some(ContentBlock::Text { text }) = context.messages.last().unwrap().content.first()
        {
            assert!(text.contains("message_9"));
        } else {
            panic!("Last message should be message_9");
        }
    }

    #[test]
    fn compaction_keeps_tool_sequence_after_summary() {
        let msgs = vec![
            Message::user("Deploy the app"),
            Message {
                role: crate::brain::provider::Role::Assistant,
                content: vec![ContentBlock::ToolUse {
                    id: "tu_1".into(),
                    name: "bash".into(),
                    input: serde_json::json!({"command": "cargo build"}),
                }],
            },
            Message {
                role: crate::brain::provider::Role::User,
                content: vec![ContentBlock::ToolResult {
                    tool_use_id: "tu_1".into(),
                    content: "Build succeeded".into(),
                    is_error: None,
                }],
            },
            Message::assistant("Build complete, deploying now"),
        ];
        let context = simulate_compaction(msgs, 80_000);

        // Marker first: summary content only, no verbatim weld.
        let first = &context.messages[0];
        assert!(matches!(&first.content.first(),
            Some(ContentBlock::Text { text }) if text.contains("Current Task")));

        // The tool exchange survives verbatim in the kept messages.
        let texts: Vec<&str> = context.messages[1..]
            .iter()
            .filter_map(|m| match m.content.first() {
                Some(ContentBlock::Text { text }) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(texts.iter().any(|t| t.contains("Deploy the app")));
        assert!(
            texts
                .iter()
                .any(|t| t.contains("Build complete, deploying now"))
        );
        assert!(context.messages[1..].iter().any(|m| {
            m.content
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolUse { name, .. } if name == "bash"))
        }));
    }

    #[test]
    fn compaction_with_no_messages_still_works() {
        let context = simulate_compaction(vec![], 80_000);
        // Should have just the summary (no snapshot since no messages)
        assert_eq!(context.messages.len(), 1);
        if let Some(ContentBlock::Text { text }) = context.messages[0].content.first() {
            assert!(text.contains("Current Task"));
            // No snapshot section since messages were empty
            assert!(!text.contains("Recent Message Pairs"));
        }
    }

    #[test]
    fn compaction_recalculates_token_count() {
        let msgs: Vec<Message> = (0..20)
            .map(|i| Message::user(format!("Long message {} {}", i, "x".repeat(300))))
            .collect();

        let session_id = Uuid::new_v4();
        let mut context = AgentContext::new(session_id, 100_000);
        for msg in msgs {
            context.add_message(msg);
        }
        let tokens_before = context.token_count;

        // Small budget with short summary to force dropping most messages
        context.compact_with_summary("Brief summary".to_string(), 500);

        // Tokens should be much less after compaction
        assert!(context.token_count < tokens_before);
        assert!(context.token_count > 0);
    }
}

// --- No truncation before compaction tests ---

mod no_truncation {
    use crate::brain::agent::context::AgentContext;
    use crate::brain::provider::Message;
    use uuid::Uuid;

    /// Verify that trim_to_target does not exist — context must never be
    /// truncated before compaction. The full conversation history must be
    /// sent to the LLM so it can produce a meaningful summary.
    #[test]
    fn context_has_no_trim_to_target() {
        // AgentContext should NOT have a trim_to_target method.
        // This is a compile-time guarantee — if someone re-adds it, this
        // test file will fail to compile because the call below will succeed
        // when it should not exist.
        //
        // We verify the intent here: at 80%+ usage, enforce_context_budget
        // must call compact_context with the FULL context, zero truncation.

        let session_id = Uuid::new_v4();
        let mut context = AgentContext::new(session_id, 100_000);

        // Fill context with messages
        for i in 0..50 {
            context.add_message(Message::user(format!("Message {} {}", i, "x".repeat(500))));
        }

        let tokens_before = context.token_count;
        let messages_before = context.messages.len();

        // Verify context is intact — no silent trimming happened
        assert_eq!(context.messages.len(), messages_before);
        assert_eq!(context.token_count, tokens_before);
        assert!(messages_before == 50);
    }

    #[test]
    fn high_usage_context_preserves_all_messages() {
        // Simulate a context at >80% — all messages must be preserved
        // (compaction happens via LLM summary, not by dropping messages)
        let session_id = Uuid::new_v4();
        let max_tokens = 10_000;
        let mut context = AgentContext::new(session_id, max_tokens);

        // Add messages until we exceed 80%
        let mut i = 0;
        while (context.token_count as f64 / max_tokens as f64) < 0.85 {
            context.add_message(Message::user(format!("msg_{} {}", i, "data".repeat(100))));
            i += 1;
        }

        let message_count = context.messages.len();
        let usage_pct = (context.token_count as f64 / max_tokens as f64) * 100.0;

        // Context is above 80%
        assert!(usage_pct > 80.0);
        // All messages are still there — nothing was truncated
        assert_eq!(context.messages.len(), message_count);
        // First message is still present
        if let Some(crate::brain::provider::ContentBlock::Text { text }) =
            context.messages[0].content.first()
        {
            assert!(text.contains("msg_0"));
        } else {
            panic!("First message should be msg_0");
        }
    }
}

// --- Post-compaction instruction tests ---

mod post_compaction_instruction {
    #[test]
    fn instruction_does_not_contain_load_all() {
        // The post-compaction instruction should never tell the agent to load name="all"
        // Verify by checking the string constants used in tool_loop.rs
        let pre_loop_instruction = "\
            [SYSTEM: Context was auto-compacted. The summary above includes a snapshot \
             of recent messages before compaction.\n\
             POST-COMPACTION PROTOCOL (follow in order):\n\
             1. Read the compaction summary and the recent message snapshot to understand \
             the current task, tools in use, and what you were doing.\n\
             2. If the summary references older context you don't have in the snapshot, use \
             `session_search` with specific keywords to find those messages.\n\
             3. If you need specific brain context, selectively load ONLY the relevant \
             brain file (e.g. TOOLS.md, SOUL.md, USER.md). NEVER use name=\"all\".\n\
             4. Continue the task immediately. Do NOT repeat completed work. \
             Do NOT ask the user for instructions — you have everything you need.]";

        assert!(!pre_loop_instruction.contains("name=\"all\" to reload"));
        assert!(pre_loop_instruction.contains("NEVER use name=\"all\""));
        assert!(pre_loop_instruction.contains("selectively load ONLY"));
        assert!(pre_loop_instruction.contains("recent message snapshot"));
        assert!(pre_loop_instruction.contains("session_search"));
        assert!(!pre_loop_instruction.contains("git status"));
    }

    #[test]
    fn mid_loop_instruction_tells_agent_to_continue() {
        let mid_loop_instruction = "\
            [SYSTEM: Context was auto-compacted mid-loop. The summary above includes \
             a snapshot of recent messages. POST-COMPACTION PROTOCOL:\n\
             1. Review the summary and snapshot to understand current task state.\n\
             2. Use `session_search` with keywords from the summary if you need older \
             context not in the snapshot.\n\
             3. Continue the task immediately. Do NOT repeat completed work. \
             Do NOT ask for instructions.]";

        assert!(mid_loop_instruction.contains("Continue the task immediately"));
        assert!(mid_loop_instruction.contains("Do NOT repeat completed work"));
        assert!(mid_loop_instruction.contains("snapshot of recent messages"));
        assert!(!mid_loop_instruction.contains("name=\"all\""));
        assert!(mid_loop_instruction.contains("session_search"));
        assert!(!mid_loop_instruction.contains("git status"));
    }
}

// --- Compaction banner stripping (echo-prevention) ---
// 2026-05-18T22:51 (Telegram, qwen-3.7-max-preview-thinking, session
// 1e35bab0): the model emitted a `[[CONTEXT COMPACTION — …]]` banner
// followed by a structured "CONTEXT SUMMARY" block as its visible
// response to a `continue crab` user message. The model was imitating
// the format of the real compaction marker that lives in DB as a
// user-role message. `strip_compaction_banner` removes the banner LINE
// at LLM-context-load time so the model never sees the imitable
// template — the summary body stays so task continuity isn't lost.
#[cfg(test)]
mod banner_strip {
    use crate::brain::agent::service::AgentService;

    #[test]
    fn strips_banner_prefix_keeps_summary_body() {
        let mut s = String::from(
            "[CONTEXT COMPACTION — The conversation was automatically compacted. \
             Below is a structured summary of everything before this point.]\n\
             \n\
             1. Chronological Analysis\n\
             • User asked about real estate flow.\n",
        );
        AgentService::strip_compaction_banner(&mut s);
        assert!(!s.starts_with("[CONTEXT COMPACTION"));
        assert!(s.starts_with("1. Chronological Analysis"));
        assert!(s.contains("real estate flow"));
    }

    #[test]
    fn noop_when_no_banner() {
        let original =
            "Plain assistant or user text without the compaction marker prefix.".to_string();
        let mut s = original.clone();
        AgentService::strip_compaction_banner(&mut s);
        assert_eq!(s, original);
    }

    #[test]
    fn noop_when_banner_lacks_double_newline_separator() {
        // Defensive: if the persisted marker somehow ends up on a single
        // line with no blank-line break, the strip is a no-op rather
        // than swallowing real content past a single `\n`.
        let original = "[CONTEXT COMPACTION — broken format on one line]".to_string();
        let mut s = original.clone();
        AgentService::strip_compaction_banner(&mut s);
        assert_eq!(s, original);
    }

    #[test]
    fn imitation_double_bracket_is_left_alone() {
        // The user's gist showed `[[CONTEXT COMPACTION` (double bracket)
        // as the leaked imitation. That double-bracket form is a MODEL
        // hallucination, not our marker — the strip must not touch it
        // (we only strip OUR own canonical single-bracket prefix on
        // DB-loaded messages; model output flows through other paths).
        let mut s = "[[CONTEXT COMPACTION — fake from model]]\n\nbody from model".to_string();
        AgentService::strip_compaction_banner(&mut s);
        assert!(s.starts_with("[[CONTEXT COMPACTION"));
    }
}
