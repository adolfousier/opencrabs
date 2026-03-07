//! Provider Vision Tool
//!
//! Analyzes images using the provider's own vision-capable model via
//! OpenAI-compatible API. Registered as `analyze_image` when Gemini vision
//! isn't configured but the active provider has a `vision_model` set.

use super::analyze_image::{base64_encode, detect_mime_type};
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

/// Image vision/analysis tool using the provider's own vision model.
pub struct ProviderVisionTool {
    api_key: String,
    base_url: String,
    vision_model: String,
}

impl ProviderVisionTool {
    pub fn new(api_key: String, base_url: String, vision_model: String) -> Self {
        Self {
            api_key,
            base_url,
            vision_model,
        }
    }
}

#[async_trait]
impl Tool for ProviderVisionTool {
    fn name(&self) -> &str {
        "analyze_image"
    }

    fn description(&self) -> &str {
        "Analyze an image file (local path) or URL using the provider's vision model. \
         Use when: the current model doesn't support vision, you need to analyze a saved file, \
         or the user sends an image. The vision model describes the image so you can understand it."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "image": {
                    "type": "string",
                    "description": "Local file path (e.g. /home/user/photo.png) or HTTPS URL to the image"
                },
                "question": {
                    "type": "string",
                    "description": "What to ask about the image. Defaults to 'Describe this image in detail.'"
                }
            },
            "required": ["image"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::Network, ToolCapability::ReadFiles]
    }

    fn requires_approval(&self) -> bool {
        false
    }

    async fn execute(
        &self,
        input: Value,
        _context: &ToolExecutionContext,
    ) -> super::error::Result<ToolResult> {
        let image_src = match input["image"].as_str() {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => {
                return Ok(ToolResult::error(
                    "Missing required parameter: image".to_string(),
                ));
            }
        };

        let question = input["question"]
            .as_str()
            .unwrap_or("Describe this image in detail.")
            .to_string();

        // Build image_url content part
        let image_url = if image_src.starts_with("http://") || image_src.starts_with("https://") {
            image_src.clone()
        } else {
            // Local file — read and base64 encode
            let bytes = tokio::fs::read(&image_src).await.map_err(|e| {
                super::error::ToolError::Execution(format!(
                    "Failed to read image file '{}': {}",
                    image_src, e
                ))
            })?;
            let mime = detect_mime_type(&image_src);
            let b64 = base64_encode(&bytes);
            format!("data:{};base64,{}", mime, b64)
        };

        // Build OpenAI-compatible vision request
        let body = serde_json::json!({
            "model": self.vision_model,
            "messages": [{
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": question
                    },
                    {
                        "type": "image_url",
                        "image_url": { "url": image_url }
                    }
                ]
            }],
            "max_tokens": 1024
        });

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| super::error::ToolError::Execution(e.to_string()))?;

        let response = client
            .post(&self.base_url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| super::error::ToolError::Execution(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let err_body = response.text().await.unwrap_or_default();
            return Ok(ToolResult::error(format!(
                "Vision API error {}: {}",
                status, err_body
            )));
        }

        let json: Value = response
            .json()
            .await
            .map_err(|e| super::error::ToolError::Execution(e.to_string()))?;

        // Extract text from OpenAI-compatible response
        let result_text = json["choices"]
            .as_array()
            .and_then(|choices| choices.first())
            .and_then(|choice| choice["message"]["content"].as_str())
            .unwrap_or("")
            .to_string();

        if result_text.is_empty() {
            Ok(ToolResult::error(
                "No text response from vision model".to_string(),
            ))
        } else {
            Ok(ToolResult::success(result_text))
        }
    }
}
