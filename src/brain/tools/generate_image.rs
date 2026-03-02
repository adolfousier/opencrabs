//! Generate Image Tool
//!
//! Generates images from text prompts using Google Gemini's image generation API.
//! Saves the result as a PNG file in ~/.opencrabs/images/ and returns the path.

use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// Image generation tool using Google Gemini
pub struct GenerateImageTool {
    api_key: String,
    model: String,
}

impl GenerateImageTool {
    pub fn new(api_key: String, model: String) -> Self {
        Self { api_key, model }
    }
}

#[async_trait]
impl Tool for GenerateImageTool {
    fn name(&self) -> &str {
        "generate_image"
    }

    fn description(&self) -> &str {
        "Generate an image from a text prompt using Google Gemini. Returns the file path to the saved PNG. Use <<IMG:path>> syntax in your reply to send the image through a channel."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "Text description of the image to generate"
                },
                "filename": {
                    "type": "string",
                    "description": "Optional filename (without path). Defaults to a UUID-based name."
                }
            },
            "required": ["prompt"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::Network, ToolCapability::WriteFiles]
    }

    fn requires_approval(&self) -> bool {
        false
    }

    async fn execute(
        &self,
        input: Value,
        _context: &ToolExecutionContext,
    ) -> super::error::Result<ToolResult> {
        let prompt = match input["prompt"].as_str() {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => {
                return Ok(ToolResult::error(
                    "Missing required parameter: prompt".to_string(),
                ));
            }
        };

        let filename = input["filename"]
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("{}.png", uuid::Uuid::new_v4().simple()));

        // Ensure images directory exists
        let images_dir = crate::config::opencrabs_home().join("images");
        if let Err(e) = tokio::fs::create_dir_all(&images_dir).await {
            return Ok(ToolResult::error(format!(
                "Failed to create images directory: {}",
                e
            )));
        }

        let save_path = images_dir.join(&filename);

        // Build Gemini request for image generation
        let url = format!(
            "{}/models/{}:generateContent?key={}",
            GEMINI_BASE_URL, self.model, self.api_key
        );

        let body = serde_json::json!({
            "contents": [{"parts": [{"text": prompt}]}],
            "generationConfig": {
                "responseModalities": ["TEXT", "IMAGE"]
            }
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

        // Find inlineData (base64 image) in response parts
        let empty_vec = vec![];
        let candidates = json["candidates"].as_array().unwrap_or(&empty_vec);
        let mut image_data: Option<String> = None;
        let mut text_response = String::new();

        'outer: for candidate in candidates {
            let empty_parts = vec![];
            let parts = candidate["content"]["parts"]
                .as_array()
                .unwrap_or(&empty_parts);
            for part in parts {
                if let Some(text) = part["text"].as_str() {
                    text_response.push_str(text);
                }
                if let Some(data) = part["inlineData"]["data"].as_str() {
                    image_data = Some(data.to_string());
                    break 'outer;
                }
            }
        }

        match image_data {
            Some(b64) => {
                // Decode base64 and save to file
                let bytes = match base64_decode(&b64) {
                    Ok(b) => b,
                    Err(e) => {
                        return Ok(ToolResult::error(format!(
                            "Failed to decode image data: {}",
                            e
                        )));
                    }
                };

                tokio::fs::write(&save_path, &bytes)
                    .await
                    .map_err(|e| super::error::ToolError::Execution(e.to_string()))?;

                let path_str = save_path.to_string_lossy().to_string();
                let mut output = format!(
                    "Generated image saved to: {}\nUse <<IMG:{}>> to reference it.",
                    path_str, path_str
                );
                if !text_response.is_empty() {
                    output = format!("{}\n\n{}", text_response.trim(), output);
                }
                Ok(ToolResult::success(output))
            }
            None => {
                // Gemini might return text only (model doesn't support image gen for this prompt)
                if !text_response.is_empty() {
                    Ok(ToolResult::success(format!(
                        "No image generated. Gemini response: {}",
                        text_response
                    )))
                } else {
                    Ok(ToolResult::error(
                        "No image data found in Gemini response".to_string(),
                    ))
                }
            }
        }
    }
}

fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    // Use base64 via the standard approach — decode without padding issues
    let clean: String = input
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '+' || *c == '/' || *c == '=')
        .collect();
    base64_decode_inner(&clean)
}

fn base64_decode_inner(input: &str) -> Result<Vec<u8>, String> {
    // Simple base64 decode without external crate (reqwest already depends on base64 indirectly)
    // Use the engine from the existing base64 crate that reqwest pulls in
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(input)
        .map_err(|e| e.to_string())
}
