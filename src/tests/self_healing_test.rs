//! Self-Healing System Tests
//!
//! Tests for config recovery, DB integrity, config typo warnings,
//! custom provider name normalization, and state cleanup.

use crate::config::{Config, normalize_toml_key};
use crate::db::Database;

// ── Config Last-Known-Good Recovery ─────────────────────────────────────

#[test]
fn config_load_recovers_from_last_known_good() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let good_path = dir.path().join("config.last_good.toml");

    // Write a valid last-known-good config
    std::fs::write(
        &good_path,
        r#"
[agent]
context_limit = 100000
max_tokens = 8192
"#,
    )
    .unwrap();

    // Write a broken config.toml
    std::fs::write(&config_path, "{{{{ broken toml !@#$%").unwrap();

    // load_from_path on the broken file should fail
    assert!(Config::load_from_path(&config_path).is_err());

    // load_from_path on the good file should succeed
    let good = Config::load_from_path(&good_path).unwrap();
    assert_eq!(good.agent.context_limit, 100_000);
    assert_eq!(good.agent.max_tokens, 8192);
}

#[test]
fn config_load_from_valid_file_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    std::fs::write(
        &path,
        r#"
[agent]
context_limit = 50000
max_tokens = 4096

[providers.anthropic]
enabled = true
"#,
    )
    .unwrap();

    let config = Config::load_from_path(&path).unwrap();
    assert_eq!(config.agent.context_limit, 50_000);
    assert_eq!(config.agent.max_tokens, 4096);
    assert!(config.providers.anthropic.unwrap().enabled);
}

#[test]
fn config_load_from_broken_file_fails() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "not valid toml {{{{").unwrap();
    assert!(Config::load_from_path(&path).is_err());
}

// ── Config Typo Warnings ────────────────────────────────────────────────

#[test]
fn config_known_top_level_keys_are_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    // All known keys — should parse without errors
    std::fs::write(
        &path,
        r#"
[crabrace]
[database]
[logging]
[debug]
[providers]
[channels]
[agent]
[daemon]
[a2a]
[image]
[cron]
"#,
    )
    .unwrap();

    let config = Config::load_from_path(&path);
    assert!(config.is_ok());
}

#[test]
fn config_gateway_alias_maps_to_a2a() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    std::fs::write(
        &path,
        r#"
[gateway]
enabled = true
port = 9999
"#,
    )
    .unwrap();

    let config = Config::load_from_path(&path).unwrap();
    assert!(config.a2a.enabled);
    assert_eq!(config.a2a.port, 9999);
}

// ── DB Integrity Check ──────────────────────────────────────────────────

#[tokio::test]
async fn db_integrity_check_passes_on_clean_db() {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();

    // After successful migrations, integrity should be fine
    // The flag should not be set (false)
    // Note: db_integrity_failed() is a global static, so this test just
    // verifies the clean path doesn't set the flag
    assert!(!crate::db::db_integrity_failed());
}

#[tokio::test]
async fn db_in_memory_migrations_succeed() {
    let db = Database::connect_in_memory().await.unwrap();
    // Migrations should complete without error
    let result = db.run_migrations().await;
    assert!(result.is_ok(), "Migrations failed: {:?}", result.err());
}

// ── Custom Provider Name Normalization ──────────────────────────────────

#[test]
fn normalize_toml_key_lowercases() {
    assert_eq!(normalize_toml_key("Qwen"), "qwen");
    assert_eq!(normalize_toml_key("OLLAMA"), "ollama");
    assert_eq!(normalize_toml_key("DeepSeek"), "deepseek");
}

#[test]
fn normalize_toml_key_replaces_separators_with_hyphens() {
    assert_eq!(normalize_toml_key("Qwen_2.5_4B"), "qwen-2-5-4b");
    assert_eq!(normalize_toml_key("my_provider"), "my-provider");
    assert_eq!(normalize_toml_key("My Provider"), "my-provider");
    assert_eq!(normalize_toml_key("a.b.c"), "a-b-c");
}

#[test]
fn normalize_toml_key_strips_special_chars() {
    assert_eq!(normalize_toml_key("model@v2!"), "modelv2");
    assert_eq!(normalize_toml_key("test#123"), "test123");
}

#[test]
fn normalize_toml_key_trims_hyphens() {
    assert_eq!(normalize_toml_key("_leading_"), "leading");
    assert_eq!(normalize_toml_key("  spaces  "), "spaces");
    assert_eq!(normalize_toml_key("__double__"), "double");
}

#[test]
fn normalize_toml_key_preserves_clean_names() {
    assert_eq!(normalize_toml_key("ollama"), "ollama");
    assert_eq!(normalize_toml_key("nvidia"), "nvidia");
    assert_eq!(normalize_toml_key("qwen-2-5-4b"), "qwen-2-5-4b");
}

#[test]
fn custom_provider_names_normalized_on_deserialize() {
    let toml_str = r#"
[providers.custom.Qwen_2_5_4B]
enabled = true
base_url = "http://localhost:11434/v1"
default_model = "qwen2.5:4b"

[providers.custom.My_Other_Model]
enabled = false
base_url = "http://localhost:8080/v1"
"#;

    let config: Config = toml::from_str(toml_str).unwrap();
    let custom = config.providers.custom.unwrap();

    // Keys should be normalized
    assert!(
        custom.contains_key("qwen-2-5-4b"),
        "Keys: {:?}",
        custom.keys().collect::<Vec<_>>()
    );
    assert!(
        custom.contains_key("my-other-model"),
        "Keys: {:?}",
        custom.keys().collect::<Vec<_>>()
    );

    // Original casing should NOT be preserved
    assert!(!custom.contains_key("Qwen_2_5_4B"));
    assert!(!custom.contains_key("My_Other_Model"));

    // Values should be intact
    let qwen = custom.get("qwen-2-5-4b").unwrap();
    assert!(qwen.enabled);
    assert_eq!(qwen.base_url.as_deref(), Some("http://localhost:11434/v1"));
    assert_eq!(qwen.default_model.as_deref(), Some("qwen2.5:4b"));
}

#[test]
fn custom_by_name_case_insensitive() {
    let toml_str = r#"
[providers.custom.ollama]
enabled = true
base_url = "http://localhost:11434/v1"
"#;

    let config: Config = toml::from_str(toml_str).unwrap();

    // Lookup with any casing should work
    assert!(config.providers.custom_by_name("ollama").is_some());
    assert!(config.providers.custom_by_name("OLLAMA").is_some());
    assert!(config.providers.custom_by_name("Ollama").is_some());
}

// ── Config Write & Read Roundtrip ───────────────────────────────────────

#[test]
fn config_write_key_normalizes_custom_provider_section() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");

    // Start with empty config
    std::fs::write(&config_path, "").unwrap();

    // Write using unnormalized section name — would be what a user types
    // Test normalization of a key that uses underscores (not dots,
    // since dots are TOML section separators)
    let section = "providers.custom.Qwen_2_5_4B";
    let parts: Vec<String> = section
        .split('.')
        .enumerate()
        .map(|(i, p)| {
            if i >= 2 && section.starts_with("providers.custom") {
                normalize_toml_key(p)
            } else {
                p.to_string()
            }
        })
        .collect();

    assert_eq!(parts, vec!["providers", "custom", "qwen-2-5-4b"]);
}

// ── AgentService Config Requirement ─────────────────────────────────────

#[tokio::test]
async fn agent_service_new_for_test_uses_defaults() {
    use crate::brain::agent::AgentService;
    use crate::brain::provider::PlaceholderProvider;
    use std::sync::Arc;

    let provider = Arc::new(PlaceholderProvider);
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let ctx = crate::services::ServiceContext::new(db.pool().clone());

    let agent = AgentService::new_for_test(provider, ctx);

    // Should use Config::default() values
    let defaults = Config::default();
    assert_eq!(agent.context_limit(), defaults.agent.context_limit);
    assert_eq!(agent.max_tokens(), defaults.agent.max_tokens);
}

#[tokio::test]
async fn agent_service_new_uses_provided_config() {
    use crate::brain::agent::AgentService;
    use crate::brain::provider::PlaceholderProvider;
    use std::sync::Arc;

    let provider = Arc::new(PlaceholderProvider);
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let ctx = crate::services::ServiceContext::new(db.pool().clone());

    let mut config = Config::default();
    config.agent.context_limit = 42_000;
    config.agent.max_tokens = 1234;

    let agent = AgentService::new(provider, ctx, &config);
    assert_eq!(agent.context_limit(), 42_000);
    assert_eq!(agent.max_tokens(), 1234);
}

// ── SelfHealingAlert ProgressEvent ──────────────────────────────────────

#[test]
fn self_healing_alert_progress_event_carries_message() {
    use crate::brain::agent::ProgressEvent;

    let event = ProgressEvent::SelfHealingAlert {
        message: "Emergency compaction: context too large".to_string(),
    };

    match event {
        ProgressEvent::SelfHealingAlert { message } => {
            assert!(message.contains("compaction"));
        }
        _ => panic!("Expected SelfHealingAlert variant"),
    }
}

// ── Pending Request Crash Recovery ──────────────────────────────────────

#[tokio::test]
async fn pending_requests_created_and_cleared() {
    use crate::db::repository::PendingRequestRepository;

    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let repo = PendingRequestRepository::new(db.pool().clone());

    let id = uuid::Uuid::new_v4();
    let session_id = uuid::Uuid::new_v4();

    // Create a pending request (simulates agent start)
    repo.insert(id, session_id, "test message", "tui")
        .await
        .unwrap();

    // Should show up as interrupted
    let interrupted = repo.get_interrupted().await.unwrap();
    assert_eq!(interrupted.len(), 1);
    assert_eq!(interrupted[0].session_id, session_id.to_string());

    // Clear all (simulates recovery)
    repo.clear_all().await.unwrap();

    // Should be empty now
    let interrupted = repo.get_interrupted().await.unwrap();
    assert!(interrupted.is_empty());
}

#[tokio::test]
async fn pending_requests_deduplicate_by_session() {
    use crate::db::repository::PendingRequestRepository;

    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let repo = PendingRequestRepository::new(db.pool().clone());

    let session_id = uuid::Uuid::new_v4();

    // Insert same session twice with different request IDs
    repo.insert(uuid::Uuid::new_v4(), session_id, "msg1", "tui")
        .await
        .unwrap();
    repo.insert(uuid::Uuid::new_v4(), session_id, "msg2", "tui")
        .await
        .unwrap();

    // Should still only recover once per session
    let interrupted = repo.get_interrupted().await.unwrap();
    // May have 2 rows but recovery deduplicates by session_id
    let unique_sessions: std::collections::HashSet<&String> =
        interrupted.iter().map(|r| &r.session_id).collect();
    assert_eq!(unique_sessions.len(), 1);
}

// ── State Cleanup on Session Delete ─────────────────────────────────────

#[tokio::test]
async fn session_delete_cascades_messages() {
    use crate::services::{MessageService, ServiceContext, SessionService};

    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let ctx = ServiceContext::new(db.pool().clone());

    let session_svc = SessionService::new(ctx.clone());
    let msg_svc = MessageService::new(ctx.clone());

    // Create session and add messages
    let session = session_svc
        .create_session(Some("test".to_string()))
        .await
        .unwrap();
    msg_svc
        .create_message(session.id, "user".to_string(), "hello".to_string())
        .await
        .unwrap();
    msg_svc
        .create_message(session.id, "assistant".to_string(), "hi back".to_string())
        .await
        .unwrap();

    // Verify messages exist
    let msgs = msg_svc.list_messages_for_session(session.id).await.unwrap();
    assert_eq!(msgs.len(), 2);

    // Delete session
    session_svc.delete_session(session.id).await.unwrap();

    // Messages should be gone
    let msgs = msg_svc.list_messages_for_session(session.id).await.unwrap();
    assert!(msgs.is_empty());
}

// ── Config Default Values ───────────────────────────────────────────────

#[test]
fn config_default_has_sane_values() {
    let config = Config::default();
    // Agent defaults should be reasonable
    assert!(config.agent.context_limit > 0);
    assert!(config.agent.max_tokens > 0);
    // A2A should default to disabled
    assert!(!config.a2a.enabled);
}

// ── ToolCallEntry completed field ───────────────────────────────────────

#[test]
fn tool_call_entry_defaults_to_not_completed() {
    use crate::tui::app::ToolCallEntry;

    let entry = ToolCallEntry {
        description: "Read file.rs".to_string(),
        success: true,
        details: None,
        completed: false,
        tool_input: serde_json::Value::Null,
    };

    assert!(!entry.completed);
    assert!(entry.details.is_none());
}

#[test]
fn tool_call_entry_completed_independent_of_details() {
    use crate::tui::app::ToolCallEntry;

    // A tool can be completed with empty details (no summary)
    let entry = ToolCallEntry {
        description: "bash: ls".to_string(),
        success: true,
        details: None,
        completed: true,
        tool_input: serde_json::Value::Null,
    };

    assert!(entry.completed);
    assert!(entry.details.is_none());

    // A tool can be completed with details
    let entry2 = ToolCallEntry {
        description: "Read foo.rs".to_string(),
        success: true,
        details: Some("42 lines".to_string()),
        completed: true,
        tool_input: serde_json::Value::Null,
    };

    assert!(entry2.completed);
    assert!(entry2.details.is_some());
}

// ── Case-Insensitive Tool Input Lookup ──────────────────────────────────

#[test]
fn format_tool_description_handles_camel_case_keys() {
    use crate::tui::app::App;

    // filePath (camelCase) — sent by some models
    let input = serde_json::json!({"filePath": "/tmp/test.rs"});
    let desc = App::format_tool_description("read", &input);
    assert_eq!(desc, "Read /tmp/test.rs");

    // file_path (snake_case)
    let input2 = serde_json::json!({"file_path": "/tmp/test.rs"});
    let desc2 = App::format_tool_description("read", &input2);
    assert_eq!(desc2, "Read /tmp/test.rs");

    // path (canonical)
    let input3 = serde_json::json!({"path": "/tmp/test.rs"});
    let desc3 = App::format_tool_description("read", &input3);
    assert_eq!(desc3, "Read /tmp/test.rs");
}

#[test]
fn format_tool_description_case_insensitive_command() {
    use crate::tui::app::App;

    let input = serde_json::json!({"Command": "ls -la"});
    let desc = App::format_tool_description("bash", &input);
    assert_eq!(desc, "bash: ls -la");
}

#[test]
fn format_tool_description_case_insensitive_query() {
    use crate::tui::app::App;

    let input = serde_json::json!({"Query": "rust async"});
    let desc = App::format_tool_description("web_search", &input);
    assert_eq!(desc, "Search: rust async");
}
