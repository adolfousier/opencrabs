//! Fallback Provider Chain & Vision Model Tests
//!
//! Tests for the fallback provider chain configuration, runtime fallback
//! behavior, and per-provider vision model swapping.

// --- Fallback chain config ---

mod fallback_chain {
    use crate::brain::provider::factory::fallback_chain;
    use crate::config::FallbackProviderConfig;

    #[test]
    fn empty_config_returns_empty_chain() {
        let cfg = FallbackProviderConfig::default();
        assert!(fallback_chain(&cfg).is_empty());
    }

    #[test]
    fn legacy_single_provider() {
        let cfg = FallbackProviderConfig {
            enabled: true,
            provider: Some("openrouter".into()),
            providers: vec![],
        };
        assert_eq!(fallback_chain(&cfg), vec!["openrouter"]);
    }

    #[test]
    fn providers_array_only() {
        let cfg = FallbackProviderConfig {
            enabled: true,
            provider: None,
            providers: vec!["anthropic".into(), "openai".into()],
        };
        assert_eq!(fallback_chain(&cfg), vec!["anthropic", "openai"]);
    }

    #[test]
    fn array_plus_legacy_appended() {
        let cfg = FallbackProviderConfig {
            enabled: true,
            provider: Some("gemini".into()),
            providers: vec!["anthropic".into(), "openai".into()],
        };
        assert_eq!(fallback_chain(&cfg), vec!["anthropic", "openai", "gemini"]);
    }

    #[test]
    fn legacy_deduped_if_already_in_array() {
        let cfg = FallbackProviderConfig {
            enabled: true,
            provider: Some("anthropic".into()),
            providers: vec!["anthropic".into(), "openai".into()],
        };
        // "anthropic" already in array — should NOT be appended again
        assert_eq!(fallback_chain(&cfg), vec!["anthropic", "openai"]);
    }

    #[test]
    fn single_provider_in_array() {
        let cfg = FallbackProviderConfig {
            enabled: true,
            provider: None,
            providers: vec!["minimax".into()],
        };
        assert_eq!(fallback_chain(&cfg), vec!["minimax"]);
    }

    #[test]
    fn deserialization_from_toml_array() {
        let toml_str = r#"
enabled = true
providers = ["openrouter", "anthropic"]
"#;
        let cfg: FallbackProviderConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.providers, vec!["openrouter", "anthropic"]);
        assert!(cfg.provider.is_none());
    }

    #[test]
    fn deserialization_from_toml_legacy() {
        let toml_str = r#"
enabled = true
provider = "openrouter"
"#;
        let cfg: FallbackProviderConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.provider, Some("openrouter".into()));
        assert!(cfg.providers.is_empty());
    }

    #[test]
    fn deserialization_from_toml_both() {
        let toml_str = r#"
enabled = true
provider = "gemini"
providers = ["anthropic", "openai"]
"#;
        let cfg: FallbackProviderConfig = toml::from_str(toml_str).unwrap();
        let chain = fallback_chain(&cfg);
        assert_eq!(chain, vec!["anthropic", "openai", "gemini"]);
    }
}

// --- Fallback provider runtime ---

mod fallback_runtime {
    use crate::brain::provider::{
        FallbackProvider, LLMRequest, LLMResponse, Provider, ProviderError, ProviderStream,
    };
    use async_trait::async_trait;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A mock provider that fails N times, then succeeds.
    struct MockProvider {
        name: String,
        fail_count: AtomicUsize,
        max_failures: usize,
    }

    impl MockProvider {
        fn always_fail(name: &str) -> Self {
            Self {
                name: name.to_string(),
                fail_count: AtomicUsize::new(0),
                max_failures: usize::MAX,
            }
        }

        fn always_succeed(name: &str) -> Self {
            Self {
                name: name.to_string(),
                fail_count: AtomicUsize::new(0),
                max_failures: 0,
            }
        }
    }

    #[async_trait]
    impl Provider for MockProvider {
        async fn complete(
            &self,
            _request: LLMRequest,
        ) -> crate::brain::provider::error::Result<LLMResponse> {
            let count = self.fail_count.fetch_add(1, Ordering::SeqCst);
            if count < self.max_failures {
                Err(ProviderError::Internal(format!(
                    "{} mock failure #{}",
                    self.name,
                    count + 1
                )))
            } else {
                Ok(LLMResponse {
                    id: format!("{}-response", self.name),
                    model: "mock-model".into(),
                    content: vec![],
                    stop_reason: None,
                    usage: crate::brain::provider::TokenUsage {
                        input_tokens: 0,
                        output_tokens: 0,
                    },
                })
            }
        }

        async fn stream(
            &self,
            _request: LLMRequest,
        ) -> crate::brain::provider::error::Result<ProviderStream> {
            let count = self.fail_count.fetch_add(1, Ordering::SeqCst);
            if count < self.max_failures {
                Err(ProviderError::Internal(format!(
                    "{} stream mock failure #{}",
                    self.name,
                    count + 1
                )))
            } else {
                Ok(Box::pin(futures::stream::empty()))
            }
        }

        fn name(&self) -> &str {
            &self.name
        }

        fn default_model(&self) -> &str {
            "mock-model"
        }

        fn supported_models(&self) -> Vec<String> {
            vec!["mock-model".into()]
        }

        fn context_window(&self, _model: &str) -> Option<u32> {
            Some(4096)
        }

        fn calculate_cost(&self, _model: &str, _input: u32, _output: u32) -> f64 {
            0.0
        }
    }

    fn mock_request() -> LLMRequest {
        LLMRequest {
            model: "mock-model".into(),
            messages: vec![],
            system: None,
            max_tokens: None,
            temperature: None,
            tools: None,
            stream: false,
            metadata: None,
            working_directory: None,
        }
    }

    #[tokio::test]
    async fn primary_succeeds_no_fallback_tried() {
        let primary = Arc::new(MockProvider::always_succeed("primary"));
        let fallback = Arc::new(MockProvider::always_succeed("fallback"));
        let provider = FallbackProvider::new(primary, vec![fallback.clone()]);

        let resp = provider.complete(mock_request()).await.unwrap();
        assert_eq!(resp.id, "primary-response");
        // Fallback should not have been called
        assert_eq!(fallback.fail_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn primary_fails_first_fallback_succeeds() {
        let primary = Arc::new(MockProvider::always_fail("primary"));
        let fb1 = Arc::new(MockProvider::always_succeed("fallback1"));
        let provider = FallbackProvider::new(primary, vec![fb1]);

        let resp = provider.complete(mock_request()).await.unwrap();
        assert_eq!(resp.id, "fallback1-response");
    }

    #[tokio::test]
    async fn primary_fails_first_fallback_fails_second_succeeds() {
        let primary = Arc::new(MockProvider::always_fail("primary"));
        let fb1 = Arc::new(MockProvider::always_fail("fallback1"));
        let fb2 = Arc::new(MockProvider::always_succeed("fallback2"));
        let provider = FallbackProvider::new(primary, vec![fb1, fb2]);

        let resp = provider.complete(mock_request()).await.unwrap();
        assert_eq!(resp.id, "fallback2-response");
    }

    #[tokio::test]
    async fn all_fail_returns_primary_error() {
        let primary = Arc::new(MockProvider::always_fail("primary"));
        let fb1 = Arc::new(MockProvider::always_fail("fallback1"));
        let fb2 = Arc::new(MockProvider::always_fail("fallback2"));
        let provider = FallbackProvider::new(primary, vec![fb1, fb2]);

        let err = provider.complete(mock_request()).await.unwrap_err();
        // Should return the primary error
        assert!(err.to_string().contains("primary"));
    }

    #[tokio::test]
    async fn no_fallbacks_primary_error_propagated() {
        let primary = Arc::new(MockProvider::always_fail("primary"));
        let provider = FallbackProvider::new(primary, vec![]);

        let err = provider.complete(mock_request()).await.unwrap_err();
        assert!(err.to_string().contains("primary"));
    }

    #[tokio::test]
    async fn stream_primary_fails_fallback_succeeds() {
        let primary = Arc::new(MockProvider::always_fail("primary"));
        let fb1 = Arc::new(MockProvider::always_succeed("fallback1"));
        let provider = FallbackProvider::new(primary, vec![fb1]);

        // Should not error — fallback stream succeeds
        let _stream = provider.stream(mock_request()).await.unwrap();
    }

    #[tokio::test]
    async fn stream_all_fail() {
        let primary = Arc::new(MockProvider::always_fail("primary"));
        let fb1 = Arc::new(MockProvider::always_fail("fallback1"));
        let provider = FallbackProvider::new(primary, vec![fb1]);

        match provider.stream(mock_request()).await {
            Ok(_) => panic!("Expected error when all providers fail"),
            Err(e) => assert!(e.to_string().contains("primary")),
        }
    }

    #[tokio::test]
    async fn delegates_name_to_primary() {
        let primary = Arc::new(MockProvider::always_succeed("my-primary"));
        let provider = FallbackProvider::new(primary, vec![]);
        assert_eq!(provider.name(), "my-primary");
    }

    #[tokio::test]
    async fn delegates_default_model_to_primary() {
        let primary = Arc::new(MockProvider::always_succeed("p"));
        let provider = FallbackProvider::new(primary, vec![]);
        assert_eq!(provider.default_model(), "mock-model");
    }
}

// --- Vision model ---

mod vision_model {
    use crate::brain::provider::Provider;
    use crate::brain::provider::custom_openai_compatible::OpenAIProvider;

    #[test]
    fn no_vision_model_by_default() {
        let provider = OpenAIProvider::new("test-key".into());
        assert!(!provider.supports_vision());
    }

    #[test]
    fn with_vision_model_enables_vision() {
        let provider =
            OpenAIProvider::new("test-key".into()).with_vision_model("gpt-5-nano".into());
        assert!(provider.supports_vision());
    }

    #[test]
    fn vision_model_accessor() {
        let provider =
            OpenAIProvider::new("test-key".into()).with_vision_model("gpt-5-nano".into());
        assert_eq!(provider.vision_model(), Some("gpt-5-nano"));

        let no_vision = OpenAIProvider::new("test-key".into());
        assert_eq!(no_vision.vision_model(), None);
    }

    #[test]
    fn vision_model_config_roundtrip() {
        let toml_str = r#"
enabled = true
api_key = "test"
default_model = "MiniMax-M2.7"
vision_model = "MiniMax-Text-01"
"#;
        let cfg: crate::config::ProviderConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.vision_model, Some("MiniMax-Text-01".into()));
        assert_eq!(cfg.default_model, Some("MiniMax-M2.7".into()));
    }

    #[test]
    fn vision_model_absent_in_config() {
        let toml_str = r#"
enabled = true
api_key = "test"
default_model = "gpt-4"
"#;
        let cfg: crate::config::ProviderConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.vision_model.is_none());
    }

    #[test]
    fn factory_config_wires_vision_model() {
        use crate::config::{Config, ProviderConfig, ProviderConfigs};

        let config = Config {
            providers: ProviderConfigs {
                openai: Some(ProviderConfig {
                    enabled: true,
                    api_key: Some("test-key".into()),
                    base_url: None,
                    default_model: Some("gpt-4".into()),
                    models: vec![],
                    vision_model: Some("gpt-5-nano".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };

        let provider = crate::brain::provider::factory::create_provider(&config).unwrap();
        assert!(provider.supports_vision());
    }

    #[test]
    fn factory_config_no_vision_model() {
        use crate::config::{Config, ProviderConfig, ProviderConfigs};

        let config = Config {
            providers: ProviderConfigs {
                openai: Some(ProviderConfig {
                    enabled: true,
                    api_key: Some("test-key".into()),
                    base_url: None,
                    default_model: Some("gpt-4".into()),
                    models: vec![],
                    vision_model: None,
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };

        let provider = crate::brain::provider::factory::create_provider(&config).unwrap();
        assert!(!provider.supports_vision());
    }
}

// --- Factory fallback wiring ---

mod factory_fallback {
    use crate::config::{Config, FallbackProviderConfig, ProviderConfig, ProviderConfigs};

    #[test]
    fn no_fallback_returns_primary_directly() {
        let config = Config {
            providers: ProviderConfigs {
                openai: Some(ProviderConfig {
                    enabled: true,
                    api_key: Some("test-key".into()),
                    base_url: None,
                    default_model: None,
                    models: vec![],
                    vision_model: None,
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };

        let provider = crate::brain::provider::factory::create_provider(&config).unwrap();
        assert_eq!(provider.name(), "openai");
    }

    #[test]
    fn fallback_disabled_returns_primary_directly() {
        let config = Config {
            providers: ProviderConfigs {
                openai: Some(ProviderConfig {
                    enabled: true,
                    api_key: Some("test-key".into()),
                    base_url: None,
                    default_model: None,
                    models: vec![],
                    vision_model: None,
                    ..Default::default()
                }),
                fallback: Some(FallbackProviderConfig {
                    enabled: false,
                    provider: Some("anthropic".into()),
                    providers: vec![],
                }),
                ..Default::default()
            },
            ..Default::default()
        };

        let provider = crate::brain::provider::factory::create_provider(&config).unwrap();
        // Should be plain openai, not wrapped in fallback
        assert_eq!(provider.name(), "openai");
    }

    #[test]
    fn no_provider_no_fallback_returns_placeholder() {
        let config = Config {
            providers: ProviderConfigs::default(),
            ..Default::default()
        };

        let provider = crate::brain::provider::factory::create_provider(&config).unwrap();
        assert_eq!(provider.name(), "none");
    }

    #[test]
    fn fallback_with_unconfigured_providers_skipped() {
        // Fallback lists providers that don't have API keys — should skip them gracefully
        let config = Config {
            providers: ProviderConfigs {
                fallback: Some(FallbackProviderConfig {
                    enabled: true,
                    provider: None,
                    providers: vec!["anthropic".into(), "openai".into()],
                }),
                ..Default::default()
            },
            ..Default::default()
        };

        // No providers configured at all — should end up with placeholder
        let provider = crate::brain::provider::factory::create_provider(&config).unwrap();
        assert_eq!(provider.name(), "none");
    }
}

// --- Active provider vision discovery ---

mod active_provider_vision {
    use crate::brain::provider::factory::active_provider_vision;
    use crate::config::{Config, ProviderConfig, ProviderConfigs};

    #[test]
    fn returns_none_when_no_providers() {
        let config = Config::default();
        assert!(active_provider_vision(&config).is_none());
    }

    #[test]
    fn returns_none_when_no_vision_model() {
        let config = Config {
            providers: ProviderConfigs {
                openai: Some(ProviderConfig {
                    enabled: true,
                    api_key: Some("key".into()),
                    base_url: None,
                    default_model: None,
                    models: vec![],
                    vision_model: None,
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(active_provider_vision(&config).is_none());
    }

    #[test]
    fn returns_vision_model_from_active_provider() {
        let config = Config {
            providers: ProviderConfigs {
                minimax: Some(ProviderConfig {
                    enabled: true,
                    api_key: Some("minimax-key".into()),
                    base_url: Some("https://api.minimax.io/v1".into()),
                    default_model: Some("MiniMax-M2.7".into()),
                    models: vec![],
                    vision_model: Some("MiniMax-Text-01".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };

        let result = active_provider_vision(&config);
        assert!(result.is_some());
        let (api_key, base_url, vision_model) = result.unwrap();
        assert_eq!(api_key, "minimax-key");
        assert!(base_url.contains("minimax"));
        assert_eq!(vision_model, "MiniMax-Text-01");
    }

    #[test]
    fn skips_disabled_provider() {
        let config = Config {
            providers: ProviderConfigs {
                minimax: Some(ProviderConfig {
                    enabled: false,
                    api_key: Some("key".into()),
                    base_url: Some("https://api.minimax.io/v1".into()),
                    default_model: None,
                    models: vec![],
                    vision_model: Some("MiniMax-Text-01".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(active_provider_vision(&config).is_none());
    }

    #[test]
    fn skips_provider_without_api_key() {
        let config = Config {
            providers: ProviderConfigs {
                openai: Some(ProviderConfig {
                    enabled: true,
                    api_key: None,
                    base_url: None,
                    default_model: None,
                    models: vec![],
                    vision_model: Some("gpt-5-nano".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(active_provider_vision(&config).is_none());
    }

    #[test]
    fn picks_first_provider_with_vision_by_priority() {
        let config = Config {
            providers: ProviderConfigs {
                minimax: Some(ProviderConfig {
                    enabled: true,
                    api_key: Some("minimax-key".into()),
                    base_url: Some("https://api.minimax.io/v1".into()),
                    default_model: None,
                    models: vec![],
                    vision_model: Some("MiniMax-Text-01".into()),
                    ..Default::default()
                }),
                openai: Some(ProviderConfig {
                    enabled: true,
                    api_key: Some("openai-key".into()),
                    base_url: None,
                    default_model: None,
                    models: vec![],
                    vision_model: Some("gpt-5-nano".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        };

        let (api_key, _, vision_model) = active_provider_vision(&config).unwrap();
        // Minimax has higher priority
        assert_eq!(api_key, "minimax-key");
        assert_eq!(vision_model, "MiniMax-Text-01");
    }
}
