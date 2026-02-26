//! Voice Processing Module
//!
//! Speech-to-text (Groq Whisper) and text-to-speech (OpenAI TTS) services
//! used by the Telegram bot for voice note support.

use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;

const GROQ_TRANSCRIPTION_URL: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
const OPENAI_SPEECH_URL: &str = "https://api.openai.com/v1/audio/speech";

/// Transcribe audio bytes using Groq Whisper (whisper-large-v3-turbo).
///
/// Accepts OGG/Opus audio (Telegram voice note format).
/// Returns the transcribed text.
pub async fn transcribe_audio(audio_bytes: Vec<u8>, groq_api_key: &str) -> Result<String> {
    transcribe_audio_with_url(audio_bytes, groq_api_key, GROQ_TRANSCRIPTION_URL).await
}

/// Internal: transcribe with configurable URL (for testing).
async fn transcribe_audio_with_url(
    audio_bytes: Vec<u8>,
    api_key: &str,
    url: &str,
) -> Result<String> {
    let client = Client::new();

    let file_part = reqwest::multipart::Part::bytes(audio_bytes)
        .file_name("voice.ogg")
        .mime_str("audio/ogg")?;

    let form = reqwest::multipart::Form::new()
        .part("file", file_part)
        .text("model", "whisper-large-v3-turbo")
        .text("response_format", "json");

    let response = client
        .post(url)
        .header("Authorization", format!("Bearer {}", api_key))
        .multipart(form)
        .send()
        .await
        .context("Failed to send audio to Groq Whisper")?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response.text().await.unwrap_or_default();
        anyhow::bail!("Groq STT error ({}): {}", status, error_text);
    }

    let result: TranscriptionResponse = response
        .json()
        .await
        .context("Failed to parse Groq transcription response")?;

    tracing::info!("Groq STT: transcribed {} chars", result.text.len());

    Ok(result.text)
}

/// Synthesize speech from text using OpenAI TTS.
///
/// Returns OGG/Opus audio bytes suitable for Telegram voice notes.
pub async fn synthesize_speech(
    text: &str,
    openai_api_key: &str,
    voice: &str,
    model: &str,
) -> Result<Vec<u8>> {
    synthesize_speech_with_url(text, openai_api_key, voice, model, OPENAI_SPEECH_URL).await
}

/// Internal: synthesize with configurable URL (for testing).
async fn synthesize_speech_with_url(
    text: &str,
    api_key: &str,
    voice: &str,
    model: &str,
    url: &str,
) -> Result<Vec<u8>> {
    let client = Client::new();

    let body = serde_json::json!({
        "model": model,
        "input": text,
        "voice": voice,
        "response_format": "opus",
    });

    let response = client
        .post(url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .context("Failed to send TTS request to OpenAI")?;

    let status = response.status();
    if !status.is_success() {
        let error_text = response.text().await.unwrap_or_default();
        anyhow::bail!("OpenAI TTS error ({}): {}", status, error_text);
    }

    let audio_bytes = response
        .bytes()
        .await
        .context("Failed to read TTS audio bytes")?
        .to_vec();

    tracing::info!(
        "OpenAI TTS: generated {} bytes of audio (voice={}, model={})",
        audio_bytes.len(),
        voice,
        model,
    );

    Ok(audio_bytes)
}

#[derive(Deserialize)]
struct TranscriptionResponse {
    text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transcription_response_parse() {
        let json = r#"{"text": "Hello, this is a test."}"#;
        let result: TranscriptionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(result.text, "Hello, this is a test.");
    }

    #[test]
    fn test_transcription_response_parse_unicode() {
        let json = r#"{"text": "Olá, como você está?"}"#;
        let result: TranscriptionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(result.text, "Olá, como você está?");
    }

    #[test]
    fn test_transcription_response_parse_empty() {
        let json = r#"{"text": ""}"#;
        let result: TranscriptionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(result.text, "");
    }

    // --- STT tests with mock HTTP server ---

    #[tokio::test]
    async fn test_stt_success() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .match_header("Authorization", "Bearer test-groq-key")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"text": "Hello from voice note"}"#)
            .create_async()
            .await;

        let audio = vec![0u8; 100]; // fake audio bytes
        let result = transcribe_audio_with_url(audio, "test-groq-key", &server.url()).await;

        mock.assert_async().await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Hello from voice note");
    }

    #[tokio::test]
    async fn test_stt_api_error_returns_error() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .with_status(401)
            .with_body(r#"{"error": "Invalid API key"}"#)
            .create_async()
            .await;

        let audio = vec![0u8; 50];
        let result = transcribe_audio_with_url(audio, "bad-key", &server.url()).await;

        mock.assert_async().await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("401"),
            "error should mention status code: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_stt_server_error_500() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .with_status(500)
            .with_body("Internal Server Error")
            .create_async()
            .await;

        let audio = vec![0u8; 50];
        let result = transcribe_audio_with_url(audio, "key", &server.url()).await;

        mock.assert_async().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("500"));
    }

    #[tokio::test]
    async fn test_stt_malformed_json_response() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("not json at all")
            .create_async()
            .await;

        let audio = vec![0u8; 50];
        let result = transcribe_audio_with_url(audio, "key", &server.url()).await;

        mock.assert_async().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("parse"));
    }

    #[tokio::test]
    async fn test_stt_long_transcription() {
        let long_text = "word ".repeat(500);
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(format!(r#"{{"text": "{}"}}"#, long_text.trim()))
            .create_async()
            .await;

        let audio = vec![0u8; 100];
        let result = transcribe_audio_with_url(audio, "key", &server.url()).await;

        mock.assert_async().await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), long_text.trim());
    }

    // --- TTS tests with mock HTTP server ---

    #[tokio::test]
    async fn test_tts_success() {
        let fake_audio = vec![0xFFu8; 256]; // fake opus bytes
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .match_header("Authorization", "Bearer test-openai-key")
            .with_status(200)
            .with_header("content-type", "audio/opus")
            .with_body(fake_audio.clone())
            .create_async()
            .await;

        let result = synthesize_speech_with_url(
            "Hello world",
            "test-openai-key",
            "ash",
            "gpt-4o-mini-tts",
            &server.url(),
        )
        .await;

        mock.assert_async().await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), fake_audio);
    }

    #[tokio::test]
    async fn test_tts_sends_correct_json_body() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .match_header("content-type", "application/json")
            .match_body(mockito::Matcher::PartialJsonString(
                r#"{"model":"gpt-4o-mini-tts","voice":"ash","response_format":"opus"}"#.to_string(),
            ))
            .with_status(200)
            .with_body(vec![0u8; 10])
            .create_async()
            .await;

        let _ = synthesize_speech_with_url(
            "Test input",
            "key",
            "ash",
            "gpt-4o-mini-tts",
            &server.url(),
        )
        .await;

        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_tts_api_error_returns_error() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .with_status(429)
            .with_body(r#"{"error": "Rate limit exceeded"}"#)
            .create_async()
            .await;

        let result =
            synthesize_speech_with_url("Hello", "key", "ash", "gpt-4o-mini-tts", &server.url())
                .await;

        mock.assert_async().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("429"));
    }

    #[tokio::test]
    async fn test_tts_server_error_500() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .with_status(500)
            .with_body("Internal Server Error")
            .create_async()
            .await;

        let result =
            synthesize_speech_with_url("Hello", "key", "ash", "tts-1", &server.url()).await;

        mock.assert_async().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_tts_empty_audio_response() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/")
            .with_status(200)
            .with_body(Vec::<u8>::new())
            .create_async()
            .await;

        let result =
            synthesize_speech_with_url("Hello", "key", "ash", "tts-1", &server.url()).await;

        mock.assert_async().await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_tts_different_voices() {
        for voice in &["ash", "alloy", "nova", "shimmer"] {
            let mut server = mockito::Server::new_async().await;
            let mock = server
                .mock("POST", "/")
                .match_body(mockito::Matcher::PartialJsonString(format!(
                    r#"{{"voice":"{}"}}"#,
                    voice
                )))
                .with_status(200)
                .with_body(vec![1u8; 10])
                .create_async()
                .await;

            let result =
                synthesize_speech_with_url("Test", "key", voice, "tts-1", &server.url()).await;

            mock.assert_async().await;
            assert!(result.is_ok(), "voice '{}' should work", voice);
        }
    }
}
