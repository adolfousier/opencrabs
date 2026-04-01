use super::*;

/// Mock provider that mimics MiniMax/OpenAI deferred-usage streaming:
/// - MessageStart with usage(0, 0)
/// - Content deltas
/// - MessageDelta with stop_reason but usage(0, 0)
/// - MessageDelta with real usage and empty stop_reason (usage-only chunk)
/// - MessageStop
struct MockDeferredUsageProvider {
    input_tokens: u32,
    output_tokens: u32,
}

impl MockDeferredUsageProvider {
    fn new(input_tokens: u32, output_tokens: u32) -> Self {
        Self {
            input_tokens,
            output_tokens,
        }
    }
}

#[async_trait]
impl Provider for MockDeferredUsageProvider {
    async fn complete(&self, _request: LLMRequest) -> crate::brain::provider::Result<LLMResponse> {
        Ok(LLMResponse {
            id: "deferred-usage-resp".to_string(),
            model: "mock-model".to_string(),
            content: vec![ContentBlock::Text {
                text: "Hello from deferred usage provider".to_string(),
            }],
            stop_reason: Some(StopReason::EndTurn),
            usage: TokenUsage {
                input_tokens: self.input_tokens,
                output_tokens: self.output_tokens,
                ..Default::default()
            },
        })
    }

    async fn stream(&self, _request: LLMRequest) -> crate::brain::provider::Result<ProviderStream> {
        use crate::brain::provider::{ContentDelta, MessageDelta, StreamEvent, StreamMessage};

        let events = vec![
            // 1. MessageStart — no usage yet (MiniMax pattern)
            Ok(StreamEvent::MessageStart {
                message: StreamMessage {
                    id: "deferred-usage-resp".to_string(),
                    model: "mock-model".to_string(),
                    role: Role::Assistant,
                    usage: TokenUsage {
                        input_tokens: 0,
                        output_tokens: 0,
                        ..Default::default()
                    },
                },
            }),
            // 2. Content block
            Ok(StreamEvent::ContentBlockStart {
                index: 0,
                content_block: ContentBlock::Text {
                    text: String::new(),
                },
            }),
            Ok(StreamEvent::ContentBlockDelta {
                index: 0,
                delta: ContentDelta::TextDelta {
                    text: "Hello from deferred usage provider".to_string(),
                },
            }),
            Ok(StreamEvent::ContentBlockStop { index: 0 }),
            // 3. MessageDelta with stop_reason but NO usage (finish_reason chunk)
            Ok(StreamEvent::MessageDelta {
                delta: MessageDelta {
                    stop_reason: Some(StopReason::EndTurn),
                    stop_sequence: None,
                },
                usage: TokenUsage {
                    input_tokens: 0,
                    output_tokens: 0,
                    ..Default::default()
                },
            }),
            // 4. Usage-only chunk — real usage, no stop_reason (deferred)
            Ok(StreamEvent::MessageDelta {
                delta: MessageDelta {
                    stop_reason: None,
                    stop_sequence: None,
                },
                usage: TokenUsage {
                    input_tokens: self.input_tokens,
                    output_tokens: self.output_tokens,
                    ..Default::default()
                },
            }),
            // 5. MessageStop
            Ok(StreamEvent::MessageStop),
        ];
        Ok(Box::pin(futures::stream::iter(events)))
    }

    fn name(&self) -> &str {
        "mock-deferred-usage"
    }

    fn default_model(&self) -> &str {
        "mock-model"
    }

    fn supported_models(&self) -> Vec<String> {
        vec!["mock-model".to_string()]
    }

    fn context_window(&self, _model: &str) -> Option<u32> {
        Some(200_000)
    }

    fn calculate_cost(&self, _model: &str, _input: u32, _output: u32) -> f64 {
        0.001
    }
}

/// Mock provider that sends usage inline with stop_reason (Anthropic pattern)
struct MockInlineUsageProvider {
    input_tokens: u32,
    output_tokens: u32,
}

impl MockInlineUsageProvider {
    fn new(input_tokens: u32, output_tokens: u32) -> Self {
        Self {
            input_tokens,
            output_tokens,
        }
    }
}

#[async_trait]
impl Provider for MockInlineUsageProvider {
    async fn complete(&self, _request: LLMRequest) -> crate::brain::provider::Result<LLMResponse> {
        Ok(LLMResponse {
            id: "inline-usage-resp".to_string(),
            model: "mock-model".to_string(),
            content: vec![ContentBlock::Text {
                text: "Hello from inline usage provider".to_string(),
            }],
            stop_reason: Some(StopReason::EndTurn),
            usage: TokenUsage {
                input_tokens: self.input_tokens,
                output_tokens: self.output_tokens,
                ..Default::default()
            },
        })
    }

    async fn stream(&self, _request: LLMRequest) -> crate::brain::provider::Result<ProviderStream> {
        use crate::brain::provider::{ContentDelta, MessageDelta, StreamEvent, StreamMessage};

        let events = vec![
            Ok(StreamEvent::MessageStart {
                message: StreamMessage {
                    id: "inline-usage-resp".to_string(),
                    model: "mock-model".to_string(),
                    role: Role::Assistant,
                    usage: TokenUsage {
                        input_tokens: self.input_tokens,
                        output_tokens: 0,
                        ..Default::default()
                    },
                },
            }),
            Ok(StreamEvent::ContentBlockStart {
                index: 0,
                content_block: ContentBlock::Text {
                    text: String::new(),
                },
            }),
            Ok(StreamEvent::ContentBlockDelta {
                index: 0,
                delta: ContentDelta::TextDelta {
                    text: "Hello from inline usage provider".to_string(),
                },
            }),
            Ok(StreamEvent::ContentBlockStop { index: 0 }),
            // Single MessageDelta with both stop_reason AND usage
            Ok(StreamEvent::MessageDelta {
                delta: MessageDelta {
                    stop_reason: Some(StopReason::EndTurn),
                    stop_sequence: None,
                },
                usage: TokenUsage {
                    input_tokens: self.input_tokens,
                    output_tokens: self.output_tokens,
                    ..Default::default()
                },
            }),
            Ok(StreamEvent::MessageStop),
        ];
        Ok(Box::pin(futures::stream::iter(events)))
    }

    fn name(&self) -> &str {
        "mock-inline-usage"
    }

    fn default_model(&self) -> &str {
        "mock-model"
    }

    fn supported_models(&self) -> Vec<String> {
        vec!["mock-model".to_string()]
    }

    fn context_window(&self, _model: &str) -> Option<u32> {
        Some(200_000)
    }

    fn calculate_cost(&self, _model: &str, _input: u32, _output: u32) -> f64 {
        0.001
    }
}

// === Tests ===

#[tokio::test]
async fn test_deferred_usage_captures_real_tokens() {
    let provider = Arc::new(MockDeferredUsageProvider::new(19286, 150));
    let (agent_service, _) = create_test_service_with_provider(provider).await;

    let request = LLMRequest::new("mock-model".to_string(), vec![Message::user("Hello")]);
    let (response, _) = agent_service
        .stream_complete(Uuid::nil(), request, None, None, None, None)
        .await
        .unwrap();

    assert_eq!(
        response.usage.input_tokens, 19286,
        "deferred usage should capture real input tokens from usage-only chunk"
    );
    assert_eq!(
        response.usage.output_tokens, 150,
        "deferred usage should capture real output tokens from usage-only chunk"
    );
    assert_eq!(response.stop_reason, Some(StopReason::EndTurn));
}

#[tokio::test]
async fn test_inline_usage_still_works() {
    let provider = Arc::new(MockInlineUsageProvider::new(5000, 200));
    let (agent_service, _) = create_test_service_with_provider(provider).await;

    let request = LLMRequest::new("mock-model".to_string(), vec![Message::user("Hello")]);
    let (response, _) = agent_service
        .stream_complete(Uuid::nil(), request, None, None, None, None)
        .await
        .unwrap();

    assert_eq!(
        response.usage.input_tokens, 5000,
        "inline usage should be captured from MessageStart or MessageDelta"
    );
    assert_eq!(
        response.usage.output_tokens, 200,
        "inline usage should capture output tokens"
    );
    assert_eq!(response.stop_reason, Some(StopReason::EndTurn));
}

#[tokio::test]
async fn test_deferred_usage_zero_start_overridden_by_real() {
    // Simulates MiniMax: MessageStart has 0 tokens, real usage comes later
    let provider = Arc::new(MockDeferredUsageProvider::new(42000, 500));
    let (agent_service, _) = create_test_service_with_provider(provider).await;

    let request = LLMRequest::new("mock-model".to_string(), vec![Message::user("Hello")]);
    let (response, _) = agent_service
        .stream_complete(Uuid::nil(), request, None, None, None, None)
        .await
        .unwrap();

    assert_eq!(
        response.usage.input_tokens, 42000,
        "zero from MessageStart must be overridden by deferred usage chunk"
    );
    assert_eq!(response.usage.output_tokens, 500);
}

#[tokio::test]
async fn test_deferred_usage_with_tool_calls() {
    /// Provider that returns tool_use on first call with deferred usage
    struct DeferredToolProvider {
        call_count: std::sync::Mutex<usize>,
    }

    impl DeferredToolProvider {
        fn new() -> Self {
            Self {
                call_count: std::sync::Mutex::new(0),
            }
        }
    }

    #[async_trait]
    impl Provider for DeferredToolProvider {
        async fn complete(
            &self,
            _request: LLMRequest,
        ) -> crate::brain::provider::Result<LLMResponse> {
            let mut count = self.call_count.lock().unwrap();
            *count += 1;
            if *count == 1 {
                Ok(LLMResponse {
                    id: "resp-1".to_string(),
                    model: "mock-model".to_string(),
                    content: vec![
                        ContentBlock::Text {
                            text: "Using tool".to_string(),
                        },
                        ContentBlock::ToolUse {
                            id: "t1".to_string(),
                            name: "test_tool".to_string(),
                            input: serde_json::json!({"message": "hi"}),
                        },
                    ],
                    stop_reason: Some(StopReason::ToolUse),
                    usage: TokenUsage {
                        input_tokens: 8000,
                        output_tokens: 100,
                        ..Default::default()
                    },
                })
            } else {
                Ok(LLMResponse {
                    id: "resp-2".to_string(),
                    model: "mock-model".to_string(),
                    content: vec![ContentBlock::Text {
                        text: "Done.".to_string(),
                    }],
                    stop_reason: Some(StopReason::EndTurn),
                    usage: TokenUsage {
                        input_tokens: 9500,
                        output_tokens: 50,
                        ..Default::default()
                    },
                })
            }
        }

        async fn stream(
            &self,
            _request: LLMRequest,
        ) -> crate::brain::provider::Result<ProviderStream> {
            use crate::brain::provider::{ContentDelta, MessageDelta, StreamEvent, StreamMessage};

            let mut count = self.call_count.lock().unwrap();
            *count += 1;
            let call_num = *count;

            let (input_tok, output_tok, content, stop, tool) = if call_num == 1 {
                (
                    8000u32,
                    100u32,
                    "Using tool",
                    StopReason::ToolUse,
                    Some(("t1", "test_tool", serde_json::json!({"message": "hi"}))),
                )
            } else {
                (9500, 50, "Done.", StopReason::EndTurn, None)
            };

            let mut events = vec![
                Ok(StreamEvent::MessageStart {
                    message: StreamMessage {
                        id: format!("resp-{}", call_num),
                        model: "mock-model".to_string(),
                        role: Role::Assistant,
                        usage: TokenUsage {
                            input_tokens: 0,
                            output_tokens: 0,
                            ..Default::default()
                        },
                    },
                }),
                Ok(StreamEvent::ContentBlockStart {
                    index: 0,
                    content_block: ContentBlock::Text {
                        text: String::new(),
                    },
                }),
                Ok(StreamEvent::ContentBlockDelta {
                    index: 0,
                    delta: ContentDelta::TextDelta {
                        text: content.to_string(),
                    },
                }),
                Ok(StreamEvent::ContentBlockStop { index: 0 }),
            ];

            if let Some((id, name, input)) = tool {
                events.push(Ok(StreamEvent::ContentBlockStart {
                    index: 1,
                    content_block: ContentBlock::ToolUse {
                        id: id.to_string(),
                        name: name.to_string(),
                        input: serde_json::Value::Object(Default::default()),
                    },
                }));
                events.push(Ok(StreamEvent::ContentBlockDelta {
                    index: 1,
                    delta: ContentDelta::InputJsonDelta {
                        partial_json: serde_json::to_string(&input).unwrap(),
                    },
                }));
                events.push(Ok(StreamEvent::ContentBlockStop { index: 1 }));
            }

            // Deferred: stop_reason without usage, then usage without stop_reason
            events.push(Ok(StreamEvent::MessageDelta {
                delta: MessageDelta {
                    stop_reason: Some(stop),
                    stop_sequence: None,
                },
                usage: TokenUsage {
                    input_tokens: 0,
                    output_tokens: 0,
                    ..Default::default()
                },
            }));
            events.push(Ok(StreamEvent::MessageDelta {
                delta: MessageDelta {
                    stop_reason: None,
                    stop_sequence: None,
                },
                usage: TokenUsage {
                    input_tokens: input_tok,
                    output_tokens: output_tok,
                    ..Default::default()
                },
            }));
            events.push(Ok(StreamEvent::MessageStop));

            Ok(Box::pin(futures::stream::iter(events)))
        }

        fn name(&self) -> &str {
            "mock-deferred-tool"
        }

        fn default_model(&self) -> &str {
            "mock-model"
        }

        fn supported_models(&self) -> Vec<String> {
            vec!["mock-model".to_string()]
        }

        fn context_window(&self, _model: &str) -> Option<u32> {
            Some(200_000)
        }

        fn calculate_cost(&self, _model: &str, _input: u32, _output: u32) -> f64 {
            0.001
        }
    }

    let provider = Arc::new(DeferredToolProvider::new());
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let context = ServiceContext::new(db.pool().clone());

    let registry = ToolRegistry::new();
    registry.register(Arc::new(MockTool));

    let agent_service = AgentService::new_for_test(provider, context.clone())
        .with_tool_registry(Arc::new(registry))
        .with_auto_approve_tools(true);

    let session_service = SessionService::new(context);
    let session = session_service
        .create_session(Some("Test".to_string()))
        .await
        .unwrap();

    let response = agent_service
        .send_message_with_tools_and_mode(session.id, "Use the tool".to_string(), None, None)
        .await
        .unwrap();

    // Final response should have the second call's usage (9500 input)
    assert!(
        response.usage.input_tokens > 0,
        "tool loop with deferred usage must report non-zero input tokens"
    );
    assert!(
        response.context_tokens > 0,
        "context_tokens must be non-zero after deferred usage tool loop"
    );
}

#[tokio::test]
async fn test_deferred_usage_content_preserved() {
    let provider = Arc::new(MockDeferredUsageProvider::new(10000, 100));
    let (agent_service, _) = create_test_service_with_provider(provider).await;

    let request = LLMRequest::new("mock-model".to_string(), vec![Message::user("Hello")]);
    let (response, _) = agent_service
        .stream_complete(Uuid::nil(), request, None, None, None, None)
        .await
        .unwrap();

    let text = response
        .content
        .iter()
        .find_map(|b| {
            if let ContentBlock::Text { text } = b {
                Some(text.as_str())
            } else {
                None
            }
        })
        .unwrap();

    assert_eq!(
        text, "Hello from deferred usage provider",
        "content must not be corrupted by deferred usage flow"
    );
}
