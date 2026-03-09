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
pub use service::transcribe_audio_local;
