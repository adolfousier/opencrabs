//! OpenAI-compatible STT provider.
//!
//! Supports any OpenAI-compatible `/v1/audio/transcriptions` endpoint, including:
//! - Official OpenAI API
//! - Groq (`https://api.groq.com/openai/v1`)
//! - Ollama (`http://localhost:11434/v1`)
//! - LocalAI, OMLX, etc.

use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;

use super::openai_tts::build_endpoint_url;

/// Transcribe audio bytes using an OpenAI-compatible STT endpoint.
///
/// # Arguments
/// * `audio_bytes` - OGG/Opus audio data
/// * `api_key` - API key for the endpoint
/// * `model` - Model name (e.g. "whisper-large-v3-turbo")
/// * `base_url` - Base URL of the endpoint
///
/// # Returns
/// Transcribed text.
pub async fn transcribe_audio(
    audio_bytes: Vec<u8>,
    api_key: &str,
    model: &str,
    base_url: &str,
) -> Result<String> {
    let transcribe_url = build_endpoint_url(base_url, "v1/audio/transcriptions")?;

    let client = Client::new();

    let model_owned = model.to_string();

    let file_part = reqwest::multipart::Part::bytes(audio_bytes)
        .file_name("voice.ogg")
        .mime_str("audio/ogg")?;

    let form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("model", model_owned)
        .text("response_format", "json");

    let response = client
        .post(&transcribe_url)
        .header("Authorization", format!("Bearer {}", api_key))
        .multipart(form)
        .send()
        .await
        .context("Failed to send audio to STT endpoint")?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response.text().await.unwrap_or_default();
        anyhow::bail!("STT error ({}): {}", status, error_text);
    }

    let result: TranscriptionResponse = response
        .json()
        .await
        .context("Failed to parse transcription response")?;

    tracing::info!(
        "OpenAI-compatible STT: transcribed {} chars (model={}, url={})",
        result.text.len(),
        model,
        transcribe_url,
    );

    Ok(result.text)
}

#[derive(Deserialize)]
struct TranscriptionResponse {
    text: String,
}
