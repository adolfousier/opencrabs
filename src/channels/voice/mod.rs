//! Voice Processing Module
//!
//! Speech-to-text and text-to-speech services.
//! Supports API-based STT (Groq Whisper) and local STT (whisper.cpp).

#[cfg(feature = "local-stt")]
pub mod local_whisper;

mod service;

pub use service::{synthesize_speech, transcribe_audio};

#[cfg(feature = "local-stt")]
pub use service::transcribe_audio_local;
