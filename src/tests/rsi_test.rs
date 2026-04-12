//! Recursive Self-Improvement (RSI) Tests
//!
//! Comprehensive tests for the feedback ledger repository, feedback_record tool,
//! feedback_analyze tool, and self_improve tool.

// --- Feedback Ledger Repository Tests ---

mod feedback_ledger_repo {
    use crate::db::Database;
    use crate::db::repository::FeedbackLedgerRepository;

    async fn setup() -> (Database, FeedbackLedgerRepository) {
        let db = Database::connect_in_memory()
            .await
            .expect("in-memory DB");
        db.run_migrations().await.expect("migrations");
        let repo = FeedbackLedgerRepository::new(db.pool().clone());
        (db, repo)
    }

    #[tokio::test]
    async fn record_and_count() {
        let (_db, repo) = setup().await;
        assert_eq!(repo.total_count().await.unwrap(), 0);

        let id = repo
            .record("sess1", "tool_success", "bash", 1.0, None)
            .await
            .unwrap();
        assert!(id > 0);
        assert_eq!(repo.total_count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn record_with_metadata() {
        let (_db, repo) = setup().await;
        let id = repo
            .record(
                "sess1",
                "tool_failure",
                "edit",
                0.0,
                Some(r#"{"error":"file not found"}"#),
            )
            .await
            .unwrap();
        assert!(id > 0);

        let entries = repo.recent(10).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].event_type, "tool_failure");
        assert_eq!(entries[0].dimension, "edit");
        assert!(entries[0]
            .metadata
            .as_deref()
            .unwrap()
            .contains("file not found"));
    }

    #[tokio::test]
    async fn recent_returns_latest() {
        let (_db, repo) = setup().await;
        for i in 0..5 {
            repo.record("sess1", "tool_success", &format!("tool_{i}"), 1.0, None)
                .await
                .unwrap();
        }

        let entries = repo.recent(3).await.unwrap();
        assert_eq!(entries.len(), 3);
        // Should return some subset of the 5 entries (ordering by created_at DESC, rowid tiebreak)
        // All entries should be from our set
        for e in &entries {
            assert!(e.dimension.starts_with("tool_"));
        }
    }

    #[tokio::test]
    async fn recent_respects_limit() {
        let (_db, repo) = setup().await;
        for i in 0..10 {
            repo.record("sess1", "tool_success", &format!("t{i}"), 1.0, None)
                .await
                .unwrap();
        }
        let entries = repo.recent(5).await.unwrap();
        assert_eq!(entries.len(), 5);
    }

    #[tokio::test]
    async fn by_event_type_filters() {
        let (_db, repo) = setup().await;
        repo.record("s1", "tool_success", "bash", 1.0, None)
            .await
            .unwrap();
        repo.record("s1", "tool_failure", "edit", 0.0, None)
            .await
            .unwrap();
        repo.record("s1", "tool_success", "read", 1.0, None)
            .await
            .unwrap();
        repo.record("s1", "user_correction", "tone", 1.0, None)
            .await
            .unwrap();

        let successes = repo.by_event_type("tool_success", 50).await.unwrap();
        assert_eq!(successes.len(), 2);
        for e in &successes {
            assert_eq!(e.event_type, "tool_success");
        }

        let failures = repo.by_event_type("tool_failure", 50).await.unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].dimension, "edit");

        let corrections = repo.by_event_type("user_correction", 50).await.unwrap();
        assert_eq!(corrections.len(), 1);
    }

    #[tokio::test]
    async fn by_event_type_empty() {
        let (_db, repo) = setup().await;
        let entries = repo.by_event_type("nonexistent", 50).await.unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn stats_by_dimension() {
        let (_db, repo) = setup().await;
        // bash: 3 success, 1 failure
        for _ in 0..3 {
            repo.record("s1", "tool_success", "bash", 1.0, None)
                .await
                .unwrap();
        }
        repo.record("s1", "tool_failure", "bash", 0.0, None)
            .await
            .unwrap();
        // edit: 1 success, 2 failures
        repo.record("s1", "tool_success", "edit", 1.0, None)
            .await
            .unwrap();
        for _ in 0..2 {
            repo.record("s1", "tool_failure", "edit", 0.0, None)
                .await
                .unwrap();
        }

        let stats = repo.stats_by_dimension("tool_").await.unwrap();
        assert_eq!(stats.len(), 2);

        // bash has more total events, should be first
        let bash = &stats[0];
        assert_eq!(bash.dimension, "bash");
        assert_eq!(bash.total_events, 4);
        assert_eq!(bash.successes, 3);
        assert_eq!(bash.failures, 1);
        assert!((bash.success_rate - 0.75).abs() < 0.01);

        let edit = &stats[1];
        assert_eq!(edit.dimension, "edit");
        assert_eq!(edit.total_events, 3);
        assert_eq!(edit.successes, 1);
        assert_eq!(edit.failures, 2);
        assert!((edit.success_rate - 1.0 / 3.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn stats_by_dimension_empty() {
        let (_db, repo) = setup().await;
        let stats = repo.stats_by_dimension("tool_").await.unwrap();
        assert!(stats.is_empty());
    }

    #[tokio::test]
    async fn summary_groups_by_event_type() {
        let (_db, repo) = setup().await;
        repo.record("s1", "tool_success", "bash", 1.0, None)
            .await
            .unwrap();
        repo.record("s1", "tool_success", "read", 1.0, None)
            .await
            .unwrap();
        repo.record("s1", "tool_failure", "edit", 0.0, None)
            .await
            .unwrap();
        repo.record("s1", "user_correction", "tone", 1.0, None)
            .await
            .unwrap();

        let summary = repo.summary().await.unwrap();
        assert_eq!(summary.len(), 3);
        // Ordered by count DESC
        assert_eq!(summary[0].0, "tool_success");
        assert_eq!(summary[0].1, 2);
    }

    #[tokio::test]
    async fn summary_empty_ledger() {
        let (_db, repo) = setup().await;
        let summary = repo.summary().await.unwrap();
        assert!(summary.is_empty());
    }

    #[tokio::test]
    async fn count_since_filters_by_date() {
        let (_db, repo) = setup().await;
        repo.record("s1", "tool_success", "bash", 1.0, None)
            .await
            .unwrap();

        // All events should be "since" a long time ago
        let count = repo.count_since("2000-01-01T00:00:00Z").await.unwrap();
        assert_eq!(count, 1);

        // None should be "since" a future date
        let count = repo.count_since("2099-01-01T00:00:00Z").await.unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn multiple_sessions() {
        let (_db, repo) = setup().await;
        repo.record("sess_a", "tool_success", "bash", 1.0, None)
            .await
            .unwrap();
        repo.record("sess_b", "tool_failure", "bash", 0.0, None)
            .await
            .unwrap();

        assert_eq!(repo.total_count().await.unwrap(), 2);
        let entries = repo.recent(10).await.unwrap();
        let sessions: Vec<&str> = entries.iter().map(|e| e.session_id.as_str()).collect();
        assert!(sessions.contains(&"sess_a"));
        assert!(sessions.contains(&"sess_b"));
    }

    #[tokio::test]
    async fn value_preserved() {
        let (_db, repo) = setup().await;
        repo.record("s1", "context_compaction", "tokens", 4096.0, None)
            .await
            .unwrap();
        let entries = repo.recent(1).await.unwrap();
        assert!((entries[0].value - 4096.0).abs() < 0.01);
    }
}

// --- Feedback Record Tool Tests ---

mod feedback_record_tool {
    use crate::brain::tools::feedback_record::FeedbackRecordTool;
    use crate::brain::tools::{Tool, ToolExecutionContext};
    use crate::db::Database;
    use crate::services::ServiceContext;
    use serde_json::json;
    use uuid::Uuid;

    async fn setup() -> (Database, ToolExecutionContext) {
        let db = Database::connect_in_memory()
            .await
            .expect("in-memory DB");
        db.run_migrations().await.expect("migrations");
        let svc = ServiceContext::new(db.pool().clone());
        let mut ctx = ToolExecutionContext::new(Uuid::new_v4());
        ctx.service_context = Some(svc);
        (db, ctx)
    }

    #[test]
    fn tool_metadata() {
        let tool = FeedbackRecordTool;
        assert_eq!(tool.name(), "feedback_record");
        assert!(!tool.requires_approval());
        assert!(tool.capabilities().is_empty());
        let schema = tool.input_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "event_type"));
        assert!(required.iter().any(|v| v == "dimension"));
    }

    #[tokio::test]
    async fn record_success() {
        let (_db, ctx) = setup().await;
        let tool = FeedbackRecordTool;
        let result = tool
            .execute(
                json!({
                    "event_type": "tool_success",
                    "dimension": "bash",
                    "value": 1.0
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("Recorded feedback"));
        assert!(result.output.contains("tool_success/bash"));
    }

    #[tokio::test]
    async fn record_with_metadata() {
        let (_db, ctx) = setup().await;
        let tool = FeedbackRecordTool;
        let result = tool
            .execute(
                json!({
                    "event_type": "tool_failure",
                    "dimension": "edit",
                    "value": 0.0,
                    "metadata": "file was read-only"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("tool_failure/edit"));
    }

    #[tokio::test]
    async fn record_default_value() {
        let (_db, ctx) = setup().await;
        let tool = FeedbackRecordTool;
        let result = tool
            .execute(
                json!({
                    "event_type": "pattern_observed",
                    "dimension": "user_prefers_concise"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.success);
        // Default value is 1.0
        assert!(result.output.contains("= 1"));
    }

    #[tokio::test]
    async fn record_missing_event_type() {
        let (_db, ctx) = setup().await;
        let tool = FeedbackRecordTool;
        let result = tool
            .execute(
                json!({
                    "dimension": "bash"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.success);
        let err = result.error.as_deref().unwrap();
        assert!(err.contains("required"));
    }

    #[tokio::test]
    async fn record_missing_dimension() {
        let (_db, ctx) = setup().await;
        let tool = FeedbackRecordTool;
        let result = tool
            .execute(
                json!({
                    "event_type": "tool_success"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.success);
        let err = result.error.as_deref().unwrap();
        assert!(err.contains("required"));
    }

    #[tokio::test]
    async fn record_empty_strings() {
        let (_db, ctx) = setup().await;
        let tool = FeedbackRecordTool;
        let result = tool
            .execute(
                json!({
                    "event_type": "",
                    "dimension": ""
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn record_no_service_context() {
        let ctx = ToolExecutionContext::new(Uuid::new_v4());
        let tool = FeedbackRecordTool;
        let result = tool
            .execute(
                json!({
                    "event_type": "tool_success",
                    "dimension": "bash"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.success);
        let err = result.error.as_deref().unwrap();
        assert!(err.contains("database"));
    }
}

// --- Feedback Analyze Tool Tests ---

mod feedback_analyze_tool {
    use crate::brain::tools::feedback_analyze::FeedbackAnalyzeTool;
    use crate::brain::tools::{Tool, ToolExecutionContext};
    use crate::db::Database;
    use crate::db::repository::FeedbackLedgerRepository;
    use crate::services::ServiceContext;
    use serde_json::json;
    use uuid::Uuid;

    async fn setup() -> (Database, ToolExecutionContext, FeedbackLedgerRepository) {
        let db = Database::connect_in_memory()
            .await
            .expect("in-memory DB");
        db.run_migrations().await.expect("migrations");
        let repo = FeedbackLedgerRepository::new(db.pool().clone());
        let svc = ServiceContext::new(db.pool().clone());
        let mut ctx = ToolExecutionContext::new(Uuid::new_v4());
        ctx.service_context = Some(svc);
        (db, ctx, repo)
    }

    /// Helper to get the text from a ToolResult (output for success, error for failure)
    fn result_text(result: &crate::brain::tools::ToolResult) -> &str {
        if result.success {
            &result.output
        } else {
            result.error.as_deref().unwrap_or("")
        }
    }

    #[test]
    fn tool_metadata() {
        let tool = FeedbackAnalyzeTool;
        assert_eq!(tool.name(), "feedback_analyze");
        assert!(!tool.requires_approval());
        assert!(tool.capabilities().is_empty());
    }

    #[tokio::test]
    async fn summary_empty_ledger() {
        let (_db, ctx, _repo) = setup().await;
        let tool = FeedbackAnalyzeTool;
        let result = tool
            .execute(json!({"query": "summary"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("No feedback data yet"));
    }

    #[tokio::test]
    async fn summary_with_data() {
        let (_db, ctx, repo) = setup().await;
        repo.record("s1", "tool_success", "bash", 1.0, None)
            .await
            .unwrap();
        repo.record("s1", "tool_failure", "edit", 0.0, None)
            .await
            .unwrap();
        repo.record("s1", "tool_success", "read", 1.0, None)
            .await
            .unwrap();

        let tool = FeedbackAnalyzeTool;
        let result = tool
            .execute(json!({"query": "summary"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("3 total events"));
        assert!(result.output.contains("tool_success"));
        assert!(result.output.contains("tool_failure"));
    }

    #[tokio::test]
    async fn tool_stats_empty() {
        let (_db, ctx, _repo) = setup().await;
        let tool = FeedbackAnalyzeTool;
        let result = tool
            .execute(json!({"query": "tool_stats"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("No tool execution data"));
    }

    #[tokio::test]
    async fn tool_stats_with_data() {
        let (_db, ctx, repo) = setup().await;
        for _ in 0..3 {
            repo.record("s1", "tool_success", "bash", 1.0, None)
                .await
                .unwrap();
        }
        repo.record("s1", "tool_failure", "bash", 0.0, None)
            .await
            .unwrap();

        let tool = FeedbackAnalyzeTool;
        let result = tool
            .execute(json!({"query": "tool_stats"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("bash"));
        assert!(result.output.contains("75.0%"));
    }

    #[tokio::test]
    async fn recent_empty() {
        let (_db, ctx, _repo) = setup().await;
        let tool = FeedbackAnalyzeTool;
        let result = tool
            .execute(json!({"query": "recent"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("No recent feedback"));
    }

    #[tokio::test]
    async fn recent_with_data() {
        let (_db, ctx, repo) = setup().await;
        repo.record("s1", "tool_success", "bash", 1.0, Some("ran ls"))
            .await
            .unwrap();

        let tool = FeedbackAnalyzeTool;
        let result = tool
            .execute(json!({"query": "recent", "limit": 10}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("1 entries"));
        assert!(result.output.contains("tool_success"));
        assert!(result.output.contains("bash"));
    }

    #[tokio::test]
    async fn recent_respects_limit() {
        let (_db, ctx, repo) = setup().await;
        for i in 0..10 {
            repo.record("s1", "tool_success", &format!("t{i}"), 1.0, None)
                .await
                .unwrap();
        }

        let tool = FeedbackAnalyzeTool;
        let result = tool
            .execute(json!({"query": "recent", "limit": 3}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("3 entries"));
    }

    #[tokio::test]
    async fn failures_empty() {
        let (_db, ctx, _repo) = setup().await;
        let tool = FeedbackAnalyzeTool;
        let result = tool
            .execute(json!({"query": "failures"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("No tool failures"));
    }

    #[tokio::test]
    async fn failures_with_data() {
        let (_db, ctx, repo) = setup().await;
        repo.record("s1", "tool_success", "bash", 1.0, None)
            .await
            .unwrap();
        repo.record(
            "s1",
            "tool_failure",
            "edit",
            0.0,
            Some("permission denied"),
        )
        .await
        .unwrap();

        let tool = FeedbackAnalyzeTool;
        let result = tool
            .execute(json!({"query": "failures"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("1 entries"));
        assert!(result.output.contains("edit"));
        assert!(result.output.contains("permission denied"));
    }

    #[tokio::test]
    async fn unknown_query_type() {
        let (_db, ctx, _repo) = setup().await;
        let tool = FeedbackAnalyzeTool;
        let result = tool
            .execute(json!({"query": "bogus"}), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result_text(&result).contains("Unknown query type"));
    }

    #[tokio::test]
    async fn no_service_context() {
        let ctx = ToolExecutionContext::new(Uuid::new_v4());
        let tool = FeedbackAnalyzeTool;
        let result = tool
            .execute(json!({"query": "summary"}), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result_text(&result).contains("database"));
    }
}

// --- Self-Improve Tool Tests ---

mod self_improve_tool {
    use crate::brain::tools::self_improve::SelfImproveTool;
    use crate::brain::tools::{Tool, ToolExecutionContext};
    use crate::db::Database;
    use crate::services::ServiceContext;
    use serde_json::json;
    use uuid::Uuid;

    fn result_text(result: &crate::brain::tools::ToolResult) -> &str {
        if result.success {
            &result.output
        } else {
            result.error.as_deref().unwrap_or("")
        }
    }

    fn setup_ctx_no_db() -> ToolExecutionContext {
        let mut ctx = ToolExecutionContext::new(Uuid::new_v4());
        ctx.working_directory = std::env::temp_dir();
        ctx
    }

    async fn setup_ctx_with_db() -> (Database, ToolExecutionContext) {
        let db = Database::connect_in_memory()
            .await
            .expect("in-memory DB");
        db.run_migrations().await.expect("migrations");
        let svc = ServiceContext::new(db.pool().clone());
        let mut ctx = ToolExecutionContext::new(Uuid::new_v4());
        ctx.working_directory = std::env::temp_dir();
        ctx.service_context = Some(svc);
        (db, ctx)
    }

    #[test]
    fn tool_metadata() {
        let tool = SelfImproveTool;
        assert_eq!(tool.name(), "self_improve");
        assert!(tool.requires_approval());
        assert!(!tool.capabilities().is_empty());
    }

    #[test]
    fn requires_approval_for_propose() {
        let tool = SelfImproveTool;
        assert!(tool.requires_approval_for_input(&json!({"action": "propose"})));
    }

    #[test]
    fn requires_approval_for_apply() {
        let tool = SelfImproveTool;
        assert!(tool.requires_approval_for_input(&json!({"action": "apply"})));
    }

    #[test]
    fn no_approval_for_list() {
        let tool = SelfImproveTool;
        assert!(!tool.requires_approval_for_input(&json!({"action": "list"})));
    }

    #[tokio::test]
    async fn list_action() {
        let ctx = setup_ctx_no_db();
        let tool = SelfImproveTool;
        let result = tool
            .execute(json!({"action": "list"}), &ctx)
            .await
            .unwrap();
        // list always succeeds — either reads file or reports it doesn't exist
        assert!(result.success);
    }

    #[tokio::test]
    async fn propose_missing_description() {
        let ctx = setup_ctx_no_db();
        let tool = SelfImproveTool;
        let result = tool
            .execute(json!({"action": "propose"}), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result_text(&result).contains("description"));
    }

    #[tokio::test]
    async fn propose_writes_to_improvements() {
        let (_db, ctx) = setup_ctx_with_db().await;
        let tool = SelfImproveTool;
        let result = tool
            .execute(
                json!({
                    "action": "propose",
                    "description": "Add retry logic to bash tool",
                    "rationale": "Frequent transient failures observed"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("proposed"));
        assert!(result.output.contains("Add retry logic"));

        // Verify IMPROVEMENTS.md was written
        let home = crate::config::opencrabs_home();
        let improvements = std::fs::read_to_string(home.join("IMPROVEMENTS.md")).unwrap();
        assert!(improvements.contains("[Proposed]"));
        assert!(improvements.contains("Add retry logic"));
        assert!(improvements.contains("Frequent transient failures"));
    }

    #[tokio::test]
    async fn apply_missing_fields() {
        let ctx = setup_ctx_no_db();
        let tool = SelfImproveTool;

        // Missing target_file
        let result = tool
            .execute(
                json!({
                    "action": "apply",
                    "description": "test",
                    "content": "test content"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result_text(&result).contains("required"));

        // Missing content
        let result = tool
            .execute(
                json!({
                    "action": "apply",
                    "target_file": "SOUL.md",
                    "description": "test"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.success);

        // Missing description
        let result = tool
            .execute(
                json!({
                    "action": "apply",
                    "target_file": "SOUL.md",
                    "content": "test"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn apply_invalid_target_file() {
        let ctx = setup_ctx_no_db();
        let tool = SelfImproveTool;
        let result = tool
            .execute(
                json!({
                    "action": "apply",
                    "target_file": "EVIL.md",
                    "description": "test",
                    "content": "malicious content"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result_text(&result).contains("must be one of"));
    }

    #[tokio::test]
    async fn apply_rejects_path_traversal() {
        let ctx = setup_ctx_no_db();
        let tool = SelfImproveTool;
        let result = tool
            .execute(
                json!({
                    "action": "apply",
                    "target_file": "../../../etc/passwd",
                    "description": "test",
                    "content": "test"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result_text(&result).contains("must be one of"));
    }

    #[tokio::test]
    async fn apply_valid_brain_file() {
        let (_db, ctx) = setup_ctx_with_db().await;
        let tool = SelfImproveTool;

        let result = tool
            .execute(
                json!({
                    "action": "apply",
                    "target_file": "SOUL.md",
                    "description": "Add conciseness guideline",
                    "rationale": "Users consistently prefer shorter responses",
                    "content": "## Conciseness\nKeep responses under 3 sentences when possible."
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("applied"));
        assert!(result.output.contains("SOUL.md"));

        // Verify content was appended to SOUL.md
        let home = crate::config::opencrabs_home();
        let soul = std::fs::read_to_string(home.join("SOUL.md")).unwrap();
        assert!(soul.contains("Conciseness"));

        // Verify IMPROVEMENTS.md logged the change
        let improvements = std::fs::read_to_string(home.join("IMPROVEMENTS.md")).unwrap();
        assert!(improvements.contains("[Applied]"));
        assert!(improvements.contains("SOUL.md"));
    }

    #[tokio::test]
    async fn apply_all_allowed_files_pass_whitelist() {
        let tool = SelfImproveTool;
        let allowed = [
            "SOUL.md",
            "USER.md",
            "AGENTS.md",
            "TOOLS.md",
            "CODE.md",
            "SECURITY.md",
            "MEMORY.md",
            "BOOT.md",
            "IDENTITY.md",
        ];

        let ctx = setup_ctx_no_db();
        for file in &allowed {
            let result = tool
                .execute(
                    json!({
                        "action": "apply",
                        "target_file": file,
                        "description": "test",
                        "content": "test"
                    }),
                    &ctx,
                )
                .await
                .unwrap();
            // Should NOT get "must be one of" error (may get other errors like file I/O)
            if !result.success {
                let err = result_text(&result);
                assert!(
                    !err.contains("must be one of"),
                    "{file} should be allowed but got: {err}",
                );
            }
        }
    }

    #[tokio::test]
    async fn unknown_action() {
        let ctx = setup_ctx_no_db();
        let tool = SelfImproveTool;
        let result = tool
            .execute(json!({"action": "delete"}), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result_text(&result).contains("Unknown action"));
    }

    #[tokio::test]
    async fn propose_without_rationale() {
        let (_db, ctx) = setup_ctx_with_db().await;
        let tool = SelfImproveTool;
        let result = tool
            .execute(
                json!({
                    "action": "propose",
                    "description": "Improve error messages"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.success);
        // Should still work, rationale defaults to "(none)"
        let home = crate::config::opencrabs_home();
        let improvements = std::fs::read_to_string(home.join("IMPROVEMENTS.md")).unwrap();
        assert!(improvements.contains("(none)"));
    }
}

// --- Integration: Record → Analyze round-trip ---

mod rsi_integration {
    use crate::brain::tools::feedback_analyze::FeedbackAnalyzeTool;
    use crate::brain::tools::feedback_record::FeedbackRecordTool;
    use crate::brain::tools::{Tool, ToolExecutionContext};
    use crate::db::Database;
    use crate::services::ServiceContext;
    use serde_json::json;
    use uuid::Uuid;

    async fn setup() -> (Database, ToolExecutionContext) {
        let db = Database::connect_in_memory()
            .await
            .expect("in-memory DB");
        db.run_migrations().await.expect("migrations");
        let svc = ServiceContext::new(db.pool().clone());
        let mut ctx = ToolExecutionContext::new(Uuid::new_v4());
        ctx.service_context = Some(svc);
        (db, ctx)
    }

    #[tokio::test]
    async fn record_then_analyze_summary() {
        let (_db, ctx) = setup().await;
        let record = FeedbackRecordTool;
        let analyze = FeedbackAnalyzeTool;

        // Record several events
        record
            .execute(
                json!({"event_type": "tool_success", "dimension": "bash", "value": 1.0}),
                &ctx,
            )
            .await
            .unwrap();
        record
            .execute(
                json!({"event_type": "tool_success", "dimension": "read", "value": 1.0}),
                &ctx,
            )
            .await
            .unwrap();
        record
            .execute(
                json!({"event_type": "tool_failure", "dimension": "edit", "value": 0.0, "metadata": "file locked"}),
                &ctx,
            )
            .await
            .unwrap();

        // Analyze summary
        let result = analyze
            .execute(json!({"query": "summary"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("3 total events"));
        assert!(result.output.contains("tool_success"));
        assert!(result.output.contains("tool_failure"));
    }

    #[tokio::test]
    async fn record_then_analyze_tool_stats() {
        let (_db, ctx) = setup().await;
        let record = FeedbackRecordTool;
        let analyze = FeedbackAnalyzeTool;

        // 5 bash successes, 1 bash failure
        for _ in 0..5 {
            record
                .execute(
                    json!({"event_type": "tool_success", "dimension": "bash"}),
                    &ctx,
                )
                .await
                .unwrap();
        }
        record
            .execute(
                json!({"event_type": "tool_failure", "dimension": "bash"}),
                &ctx,
            )
            .await
            .unwrap();

        let result = analyze
            .execute(json!({"query": "tool_stats"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("bash"));
        assert!(result.output.contains("83.3%"));
    }

    #[tokio::test]
    async fn record_then_analyze_failures() {
        let (_db, ctx) = setup().await;
        let record = FeedbackRecordTool;
        let analyze = FeedbackAnalyzeTool;

        record
            .execute(
                json!({"event_type": "tool_failure", "dimension": "edit", "metadata": "permission denied"}),
                &ctx,
            )
            .await
            .unwrap();
        record
            .execute(
                json!({"event_type": "tool_success", "dimension": "bash"}),
                &ctx,
            )
            .await
            .unwrap();

        let result = analyze
            .execute(json!({"query": "failures"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("1 entries"));
        assert!(result.output.contains("edit"));
        assert!(result.output.contains("permission denied"));
    }

    #[tokio::test]
    async fn edge_case_high_volume() {
        let (_db, ctx) = setup().await;
        let record = FeedbackRecordTool;
        let analyze = FeedbackAnalyzeTool;

        // Record 100 events
        for i in 0..100 {
            let event_type = if i % 5 == 0 {
                "tool_failure"
            } else {
                "tool_success"
            };
            record
                .execute(
                    json!({
                        "event_type": event_type,
                        "dimension": format!("tool_{}", i % 10),
                        "value": if event_type == "tool_success" { 1.0 } else { 0.0 }
                    }),
                    &ctx,
                )
                .await
                .unwrap();
        }

        // Summary should show 100 total
        let result = analyze
            .execute(json!({"query": "summary"}), &ctx)
            .await
            .unwrap();
        assert!(result.output.contains("100 total events"));

        // Tool stats should show dimensions
        let result = analyze
            .execute(json!({"query": "tool_stats"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);

        // Recent with limit
        let result = analyze
            .execute(json!({"query": "recent", "limit": 5}), &ctx)
            .await
            .unwrap();
        assert!(result.output.contains("5 entries"));
    }
}
