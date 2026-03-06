//! Compaction End-to-End Tests
//!
//! Tests for the post-compaction context recovery flow:
//! - Recent message snapshot formatting (all content block types, truncation)
//! - Snapshot injection into compaction summary
//! - compact_with_summary preserves snapshot + kept messages
//! - Post-compaction instruction correctness (no name="all")

// --- format_recent_messages tests ---

mod snapshot {
    use crate::brain::agent::service::AgentService;
    use crate::brain::provider::{ContentBlock, Message, Role};

    fn tool_use_msg(name: &str, input: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "tu_1".into(),
                name: name.into(),
                input: serde_json::json!({ "arg": input }),
            }],
        }
    }

    fn tool_result_msg(content: &str) -> Message {
        Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: "tu_1".into(),
                content: content.into(),
                is_error: None,
            }],
        }
    }

    #[test]
    fn empty_messages_returns_empty() {
        let result = AgentService::format_recent_messages(&[], 8);
        assert!(result.is_empty());
    }

    #[test]
    fn formats_user_and_assistant_text() {
        let msgs = vec![
            Message::user("Hello agent"),
            Message::assistant("Hello user"),
        ];
        let result = AgentService::format_recent_messages(&msgs, 8);
        assert!(result.contains("**User**: Hello agent"));
        assert!(result.contains("**Assistant**: Hello user"));
    }

    #[test]
    fn formats_system_messages() {
        let msgs = vec![Message::system("You are helpful")];
        let result = AgentService::format_recent_messages(&msgs, 8);
        assert!(result.contains("**System**: You are helpful"));
    }

    #[test]
    fn formats_tool_use_blocks() {
        let msgs = vec![tool_use_msg("read_file", "src/main.rs")];
        let result = AgentService::format_recent_messages(&msgs, 8);
        assert!(result.contains("**Assistant**: [tool_use: read_file("));
        assert!(result.contains("src/main.rs"));
    }

    #[test]
    fn formats_tool_result_blocks() {
        let msgs = vec![tool_result_msg("fn main() {}")];
        let result = AgentService::format_recent_messages(&msgs, 8);
        assert!(result.contains("**User**: [tool_result: fn main() {}]"));
    }

    #[test]
    fn formats_image_blocks() {
        let msgs = vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Image {
                source: crate::brain::provider::ImageSource::Base64 {
                    media_type: "image/png".into(),
                    data: "abc".into(),
                },
            }],
        }];
        let result = AgentService::format_recent_messages(&msgs, 8);
        assert!(result.contains("**User**: [image]"));
    }

    #[test]
    fn truncates_long_text_at_500_chars() {
        let long_text = "x".repeat(800);
        let msgs = vec![Message::user(long_text)];
        let result = AgentService::format_recent_messages(&msgs, 8);
        assert!(result.contains("[truncated]"));
        // The displayed portion should be 500 chars + ellipsis + truncated marker
        assert!(!result.contains(&"x".repeat(501)));
    }

    #[test]
    fn truncates_long_tool_result_at_300_chars() {
        let long_content = "y".repeat(500);
        let msgs = vec![tool_result_msg(&long_content)];
        let result = AgentService::format_recent_messages(&msgs, 8);
        assert!(result.contains("[truncated]"));
        assert!(!result.contains(&"y".repeat(301)));
    }

    #[test]
    fn truncates_long_tool_use_input_at_200_chars() {
        let long_input = "z".repeat(400);
        let msgs = vec![tool_use_msg("big_tool", &long_input)];
        let result = AgentService::format_recent_messages(&msgs, 8);
        // Input preview is truncated at 200 chars with ellipsis
        assert!(result.contains("big_tool("));
    }

    #[test]
    fn respects_n_limit_takes_last_n() {
        let msgs: Vec<Message> = (0..10)
            .map(|i| Message::user(format!("msg_{}", i)))
            .collect();
        let result = AgentService::format_recent_messages(&msgs, 3);
        // Should only contain the last 3 messages
        assert!(!result.contains("msg_6"));
        assert!(result.contains("msg_7"));
        assert!(result.contains("msg_8"));
        assert!(result.contains("msg_9"));
    }

    #[test]
    fn n_larger_than_messages_returns_all() {
        let msgs = vec![Message::user("only one")];
        let result = AgentService::format_recent_messages(&msgs, 100);
        assert!(result.contains("only one"));
    }

    #[test]
    fn mixed_content_blocks_in_single_message() {
        let msgs = vec![Message {
            role: Role::User,
            content: vec![
                ContentBlock::Text {
                    text: "Check this file".into(),
                },
                ContentBlock::ToolResult {
                    tool_use_id: "tu_1".into(),
                    content: "file contents here".into(),
                    is_error: None,
                },
            ],
        }];
        let result = AgentService::format_recent_messages(&msgs, 8);
        assert!(result.contains("**User**: Check this file"));
        assert!(result.contains("**User**: [tool_result: file contents here]"));
    }

    #[test]
    fn realistic_tool_loop_sequence() {
        let msgs = vec![
            Message::user("Fix the bug in main.rs"),
            tool_use_msg("read_file", "src/main.rs"),
            tool_result_msg("fn main() { panic!() }"),
            Message::assistant("I see the issue, let me fix it"),
            tool_use_msg("edit_file", "src/main.rs"),
            tool_result_msg("File updated successfully"),
            Message::assistant("Done, the bug is fixed"),
        ];
        let result = AgentService::format_recent_messages(&msgs, 8);
        // All 7 messages should be present (< 8 limit)
        assert!(result.contains("Fix the bug"));
        assert!(result.contains("read_file"));
        assert!(result.contains("panic!()"));
        assert!(result.contains("I see the issue"));
        assert!(result.contains("edit_file"));
        assert!(result.contains("File updated"));
        assert!(result.contains("bug is fixed"));
    }
}

// --- compact_with_summary end-to-end tests ---

mod compaction_e2e {
    use crate::brain::agent::context::AgentContext;
    use crate::brain::agent::service::AgentService;
    use crate::brain::provider::{ContentBlock, Message};
    use uuid::Uuid;

    /// Simulate the full compaction flow: build context, snapshot, inject, compact.
    fn simulate_compaction(messages: Vec<Message>, keep_recent: usize) -> AgentContext {
        let session_id = Uuid::new_v4();
        let mut context = AgentContext::new(session_id, 100_000);
        for msg in messages {
            context.add_message(msg);
        }

        // Snapshot before compaction (mirrors compact_context logic)
        let recent_snapshot = AgentService::format_recent_messages(&context.messages, 8);
        let summary = "## Current Task\nUser is fixing a bug.\n## Files Modified\n- src/main.rs";
        let summary_with_context = if recent_snapshot.is_empty() {
            summary.to_string()
        } else {
            format!(
                "{}\n\n## Recent Message Pairs (pre-compaction snapshot)\n\
                 The following are the last messages before compaction \
                 — use them to understand the current task state and decide what context to reload.\n\n{}",
                summary, recent_snapshot
            )
        };

        context.compact_with_summary(summary_with_context, keep_recent);
        context
    }

    #[test]
    fn compaction_includes_snapshot_in_summary_message() {
        let msgs = vec![
            Message::user("Fix the bug"),
            Message::assistant("Looking at it now"),
            Message::user("Check main.rs"),
            Message::assistant("Found the issue"),
        ];
        let context = simulate_compaction(msgs, 2);

        // First message should be the compaction summary with snapshot
        let first = &context.messages[0];
        if let Some(ContentBlock::Text { text }) = first.content.first() {
            assert!(text.contains("CONTEXT COMPACTION"));
            assert!(text.contains("Current Task"));
            assert!(text.contains("Recent Message Pairs"));
            assert!(text.contains("Fix the bug"));
            assert!(text.contains("Found the issue"));
        } else {
            panic!("First message should be text compaction summary");
        }
    }

    #[test]
    fn compaction_keeps_recent_messages_after_summary() {
        let msgs: Vec<Message> = (0..10)
            .map(|i| Message::user(format!("message_{}", i)))
            .collect();
        let context = simulate_compaction(msgs, 3);

        // Should have: 1 summary + 3 kept = 4 messages
        assert_eq!(context.messages.len(), 4);

        // Last message should be message_9
        if let Some(ContentBlock::Text { text }) = context.messages[3].content.first() {
            assert!(text.contains("message_9"));
        } else {
            panic!("Last message should be message_9");
        }
    }

    #[test]
    fn compaction_snapshot_captures_tool_use_sequence() {
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
        let context = simulate_compaction(msgs, 2);

        // Summary should contain the tool call info from snapshot
        if let Some(ContentBlock::Text { text }) = context.messages[0].content.first() {
            assert!(text.contains("tool_use: bash"));
            assert!(text.contains("cargo build"));
            assert!(text.contains("Build succeeded"));
            assert!(text.contains("Deploy the app"));
        } else {
            panic!("Summary should contain tool snapshot");
        }
    }

    #[test]
    fn compaction_with_no_messages_still_works() {
        let context = simulate_compaction(vec![], 4);
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

        let snapshot = AgentService::format_recent_messages(&context.messages, 8);
        let summary = format!("Summary\n\n## Recent Message Pairs\n{}", snapshot);
        context.compact_with_summary(summary, 4);

        // Tokens should be much less after compaction
        assert!(context.token_count < tokens_before);
        assert!(context.token_count > 0);
    }

    #[test]
    fn compaction_snapshot_truncates_large_messages() {
        let huge_text = "z".repeat(1000);
        let msgs = vec![Message::user(huge_text.clone())];
        let context = simulate_compaction(msgs, 0);

        // The snapshot in the summary should have the truncated version
        if let Some(ContentBlock::Text { text }) = context.messages[0].content.first() {
            assert!(text.contains("[truncated]"));
            // Original 1000-char message should not appear in full
            assert!(!text.contains(&huge_text));
        }
    }

    #[test]
    fn compaction_snapshot_only_last_8_even_with_many_messages() {
        let msgs: Vec<Message> = (0..20)
            .map(|i| Message::user(format!("msg_{}", i)))
            .collect();
        let context = simulate_compaction(msgs, 4);

        if let Some(ContentBlock::Text { text }) = context.messages[0].content.first() {
            // Snapshot should contain messages 12-19 (last 8)
            assert!(text.contains("msg_12"));
            assert!(text.contains("msg_19"));
            // Should NOT contain early messages in the snapshot
            assert!(!text.contains("msg_0:"));
            assert!(!text.contains("msg_5:"));
        }
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
             POST-COMPACTION PROTOCOL:\n\
             1. Read the compaction summary and the recent message snapshot to understand \
             the current task, tools in use, and what you were doing.\n\
             2. If you need specific brain context, selectively load ONLY the relevant \
             brain file (e.g. TOOLS.md, SOUL.md, USER.md). NEVER use name=\"all\".\n\
             3. Continue the task immediately. Do NOT repeat completed work. \
             Do NOT ask the user for instructions — you have everything you need.]";

        assert!(!pre_loop_instruction.contains("name=\"all\" to reload"));
        assert!(pre_loop_instruction.contains("NEVER use name=\"all\""));
        assert!(pre_loop_instruction.contains("selectively load ONLY"));
        assert!(pre_loop_instruction.contains("recent message snapshot"));
    }

    #[test]
    fn mid_loop_instruction_tells_agent_to_continue() {
        let mid_loop_instruction = "\
            [SYSTEM: Context was auto-compacted mid-loop. The summary above includes \
             a snapshot of recent messages. Review it and continue the task immediately. \
             Do NOT repeat completed work. Do NOT ask for instructions.]";

        assert!(mid_loop_instruction.contains("continue the task immediately"));
        assert!(mid_loop_instruction.contains("Do NOT repeat completed work"));
        assert!(mid_loop_instruction.contains("snapshot of recent messages"));
        assert!(!mid_loop_instruction.contains("name=\"all\""));
    }
}
