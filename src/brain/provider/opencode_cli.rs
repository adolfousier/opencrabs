//! OpenCode CLI Provider — direct subprocess integration
//!
//! Spawns the `opencode` CLI binary in non-interactive mode and reads
//! its NDJSON stream output, converting it to standard `StreamEvent`s.
//! OpenCrabs handles all tools, memory, and context locally — opencode
//! is used purely as an LLM backend for its model access (including free models).

use super::error::{ProviderError, Result};
use super::r#trait::{Provider, ProviderStream};
use super::types::*;
use async_trait::async_trait;
use futures::stream::StreamExt;
use serde::Deserialize;
use std::process::Stdio;
use tokio::io::AsyncBufReadExt;

/// OpenCode CLI provider — talks directly to the `opencode` binary.
#[derive(Clone)]
pub struct OpenCodeCliProvider {
    opencode_path: String,
    default_model: String,
}

impl OpenCodeCliProvider {
    /// Create a new provider, auto-detecting the opencode binary.
    pub fn new() -> Result<Self> {
        let path = resolve_opencode_path()?;
        Ok(Self {
            opencode_path: path,
            default_model: "opencode/gpt-5-nano".to_string(),
        })
    }

    /// Override the default model.
    pub fn with_default_model(mut self, model: String) -> Self {
        self.default_model = model;
        self
    }

    /// Build a plain-text prompt from LLMRequest messages.
    fn build_prompt(request: &LLMRequest) -> String {
        let mut parts = Vec::new();

        if let Some(ref system) = request.system
            && !system.is_empty()
        {
            parts.push(system.clone());
        }

        for msg in &request.messages {
            let role = match msg.role {
                Role::User => "Human",
                Role::Assistant => "Assistant",
                Role::System => "System",
            };
            let content: String = msg
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.clone()),
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        ..
                    } => Some(format!("[tool_result for {}]: {}", tool_use_id, content)),
                    ContentBlock::ToolUse { id, name, input } => {
                        Some(format!("[tool_use {} ({}): {}]", name, id, input))
                    }
                    ContentBlock::Thinking { thinking, .. } => {
                        if thinking.is_empty() {
                            None
                        } else {
                            Some(format!("<thinking>{}</thinking>", thinking))
                        }
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");

            if content.trim().is_empty() {
                continue;
            }
            parts.push(format!("{}: {}", role, content));
        }

        parts.join("\n\n")
    }
}

/// Resolve the opencode CLI binary path.
fn resolve_opencode_path() -> Result<String> {
    if let Ok(path) = std::env::var("OPENCODE_PATH") {
        if std::path::Path::new(&path).exists() {
            return Ok(path);
        }
        return Err(ProviderError::Internal(format!(
            "OPENCODE_PATH set but not found: {}",
            path
        )));
    }

    // Check common locations
    let home = dirs::home_dir().unwrap_or_default();
    let candidates = [
        home.join(".opencode/bin/opencode"),
        std::path::PathBuf::from("/opt/homebrew/bin/opencode"),
        std::path::PathBuf::from("/usr/local/bin/opencode"),
    ];

    for candidate in &candidates {
        if candidate.exists() {
            return Ok(candidate.to_string_lossy().to_string());
        }
    }

    // Try PATH via `which`
    if let Ok(output) = std::process::Command::new("which")
        .arg("opencode")
        .output()
        && output.status.success()
    {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Ok(path);
        }
    }

    Err(ProviderError::Internal(
        "opencode CLI not found — install it or set OPENCODE_PATH".to_string(),
    ))
}

// ── CLI NDJSON types ──

/// A parsed NDJSON event from opencode CLI stdout.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CliEvent {
    StepStart {
        #[allow(dead_code)]
        part: serde_json::Value,
    },
    Text {
        part: TextPart,
    },
    Reasoning {
        part: ReasoningPart,
    },
    StepFinish {
        part: StepFinishPart,
    },
    Error {
        error: CliError,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
struct TextPart {
    pub text: String,
}

#[derive(Debug, Deserialize)]
struct ReasoningPart {
    pub text: String,
}

#[derive(Debug, Deserialize)]
struct StepFinishPart {
    pub reason: Option<String>,
    #[serde(default)]
    pub tokens: Option<CliTokens>,
}

#[derive(Debug, Deserialize)]
struct CliTokens {
    #[serde(default)]
    pub input: u32,
    #[serde(default)]
    pub output: u32,
    #[serde(default)]
    pub reasoning: u32,
}

#[derive(Debug, Deserialize)]
struct CliError {
    #[serde(default)]
    pub data: Option<CliErrorData>,
    #[allow(dead_code)]
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CliErrorData {
    pub message: Option<String>,
}

#[async_trait]
impl Provider for OpenCodeCliProvider {
    async fn complete(&self, request: LLMRequest) -> Result<LLMResponse> {
        let mut stream = self.stream(request).await?;

        let mut id = String::new();
        let mut model = String::new();
        let mut content = Vec::new();
        let mut stop_reason = None;
        let mut usage = TokenUsage {
            input_tokens: 0,
            output_tokens: 0,
        };
        let mut text_buf = String::new();

        while let Some(event) = stream.next().await {
            match event? {
                StreamEvent::MessageStart { message } => {
                    id = message.id;
                    model = message.model;
                    usage.input_tokens = message.usage.input_tokens;
                }
                StreamEvent::ContentBlockDelta {
                    delta: ContentDelta::TextDelta { text },
                    ..
                } => {
                    text_buf.push_str(&text);
                }
                StreamEvent::MessageDelta { delta: d, usage: u } => {
                    stop_reason = d.stop_reason;
                    usage.output_tokens = u.output_tokens;
                }
                StreamEvent::MessageStop => break,
                _ => {}
            }
        }

        if !text_buf.is_empty() {
            content.push(ContentBlock::Text { text: text_buf });
        }

        Ok(LLMResponse {
            id,
            model,
            content,
            stop_reason,
            usage,
        })
    }

    async fn stream(&self, request: LLMRequest) -> Result<ProviderStream> {
        let prompt = Self::build_prompt(&request);
        let model = request.model.clone();

        let cwd = request
            .working_directory
            .as_deref()
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_dir())
            .unwrap_or_else(|| {
                dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/"))
            });

        tracing::info!(
            "Spawning opencode CLI: model={}, prompt_len={}, cwd={}",
            model,
            prompt.len(),
            cwd.display()
        );

        let mut cmd = tokio::process::Command::new(&self.opencode_path);
        cmd.arg("run")
            .arg("--format")
            .arg("json")
            .arg("--thinking")
            .arg("--model")
            .arg(&model)
            .arg("--")
            .arg(&prompt)
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| ProviderError::Internal(format!("failed to spawn opencode CLI: {}", e)))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ProviderError::Internal("failed to capture stdout".to_string()))?;

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| ProviderError::Internal("failed to capture stderr".to_string()))?;

        // Log stderr
        tokio::spawn(async move {
            let reader = tokio::io::BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim().to_string();
                if !line.is_empty() {
                    tracing::warn!("opencode CLI stderr: {}", line);
                }
            }
        });

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<StreamEvent>>(64);
        let model_for_task = model.clone();

        tokio::spawn(async move {
            let reader = tokio::io::BufReader::new(stdout);
            let mut lines = reader.lines();
            let mut started = false;
            let mut block_index: usize = 0;

            loop {
                let line = match lines.next_line().await {
                    Ok(Some(line)) => line,
                    Ok(None) => break,
                    Err(e) => {
                        tracing::error!("opencode CLI stdout read error: {}", e);
                        break;
                    }
                };
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }

                tracing::debug!(
                    "opencode CLI raw: {}",
                    &line[..line.floor_char_boundary(300)]
                );

                let event: CliEvent = match serde_json::from_str(&line) {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::warn!(
                            "Skipping unparseable opencode line: {} — {}",
                            e,
                            &line[..line.floor_char_boundary(200)]
                        );
                        continue;
                    }
                };

                match event {
                    CliEvent::StepStart { .. } => {
                        if !started {
                            started = true;
                            let msg_id =
                                format!("msg_{}", uuid::Uuid::new_v4().simple());
                            let _ = tx
                                .send(Ok(StreamEvent::MessageStart {
                                    message: StreamMessage {
                                        id: msg_id,
                                        model: model_for_task.clone(),
                                        role: Role::Assistant,
                                        usage: TokenUsage {
                                            input_tokens: 0,
                                            output_tokens: 0,
                                        },
                                    },
                                }))
                                .await;
                        }
                    }

                    CliEvent::Reasoning { part } => {
                        if !started {
                            started = true;
                            let msg_id =
                                format!("msg_{}", uuid::Uuid::new_v4().simple());
                            let _ = tx
                                .send(Ok(StreamEvent::MessageStart {
                                    message: StreamMessage {
                                        id: msg_id,
                                        model: model_for_task.clone(),
                                        role: Role::Assistant,
                                        usage: TokenUsage {
                                            input_tokens: 0,
                                            output_tokens: 0,
                                        },
                                    },
                                }))
                                .await;
                        }

                        // Emit thinking block: start + delta + stop
                        let _ = tx
                            .send(Ok(StreamEvent::ContentBlockStart {
                                index: block_index,
                                content_block: ContentBlock::Thinking {
                                    thinking: String::new(),
                                    signature: None,
                                },
                            }))
                            .await;
                        let _ = tx
                            .send(Ok(StreamEvent::ContentBlockDelta {
                                index: block_index,
                                delta: ContentDelta::ThinkingDelta {
                                    thinking: part.text,
                                },
                            }))
                            .await;
                        let _ = tx
                            .send(Ok(StreamEvent::ContentBlockStop {
                                index: block_index,
                            }))
                            .await;
                        block_index += 1;
                    }

                    CliEvent::Text { part } => {
                        if !started {
                            started = true;
                            let msg_id =
                                format!("msg_{}", uuid::Uuid::new_v4().simple());
                            let _ = tx
                                .send(Ok(StreamEvent::MessageStart {
                                    message: StreamMessage {
                                        id: msg_id,
                                        model: model_for_task.clone(),
                                        role: Role::Assistant,
                                        usage: TokenUsage {
                                            input_tokens: 0,
                                            output_tokens: 0,
                                        },
                                    },
                                }))
                                .await;
                        }

                        // Emit text block: start + delta + stop
                        let _ = tx
                            .send(Ok(StreamEvent::ContentBlockStart {
                                index: block_index,
                                content_block: ContentBlock::Text {
                                    text: String::new(),
                                },
                            }))
                            .await;
                        let _ = tx
                            .send(Ok(StreamEvent::ContentBlockDelta {
                                index: block_index,
                                delta: ContentDelta::TextDelta { text: part.text },
                            }))
                            .await;
                        let _ = tx
                            .send(Ok(StreamEvent::ContentBlockStop {
                                index: block_index,
                            }))
                            .await;
                        block_index += 1;
                    }

                    CliEvent::StepFinish { part } => {
                        let reason = part.reason.map(|r| match r.as_str() {
                            "stop" | "end_turn" => StopReason::EndTurn,
                            "tool_use" => StopReason::ToolUse,
                            "max_tokens" => StopReason::MaxTokens,
                            _ => StopReason::EndTurn,
                        });

                        let (input_tokens, output_tokens) = part
                            .tokens
                            .map(|t| (t.input, t.output + t.reasoning))
                            .unwrap_or((0, 0));

                        let _ = tx
                            .send(Ok(StreamEvent::MessageDelta {
                                delta: MessageDelta {
                                    stop_reason: reason,
                                    stop_sequence: None,
                                },
                                usage: TokenUsage {
                                    input_tokens,
                                    output_tokens,
                                },
                            }))
                            .await;
                        let _ = tx.send(Ok(StreamEvent::MessageStop)).await;
                        break;
                    }

                    CliEvent::Error { error } => {
                        let msg = error
                            .data
                            .and_then(|d| d.message)
                            .unwrap_or_else(|| "opencode CLI error".to_string());
                        tracing::error!("opencode CLI error: {}", msg);
                        let _ = tx
                            .send(Err(ProviderError::ApiError {
                                status: 500,
                                message: msg,
                                error_type: Some("opencode_error".to_string()),
                            }))
                            .await;
                        break;
                    }

                    CliEvent::Unknown => {}
                }
            }

            // Wait for process exit
            let exit_status = child.wait().await;
            match &exit_status {
                Ok(status) if !status.success() => {
                    tracing::warn!("opencode CLI exited with status: {}", status);
                    if !started {
                        let _ = tx
                            .send(Err(ProviderError::Internal(format!(
                                "opencode CLI exited with {} before producing any output",
                                status
                            ))))
                            .await;
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to wait on opencode CLI: {}", e);
                }
                Ok(_) => {
                    if !started {
                        tracing::warn!(
                            "opencode CLI exited successfully but produced no stream events"
                        );
                    }
                }
            }
        });

        let stream = futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(|item| (item, rx))
        });
        Ok(Box::pin(stream))
    }

    fn name(&self) -> &str {
        "opencode"
    }

    fn default_model(&self) -> &str {
        &self.default_model
    }

    fn supported_models(&self) -> Vec<String> {
        vec![
            "opencode/big-pickle".to_string(),
            "opencode/gpt-5-nano".to_string(),
            "opencode/mimo-v2-omni-free".to_string(),
            "opencode/mimo-v2-pro-free".to_string(),
            "opencode/minimax-m2.5-free".to_string(),
            "opencode/nemotron-3-super-free".to_string(),
        ]
    }

    fn context_window(&self, _model: &str) -> Option<u32> {
        Some(128_000) // Conservative default
    }

    fn calculate_cost(&self, model: &str, input_tokens: u32, output_tokens: u32) -> f64 {
        crate::pricing::PricingConfig::load().calculate_cost(model, input_tokens, output_tokens)
    }

    fn supports_tools(&self) -> bool {
        false // OpenCrabs handles tools — opencode is just the LLM pipe
    }

    fn supports_vision(&self) -> bool {
        false
    }
}
