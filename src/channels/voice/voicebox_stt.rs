//! Voicebox STT provider.
//!
//! POST to `/transcribe` endpoint with audio file.
//! Returns transcription text.

use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use url::Url;

use super::openai_tts::build_endpoint_url;

/// Transcribe audio bytes using Voicebox.
///
/// POST audio file to `/transcribe` endpoint.
/// Returns the transcribed text.
pub async fn transcribe(audio_bytes: Vec<u8>, base_url: &str) -> Result<String> {
    let transcribe_url = build_endpoint_url(base_url, "transcribe")?;

    let client = Client::new();

    let file_part = reqwest::multipart::Part::bytes(audio_bytes)
        .file_name("voice.ogg")
        .mime_str("audio/ogg")?;

    let form = reqwest::multipart::Form::new().part("file", file_part);

    let response = client
        .post(&transcribe_url)
        .multipart(form)
        .send()
        .await
        .context("Failed to send audio to Voicebox STT")?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response.text().await.unwrap_or_default();
        anyhow::bail!("Voicebox STT error ({}): {}", status, error_text);
    }

    let result: TranscribeResponse = response
        .json()
        .await
        .context("Failed to parse Voicebox STT response")?;

    tracing::info!(
        "Voicebox STT: transcribed {} chars (url={})",
        result.text.len(),
        transcribe_url,
    );

    Ok(result.text)
}

#[derive(Deserialize)]
struct TranscribeResponse {
    text: String,
}
