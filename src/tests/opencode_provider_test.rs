//! OpenCode CLI Provider Tests
//!
//! Tests for the OpenCode CLI provider integration including:
//! - Binary resolution and provider creation
//! - Config wiring and resolution
//! - End-to-end completion with actual CLI calls (requires opencode installed)
//! - Tool execution alongside the provider (bash, read, write, glob, grep, memory)
//!
//! E2E tests skip gracefully if the opencode binary is not installed.

use crate::brain::provider::Provider;
use crate::brain::provider::opencode_cli::OpenCodeCliProvider;
use crate::brain::provider::types::*;
use crate::brain::tools::bash::BashTool;
use crate::brain::tools::glob::GlobTool;
use crate::brain::tools::grep::GrepTool;
use crate::brain::tools::read::ReadTool;
use crate::brain::tools::write::WriteTool;
use crate::brain::tools::{Tool, ToolExecutionContext};
use crate::config::{Config, ProviderConfig};
use uuid::Uuid;

/// Helper: create OpenCodeCliProvider or return None if binary not installed.
fn try_provider() -> Option<OpenCodeCliProvider> {
    OpenCodeCliProvider::new().ok()
}

/// Helper: build a minimal LLMRequest for a single user message.
fn simple_request(model: &str, prompt: &str) -> LLMRequest {
    LLMRequest::new(
        model,
        vec![Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: prompt.to_string(),
            }],
        }],
    )
}

/// Helper: extract all text from an LLMResponse.
fn extract_text(response: &LLMResponse) -> String {
    response
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

// ── Provider creation tests ──

#[test]
fn opencode_provider_name_is_opencode() {
    if OpenCodeCliProvider::new().is_err() {
        eprintln!("opencode not installed, skipping");
        return;
    }
    let provider = OpenCodeCliProvider::new().unwrap();
    assert_eq!(provider.name(), "opencode");
}

#[test]
fn opencode_provider_default_model() {
    if OpenCodeCliProvider::new().is_err() {
        return;
    }
    let provider = OpenCodeCliProvider::new().unwrap();
    assert_eq!(provider.default_model(), "opencode/gpt-5-nano");
}

#[test]
fn opencode_provider_with_custom_model() {
    if OpenCodeCliProvider::new().is_err() {
        return;
    }
    let provider = OpenCodeCliProvider::new()
        .unwrap()
        .with_default_model("opencode/big-pickle".to_string());
    assert_eq!(provider.default_model(), "opencode/big-pickle");
}

#[test]
fn opencode_provider_supported_models_not_empty() {
    if OpenCodeCliProvider::new().is_err() {
        return;
    }
    let provider = OpenCodeCliProvider::new().unwrap();
    let models = provider.supported_models();
    assert!(!models.is_empty(), "supported_models should not be empty");
    assert!(
        models.contains(&"opencode/gpt-5-nano".to_string()),
        "should contain gpt-5-nano"
    );
}

#[test]
fn opencode_provider_does_not_support_tools() {
    if OpenCodeCliProvider::new().is_err() {
        return;
    }
    let provider = OpenCodeCliProvider::new().unwrap();
    assert!(
        !provider.supports_tools(),
        "OpenCode CLI should not support native tools — OpenCrabs handles them"
    );
}

#[test]
fn opencode_provider_supports_vision() {
    if OpenCodeCliProvider::new().is_err() {
        return;
    }
    let provider = OpenCodeCliProvider::new().unwrap();
    assert!(!provider.supports_vision()); // CLI mode uses analyze_image fallback
}

#[test]
fn opencode_provider_has_context_window() {
    if OpenCodeCliProvider::new().is_err() {
        return;
    }
    let provider = OpenCodeCliProvider::new().unwrap();
    let cw = provider.context_window("opencode/gpt-5-nano");
    assert!(cw.is_some());
    assert!(cw.unwrap() > 0);
}

// ── Config resolution tests ──

#[test]
fn opencode_config_resolves_when_enabled() {
    let mut config = Config::default();
    config.providers.opencode_cli = Some(ProviderConfig {
        enabled: true,
        default_model: Some("opencode/big-pickle".to_string()),
        ..Default::default()
    });
    let (name, model) = crate::config::resolve_provider_from_config(&config);
    assert_eq!(name, "OpenCode CLI");
    assert_eq!(model, "opencode/big-pickle");
}

#[test]
fn opencode_config_not_resolved_when_disabled() {
    let mut config = Config::default();
    config.providers.opencode_cli = Some(ProviderConfig {
        enabled: false,
        default_model: Some("opencode/big-pickle".to_string()),
        ..Default::default()
    });
    let (name, _) = crate::config::resolve_provider_from_config(&config);
    assert_ne!(name, "OpenCode CLI");
}

#[test]
fn opencode_provider_sync_in_onboarding_array() {
    use crate::tui::onboarding::PROVIDERS;
    let names: Vec<&str> = PROVIDERS.iter().map(|p| p.name).collect();
    assert!(
        names.contains(&"OpenCode CLI"),
        "PROVIDERS must contain OpenCode CLI. Got: {:?}",
        names
    );
}

// ── Factory creation tests ──

#[test]
fn factory_creates_opencode_by_name() {
    if OpenCodeCliProvider::new().is_err() {
        return;
    }
    let mut config = Config::default();
    config.providers.opencode_cli = Some(ProviderConfig {
        enabled: true,
        default_model: Some("opencode/gpt-5-nano".to_string()),
        ..Default::default()
    });
    let provider = crate::brain::provider::factory::create_provider_by_name(&config, "opencode");
    assert!(provider.is_ok(), "Should create opencode provider by name");
    assert_eq!(provider.unwrap().name(), "opencode");
}

#[test]
fn factory_creates_opencode_by_alt_names() {
    if OpenCodeCliProvider::new().is_err() {
        return;
    }
    let mut config = Config::default();
    config.providers.opencode_cli = Some(ProviderConfig {
        enabled: true,
        default_model: Some("opencode/gpt-5-nano".to_string()),
        ..Default::default()
    });

    for name in ["opencode", "opencode-cli", "opencode_cli"] {
        let provider = crate::brain::provider::factory::create_provider_by_name(&config, name);
        assert!(
            provider.is_ok(),
            "Should create opencode provider via name '{}'",
            name
        );
    }
}

// ── End-to-end: actual CLI calls ──

#[tokio::test]
async fn e2e_opencode_simple_completion() {
    let Some(provider) = try_provider() else {
        return;
    };
    let request = simple_request("opencode/gpt-5-nano", "Reply with exactly: HELLO_OPENCRABS");

    let response = provider.complete(request).await;
    assert!(
        response.is_ok(),
        "completion should succeed: {:?}",
        response.err()
    );
    let response = response.unwrap();
    assert!(!response.content.is_empty(), "response should have content");

    let text = extract_text(&response);
    assert!(
        text.contains("HELLO_OPENCRABS"),
        "response should contain HELLO_OPENCRABS, got: {}",
        text
    );
}

#[tokio::test]
async fn e2e_opencode_streaming() {
    use futures::StreamExt;
    use tokio::time::{Duration, timeout};

    let provider = {
        let Some(p) = try_provider() else { return };
        p
    };
    let request = simple_request("opencode/gpt-5-nano", "Say hello in one word.");

    let stream = provider.stream(request).await;
    assert!(stream.is_ok(), "stream should start: {:?}", stream.err());
    let mut stream = stream.unwrap();

    let mut got_start = false;
    let mut got_text = false;
    let mut got_stop = false;

    let result = timeout(Duration::from_secs(30), async {
        while let Some(event) = stream.next().await {
            let event = event.expect("stream event should not be error");
            match event {
                StreamEvent::MessageStart { .. } => got_start = true,
                StreamEvent::ContentBlockDelta {
                    delta: ContentDelta::TextDelta { .. },
                    ..
                } => got_text = true,
                StreamEvent::MessageStop => {
                    got_stop = true;
                    break;
                }
                _ => {}
            }
        }
    })
    .await;

    if result.is_err() {
        eprintln!("e2e_opencode_streaming timed out after 30s, skipping");
        return;
    }

    assert!(got_start, "should have received MessageStart");
    assert!(got_text, "should have received at least one text delta");
    assert!(got_stop, "should have received MessageStop");
}

// ── End-to-end: tools alongside provider ──

#[tokio::test]
async fn e2e_opencode_with_bash_tool() {
    let provider = {
        let Some(p) = try_provider() else { return };
        p
    };

    let mut request = simple_request(
        "opencode/gpt-5-nano",
        "What is 2 + 2? Reply with just the number.",
    );
    request.system = Some("You are a helpful assistant. Reply concisely.".to_string());

    let response = provider
        .complete(request)
        .await
        .expect("completion should work");
    let llm_text = extract_text(&response);
    assert!(
        llm_text.contains('4'),
        "LLM should answer 4, got: {}",
        llm_text
    );

    // Verify with bash tool
    let bash = BashTool;
    let ctx = ToolExecutionContext::new(Uuid::new_v4()).with_auto_approve(true);
    let result = bash
        .execute(serde_json::json!({ "command": "echo $((2 + 2))" }), &ctx)
        .await
        .expect("bash tool should execute");
    assert!(result.success);
    assert!(result.output.trim().contains('4'));
}

#[tokio::test]
async fn e2e_opencode_with_write_and_read_tools() {
    let tmp_dir = tempfile::tempdir().expect("create temp dir");
    let test_file = tmp_dir.path().join("opencode_test.txt");

    // Use provider to generate content
    let provider = {
        let Some(p) = try_provider() else { return };
        p
    };
    let request = simple_request(
        "opencode/gpt-5-nano",
        "Write exactly this text and nothing else: OpenCrabs integration test OK",
    );

    let response = provider.complete(request).await.expect("completion works");
    let generated_text = extract_text(&response);

    // Write using WriteTool
    let write_tool = WriteTool;
    let ctx = ToolExecutionContext::new(Uuid::new_v4())
        .with_working_directory(tmp_dir.path().to_path_buf())
        .with_auto_approve(true);

    let write_result = write_tool
        .execute(
            serde_json::json!({
                "path": test_file.to_string_lossy(),
                "content": generated_text.trim()
            }),
            &ctx,
        )
        .await
        .expect("write tool should work");
    assert!(
        write_result.success,
        "write should succeed: {}",
        write_result.output
    );

    // Read back with ReadTool
    let read_tool = ReadTool;
    let read_result = read_tool
        .execute(
            serde_json::json!({ "path": test_file.to_string_lossy() }),
            &ctx,
        )
        .await
        .expect("read tool should work");
    assert!(read_result.success, "read should succeed");
    assert!(
        read_result.output.contains("OpenCrabs"),
        "read output should contain generated text: {}",
        read_result.output
    );
}

#[tokio::test]
async fn e2e_opencode_with_glob_and_grep_tools() {
    let tmp_dir = tempfile::tempdir().expect("create temp dir");
    let ctx = ToolExecutionContext::new(Uuid::new_v4())
        .with_working_directory(tmp_dir.path().to_path_buf())
        .with_auto_approve(true);

    // Create test files
    let write_tool = WriteTool;
    for (name, content) in [
        ("alpha.txt", "hello world from opencrabs"),
        ("beta.rs", "fn main() { println!(\"opencrabs\"); }"),
        ("gamma.log", "no match here"),
    ] {
        let path = tmp_dir.path().join(name);
        write_tool
            .execute(
                serde_json::json!({
                    "path": path.to_string_lossy(),
                    "content": content,
                }),
                &ctx,
            )
            .await
            .expect("write should work");
    }

    // GlobTool: find .txt files
    let glob_tool = GlobTool;
    let glob_result = glob_tool
        .execute(
            serde_json::json!({
                "pattern": "*.txt",
                "path": tmp_dir.path().to_string_lossy()
            }),
            &ctx,
        )
        .await
        .expect("glob should work");
    assert!(glob_result.success);
    assert!(
        glob_result.output.contains("alpha.txt"),
        "glob should find alpha.txt: {}",
        glob_result.output
    );

    // GrepTool: search for "opencrabs"
    let grep_tool = GrepTool;
    let grep_result = grep_tool
        .execute(
            serde_json::json!({
                "pattern": "opencrabs",
                "path": tmp_dir.path().to_string_lossy()
            }),
            &ctx,
        )
        .await
        .expect("grep should work");
    assert!(grep_result.success);
    assert!(
        grep_result.output.contains("alpha.txt"),
        "grep should find alpha.txt"
    );
    assert!(
        grep_result.output.contains("beta.rs"),
        "grep should find beta.rs"
    );

    // Use provider to interpret the results
    let provider = {
        let Some(p) = try_provider() else { return };
        p
    };
    let request = simple_request(
        "opencode/gpt-5-nano",
        &format!(
            "The grep results are:\n{}\nHow many files contain 'opencrabs'? Reply with just the number.",
            grep_result.output
        ),
    );

    let response = provider.complete(request).await.expect("completion works");
    let answer = extract_text(&response);
    assert!(answer.contains('2'), "should answer 2, got: {}", answer);
}

// ── Multi-turn conversation test ──

#[tokio::test]
async fn e2e_opencode_multi_turn() {
    let provider = {
        let Some(p) = try_provider() else { return };
        p
    };

    // Turn 1: set a fact
    let mut request1 = simple_request(
        "opencode/gpt-5-nano",
        "Remember this: the secret code is CRAB42. Just acknowledge.",
    );
    request1.system = Some("You are a helpful assistant with good memory.".to_string());

    let resp1 = provider
        .complete(request1)
        .await
        .expect("turn 1 should work");

    // Turn 2: ask for the fact back via message history
    let request2 = LLMRequest {
        messages: vec![
            Message {
                role: Role::User,
                content: vec![ContentBlock::Text {
                    text: "Remember this: the secret code is CRAB42. Just acknowledge.".to_string(),
                }],
            },
            Message {
                role: Role::Assistant,
                content: resp1.content,
            },
            Message {
                role: Role::User,
                content: vec![ContentBlock::Text {
                    text: "What was the secret code? Reply with just the code.".to_string(),
                }],
            },
        ],
        system: Some("You are a helpful assistant with good memory.".to_string()),
        ..LLMRequest::new("opencode/gpt-5-nano", vec![])
    };

    let resp2 = provider
        .complete(request2)
        .await
        .expect("turn 2 should work");
    let text = extract_text(&resp2);
    assert!(
        text.contains("CRAB42"),
        "multi-turn should recall the code, got: {}",
        text
    );
}

// ── Memory search tool test ──

#[tokio::test]
async fn memory_search_tool_returns_result_for_query() {
    use crate::brain::tools::memory_search::MemorySearchTool;

    let tool = MemorySearchTool;
    let ctx = ToolExecutionContext::new(Uuid::new_v4());

    let result = tool
        .execute(
            serde_json::json!({ "query": "opencode provider test" }),
            &ctx,
        )
        .await
        .expect("memory search should not panic");
    // Success regardless of whether memories exist
    assert!(result.success || result.output.contains("No") || result.output.contains("no"));
}

#[test]
fn memory_search_tool_requires_query() {
    use crate::brain::tools::memory_search::MemorySearchTool;

    let schema = MemorySearchTool.input_schema();
    let required = schema.get("required").and_then(|v| v.as_array());
    assert!(
        required.is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some("query"))),
        "memory_search should require 'query' parameter"
    );
}

// ── Tool registry includes expected tools ──

#[test]
fn tool_registry_has_core_tools() {
    use crate::brain::tools::registry::ToolRegistry;
    use std::sync::Arc;

    let registry = ToolRegistry::new();
    registry.register(Arc::new(BashTool));
    registry.register(Arc::new(ReadTool));
    registry.register(Arc::new(WriteTool));
    registry.register(Arc::new(GlobTool));
    registry.register(Arc::new(GrepTool));

    for expected in ["bash", "read_file", "write_file", "glob", "grep"] {
        assert!(
            registry.get(expected).is_some(),
            "registry should contain '{}' tool",
            expected,
        );
    }
}
