//! Voicebox TTS provider.
//!
//! POST to `/generate` endpoint, get back audio_path, read the audio file.
//! Supports voice cloning via profile_id.

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::Path;

use super::openai_tts::build_endpoint_url;

/// Voicebox TTS client.
pub struct VoiceboxTts {
    client: Client,
    base_url: String,
    profile_id: String,
}

impl VoiceboxTts {
    /// Create a new Voicebox TTS client.
    pub fn new(base_url: &str, profile_id: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.to_string(),
            profile_id: profile_id.to_string(),
        }
    }

    /// Synthesize speech using Voicebox.
    ///
    /// Flow: POST /generate → get audio_path → fs::read → return bytes.
    pub async fn synthesize(&self, text: &str) -> Result<Vec<u8>> {
        if text.is_empty() {
            anyhow::bail!("Cannot synthesize empty text");
        }

        let generate_url = build_endpoint_url(&self.base_url, "generate")?;

        let request = GenerateRequest {
            profile_id: self.profile_id.clone(),
            text: text.to_string(),
        };

        let response = self
            .client
            .post(&generate_url)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .context("Failed to send Voicebox TTS request")?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Voicebox TTS error ({}): {}", status, error_text);
        }

        let result: GenerateResponse = response
            .json()
            .await
            .context("Failed to parse Voicebox TTS response")?;

        // Read audio file from the returned path
        let audio_path = Path::new(&result.audio_path);
        if !audio_path.exists() {
            anyhow::bail!("Voicebox audio file not found: {}", result.audio_path);
        }

        let audio_bytes =
            tokio::fs::read(audio_path)
                .await
                .with_context(|| format!("Failed to read Voicebox audio file: {}", result.audio_path))?;

        tracing::info!(
            "Voicebox TTS: generated {} bytes of audio (profile={}, duration={:.2}s, path={})",
            audio_bytes.len(),
            result.profile_id,
            result.duration,
            result.audio_path,
        );

        Ok(audio_bytes)
    }
}

#[derive(Serialize)]
struct GenerateRequest {
    profile_id: String,
    text: String,
}

#[derive(Deserialize)]
struct GenerateResponse {
    #[allow(dead_code)]
    id: String,
    profile_id: String,
    #[allow(dead_code)]
    text: String,
    #[allow(dead_code)]
    language: String,
    audio_path: String,
    duration: f64,
    #[allow(dead_code)]
    seed: u64,
    #[allow(dead_code)]
    created_at: String,
}
