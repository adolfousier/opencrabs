//! Voicebox TTS provider.
//!
//! POST to `/generate` endpoint, poll `/generate/{id}/status` until completed,
//! then read the audio file from the returned path.

use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use std::path::Path;
use tokio::time::{Duration, sleep};

use super::openai_tts::build_endpoint_url;

/// Maximum time to wait for Voicebox generation to complete.
const POLL_TIMEOUT: Duration = Duration::from_secs(120);
/// Interval between status polls.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Voicebox TTS client.
pub struct VoiceboxTts {
    client: Client,
    base_url: String,
    profile_id: String,
}

#[derive(Deserialize)]
struct GenerateResponse {
    id: String,
    status: String,
    audio_path: String,
    #[allow(dead_code)]
    duration: f64,
    #[allow(dead_code)]
    error: Option<String>,
}

#[derive(Deserialize)]
struct StatusResponse {
    id: String,
    status: String,
    audio_path: String,
    duration: f64,
    error: Option<String>,
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
    /// Flow: POST /generate → poll /generate/{id}/status → read audio file.
    pub async fn synthesize(&self, text: &str) -> Result<Vec<u8>> {
        if text.is_empty() {
            anyhow::bail!("Cannot synthesize empty text");
        }

        let generate_url = build_endpoint_url(&self.base_url, "generate")?;

        let body = serde_json::json!({
            "profile_id": self.profile_id,
            "text": text,
        });

        let response = self
            .client
            .post(&generate_url)
            .header("Content-Type", "application/json")
            .json(&body)
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

        let generation_id = result.id;

        // If already completed (sync mode), use the result directly
        if result.status == "completed" && !result.audio_path.is_empty() {
            return self.read_audio_file(&result.audio_path, result.duration).await;
        }

        // Async mode: poll until completed
        let final_result = self.poll_until_completed(&generation_id).await?;
        self.read_audio_file(&final_result.audio_path, final_result.duration)
            .await
    }

    /// Poll `/generate/{id}/status` until the generation completes.
    async fn poll_until_completed(&self, generation_id: &str) -> Result<StatusResponse> {
        let status_url = build_endpoint_url(
            &self.base_url,
            &format!("generate/{}/status", generation_id),
        )?;

        let deadline = tokio::time:: Instant::now() + POLL_TIMEOUT;

        loop {
            if tokio::time::Instant::now() > deadline {
                anyhow::bail!(
                    "Voicebox TTS timed out after {}s waiting for generation {}",
                    POLL_TIMEOUT.as_secs(),
                    generation_id
                );
            }

            sleep(POLL_INTERVAL).await;

            let response = self
                .client
                .get(&status_url)
                .send()
                .await
                .context("Failed to poll Voicebox generation status")?;

            let result: StatusResponse = response
                .json()
                .await
                .context("Failed to parse Voicebox status response")?;

            match result.status.as_str() {
                "completed" => return Ok(result),
                "failed" | "error" => {
                    anyhow::bail!(
                        "Voicebox TTS generation failed: {}",
                        result.error.unwrap_or_else(|| "Unknown error".to_string())
                    );
                }
                "generating" | "queued" | "pending" => continue,
                other => {
                    tracing::warn!("Voicebox TTS: unexpected status '{}', continuing to poll", other);
                    continue;
                }
            }
        }
    }

    /// Read audio bytes from the file path returned by Voicebox.
    async fn read_audio_file(&self, audio_path: &str, duration: f64) -> Result<Vec<u8>> {
        let path = Path::new(audio_path);
        if !path.exists() {
            anyhow::bail!("Voicebox audio file not found: {}", audio_path);
        }

        let audio_bytes = tokio::fs::read(path)
            .await
            .with_context(|| format!("Failed to read Voicebox audio file: {}", audio_path))?;

        tracing::info!(
            "Voicebox TTS: generated {} bytes of audio (profile={}, duration={:.2}s, path={})",
            audio_bytes.len(),
            self.profile_id,
            duration,
            audio_path,
        );

        Ok(audio_bytes)
    }
}
