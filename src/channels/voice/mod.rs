//! Voice Processing Module
//!
//! Speech-to-text and text-to-speech services.
//! Supports API-based STT (Groq Whisper) and local STT (whisper.cpp).
//! Supports API-based TTS (OpenAI) and local TTS (Piper).

#[cfg(feature = "local-stt")]
pub mod local_whisper;

#[cfg(feature = "local-tts")]
pub mod local_tts;

mod service;

pub use service::{synthesize, synthesize_speech, transcribe, transcribe_audio};

#[cfg(feature = "local-stt")]
pub use service::{preload_local_whisper, transcribe_audio_local};

/// Returns true if local STT is compiled in and can run on this machine.
pub fn local_stt_available() -> bool {
    cfg!(feature = "local-stt")
}

/// Returns true if local TTS (Piper) can run on this machine.
/// Requires `python3` to be available on the system PATH.
/// Result is cached so the probe runs at most once per process.
pub fn local_tts_available() -> bool {
    #[cfg(feature = "local-tts")]
    {
        static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *AVAILABLE.get_or_init(|| {
            std::process::Command::new("python3")
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        })
    }
    #[cfg(not(feature = "local-tts"))]
    {
        false
    }
}
