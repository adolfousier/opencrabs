//! Analyze Image Tool
//!
//! Analyzes image files or URLs using vision-capable models.
//! Two backends: Google Gemini (default) or provider-native vision model
//! (uses the same OpenAI-compatible API with a vision-capable model).

use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// Image vision/analysis tool using Google Gemini
pub struct AnalyzeImageTool {
    api_key: String,
    model: String,
}

impl AnalyzeImageTool {
    pub fn new(api_key: String, model: String) -> Self {
        Self { api_key, model }
    }
}

#[async_trait]
impl Tool for AnalyzeImageTool {
    fn name(&self) -> &str {
        "analyze_image"
    }

    fn description(&self) -> &str {
        "Analyze an image file (local path) or URL using Google Gemini vision. \
         Use when: the current model doesn't support vision, you need to analyze a saved file, \
         or the user explicitly requests Google vision analysis."
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

        // Build the image part — either base64 from file or URL
        let image_part = if image_src.starts_with("http://") || image_src.starts_with("https://") {
            // Fetch URL and convert to base64
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| super::error::ToolError::Execution(e.to_string()))?;

            let resp = client
                .get(&image_src)
                .send()
                .await
                .map_err(|e| super::error::ToolError::Execution(e.to_string()))?;

            if !resp.status().is_success() {
                return Ok(ToolResult::error(format!(
                    "Failed to fetch image URL: HTTP {}",
                    resp.status()
                )));
            }

            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("image/jpeg")
                .to_string();
            let mime_type = content_type
                .split(';')
                .next()
                .unwrap_or("image/jpeg")
                .to_string();

            let bytes = resp
                .bytes()
                .await
                .map_err(|e| super::error::ToolError::Execution(e.to_string()))?;

            let b64 = base64_encode(&bytes);
            serde_json::json!({
                "inlineData": {
                    "mimeType": mime_type,
                    "data": b64
                }
            })
        } else {
            // Local file
            let bytes = tokio::fs::read(&image_src).await.map_err(|e| {
                super::error::ToolError::Execution(format!(
                    "Failed to read image file '{}': {}",
                    image_src, e
                ))
            })?;

            let mime_type = detect_mime_type(&image_src);
            let b64 = base64_encode(&bytes);
            serde_json::json!({
                "inlineData": {
                    "mimeType": mime_type,
                    "data": b64
                }
            })
        };

        // Build Gemini vision request
        let url = format!(
            "{}/models/{}:generateContent?key={}",
            GEMINI_BASE_URL, self.model, self.api_key
        );

        let body = serde_json::json!({
            "contents": [{
                "parts": [
                    image_part,
                    {"text": question}
                ]
            }]
        });

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| super::error::ToolError::Execution(e.to_string()))?;

        let response = client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| super::error::ToolError::Execution(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let err_body = response.text().await.unwrap_or_default();
            return Ok(ToolResult::error(format!(
                "Gemini API error {}: {}",
                status, err_body
            )));
        }

        let json: Value = response
            .json()
            .await
            .map_err(|e| super::error::ToolError::Execution(e.to_string()))?;

        // Extract text response
        let empty_vec = vec![];
        let candidates = json["candidates"].as_array().unwrap_or(&empty_vec);
        let mut result_text = String::new();

        for candidate in candidates {
            let empty_parts = vec![];
            let parts = candidate["content"]["parts"]
                .as_array()
                .unwrap_or(&empty_parts);
            for part in parts {
                if let Some(text) = part["text"].as_str() {
                    result_text.push_str(text);
                }
            }
        }

        if result_text.is_empty() {
            Ok(ToolResult::error(
                "No text response from Gemini vision".to_string(),
            ))
        } else {
            Ok(ToolResult::success(result_text))
        }
    }
}

pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub(crate) fn detect_mime_type(path: &str) -> &'static str {
    let lower = path.to_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".heic") || lower.ends_with(".heif") {
        "image/heic"
    } else {
        "image/jpeg"
    }
}
