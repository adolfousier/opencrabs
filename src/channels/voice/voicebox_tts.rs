//! Voicebox TTS provider.
//!
//! POST to `/generate` endpoint, poll `/generate/{id}/status` until completed,
//! then fetch the audio file via HTTP GET.

use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
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
    #[serde(default)]
    id: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    audio_path: String,
    #[serde(default)]
    #[allow(dead_code)]
    duration: f64,
    #[serde(default)]
    #[allow(dead_code)]
    error: Option<String>,
}

#[derive(Deserialize)]
struct StatusResponse {
    #[serde(default)]
    status: String,
    #[serde(default)]
    audio_path: String,
    #[serde(default)]
    duration: f64,
    #[serde(default)]
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
    /// Flow: POST /generate → poll /generate/{id}/status → fetch audio via HTTP.
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
            return self.fetch_audio(&result.audio_path, result.duration).await;
        }

        // Async mode: poll until completed
        let final_result = self.poll_until_completed(&generation_id).await?;
        self.fetch_audio(&final_result.audio_path, final_result.duration)
            .await
    }

    /// Poll `/generate/{id}/status` until the generation completes.
    async fn poll_until_completed(&self, generation_id: &str) -> Result<StatusResponse> {
        let status_url = build_endpoint_url(
            &self.base_url,
            &format!("generate/{}/status", generation_id),
        )?;

        let deadline = tokio::time::Instant::now() + POLL_TIMEOUT;

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

            if response.status() == reqwest::StatusCode::NO_CONTENT {
                // Still generating, not ready yet
                continue;
            }

            if !response.status().is_success() {
                // Not ready yet or not found — keep polling
                continue;
            }

            let text = response.text().await.unwrap_or_default();

            if text.is_empty() || text.trim().is_empty() {
                // Empty body = not ready yet
                continue;
            }

            // SSE responses have "data: " prefix — strip it before JSON parsing
            let json_text = text
                .lines()
                .find(|l| l.starts_with("data: "))
                .map(|l| l.trim_start_matches("data: ").trim())
                .unwrap_or(text.trim());

            let result: StatusResponse = serde_json::from_str(json_text).with_context(|| {
                format!(
                    "Failed to parse Voicebox status response: '{}'",
                    json_text.chars().take(200).collect::<String>()
                )
            })?;

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
                    tracing::warn!(
                        "Voicebox TTS: unexpected status '{}', continuing to poll",
                        other
                    );
                    continue;
                }
            }
        }
    }

    /// Fetch audio bytes from Voicebox via HTTP GET.
    async fn fetch_audio(&self, audio_path: &str, _duration: f64) -> Result<Vec<u8>> {
        let audio_url = if audio_path.starts_with("http://") || audio_path.starts_with("https://") {
            audio_path.to_string()
        } else {
            build_endpoint_url(&self.base_url, audio_path.trim_start_matches('/'))?
        };

        let resp = self
            .client
            .get(&audio_url)
            .send()
            .await
            .with_context(|| format!("Failed to fetch audio from {}", audio_url))?;

        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap_or("unknown"))
            .unwrap_or("unknown")
            .to_string();

        let resp = resp
            .error_for_status()
            .with_context(|| format!("Voicebox audio fetch returned error for {}", audio_url))?;

        let audio_bytes = resp
            .bytes()
            .await
            .with_context(|| "Failed to read audio bytes from response")?
            .to_vec();

        // Detect format from magic bytes
        let detected_format = if audio_bytes.starts_with(b"RIFF") {
            "WAV"
        } else if audio_bytes.starts_with(b"ID3") || audio_bytes.starts_with(b"\xff\xfb") {
            "MP3"
        } else if audio_bytes.starts_with(b"OggS") {
            "OGG"
        } else if audio_bytes.starts_with(&[0x00, 0x00, 0x00, 0x1c, 0x66, 0x74, 0x79, 0x70]) {
            "M4A"
        } else {
            "unknown"
        };

        tracing::info!(
            "Voicebox TTS: fetched {} bytes (content_type={}, detected={}, profile={}, path={})",
            audio_bytes.len(),
            content_type,
            detected_format,
            self.profile_id,
            audio_path,
        );

        Ok(audio_bytes)
    }
}
