use crate::brain::provider::custom_openai_compatible::OpenAIProvider;
use crate::brain::provider::custom_openai_compatible::extract_sanitized_tail;
use crate::brain::provider::types::{ContentBlock, LLMRequest, Message, Role};

#[test]
fn test_extract_sanitized_tail_empty_and_zero() {
    assert_eq!(extract_sanitized_tail("", 60), "");
    assert_eq!(extract_sanitized_tail("hello", 0), "");
    assert_eq!(extract_sanitized_tail("", 0), "");
}

#[test]
fn test_extract_sanitized_tail_shorter_than_limit() {
    assert_eq!(extract_sanitized_tail("short", 10), "short");
    assert_eq!(extract_sanitized_tail("a", 1), "a");
}

#[test]
fn test_extract_sanitized_tail_exact_and_exceeding_ascii() {
    assert_eq!(extract_sanitized_tail("1234567890", 10), "1234567890");
    assert_eq!(extract_sanitized_tail("prefix1234567890", 10), "1234567890");
    assert_eq!(
        extract_sanitized_tail("abcdefghijklmnopqrstuvwxyz", 5),
        "vwxyz"
    );
}

#[test]
fn test_extract_sanitized_tail_newline_escaping() {
    assert_eq!(
        extract_sanitized_tail("line1\nline2\r\nline3", 12),
        "line2\\r\\nline3"
    );
    assert_eq!(
        extract_sanitized_tail("mid-turn text\n\n{\"tool\": \"bash\"}", 18),
        "\\n\\n{\"tool\": \"bash\"}"
    );
}

#[test]
fn test_extract_sanitized_tail_multibyte_utf8() {
    let text = "🦀 Привет 世界 🚀";
    assert_eq!(extract_sanitized_tail(text, 2), " 🚀");
    assert_eq!(extract_sanitized_tail(text, 4), "世界 🚀");
    assert_eq!(extract_sanitized_tail(text, 50), text);

    let cyrillic = "тестовая строка для проверки хвостового среза";
    assert_eq!(extract_sanitized_tail(cyrillic, 5), "среза");
}

#[test]
fn test_reasoning_echo_summary_in_payload_build() {
    let provider = OpenAIProvider::with_base_url(
        "test-key".to_string(),
        "https://api.openai.com/v1".to_string(),
    );

    let mut request = LLMRequest::new(
        "gpt-4o",
        vec![
            Message::user("Hello"),
            Message {
                role: Role::Assistant,
                content: vec![
                    ContentBlock::Thinking {
                        thinking: "Step 1: analyze input".to_string(),
                        signature: None,
                    },
                    ContentBlock::ToolUse {
                        id: "call_1".to_string(),
                        name: "bash".to_string(),
                        input: serde_json::json!({"command": "ls"}),
                    },
                ],
            },
            Message {
                role: Role::User,
                content: vec![ContentBlock::ToolResult {
                    tool_use_id: "call_1".to_string(),
                    content: "file1.txt\nfile2.txt".to_string(),
                    is_error: None,
                }],
            },
        ],
    );
    request.system = Some("You are a helpful assistant".to_string());
    request.max_tokens = Some(1024);

    let openai_req = provider.to_openai_request(request);
    assert_eq!(openai_req.messages.len(), 4); // system + user + assistant + tool
    assert_eq!(openai_req.messages[0].role, "system");
    assert_eq!(openai_req.messages[1].role, "user");
    assert_eq!(openai_req.messages[2].role, "assistant");
    assert_eq!(openai_req.messages[3].role, "tool");
}
