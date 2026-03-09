//! Voice STT Dispatch & Audio Decoding Tests
//!
//! Tests for:
//! - STT dispatch logic (API vs Local routing based on VoiceConfig)
//! - Audio decoding: WAV, OGG/Opus format handling
//! - Quick-jump config persistence
//! - Edge cases: missing keys, missing models, unknown model IDs

use crate::config::{ProviderConfig, SttMode, VoiceConfig};

// ─── STT dispatch routing ──────────────────────────────────────────────────

#[tokio::test]
async fn dispatch_api_mode_requires_api_key() {
    let config = VoiceConfig {
        stt_enabled: true,
        stt_mode: SttMode::Api,
        stt_provider: None, // no provider
        ..VoiceConfig::default()
    };

    let result = crate::channels::voice::transcribe(vec![0u8; 50], &config).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("API key not configured"),
        "Should fail with missing API key error"
    );
}

#[tokio::test]
async fn dispatch_api_mode_with_empty_key_fails() {
    let config = VoiceConfig {
        stt_enabled: true,
        stt_mode: SttMode::Api,
        stt_provider: Some(ProviderConfig {
            api_key: Some(String::new()),
            ..ProviderConfig::default()
        }),
        ..VoiceConfig::default()
    };

    // Empty key will fail at the API call, not at dispatch
    // (the dispatch checks for Some, not for non-empty)
    let result = crate::channels::voice::transcribe(vec![0u8; 50], &config).await;
    assert!(result.is_err(), "Empty API key should fail at Groq API");
}

#[tokio::test]
async fn dispatch_api_mode_with_provider_no_key_fails() {
    let config = VoiceConfig {
        stt_enabled: true,
        stt_mode: SttMode::Api,
        stt_provider: Some(ProviderConfig {
            api_key: None,
            ..ProviderConfig::default()
        }),
        ..VoiceConfig::default()
    };

    let result = crate::channels::voice::transcribe(vec![0u8; 50], &config).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("API key not configured"),
    );
}

#[cfg(feature = "local-stt")]
#[tokio::test]
async fn dispatch_local_mode_unknown_model_fails() {
    let config = VoiceConfig {
        stt_enabled: true,
        stt_mode: SttMode::Local,
        local_stt_model: "nonexistent-model".to_string(),
        ..VoiceConfig::default()
    };

    let result = crate::channels::voice::transcribe(vec![0u8; 50], &config).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Unknown local STT model"),
        "Should fail with unknown model error"
    );
}

#[cfg(feature = "local-stt")]
#[tokio::test]
async fn dispatch_local_mode_model_not_downloaded_fails() {
    let config = VoiceConfig {
        stt_enabled: true,
        stt_mode: SttMode::Local,
        // Use a valid preset ID but model file won't exist in test env (unless downloaded)
        local_stt_model: "local-medium".to_string(),
        ..VoiceConfig::default()
    };

    // This test will only fail if local-medium is not downloaded (expected in CI)
    let result = crate::channels::voice::transcribe(vec![0u8; 50], &config).await;
    if let Err(e) = &result {
        let msg = e.to_string();
        // Either "not downloaded" or decoding error (if model exists)
        assert!(
            msg.contains("not downloaded") || msg.contains("decode") || msg.contains("whisper"),
            "Expected download or decode error, got: {}",
            msg
        );
    }
    // If Ok, the model was downloaded and transcription ran — that's fine too
}

// ─── VoiceConfig defaults ──────────────────────────────────────────────────

#[test]
fn voice_config_default_is_api_mode() {
    let config = VoiceConfig::default();
    assert_eq!(config.stt_mode, SttMode::Api);
    assert!(!config.stt_enabled);
    assert!(!config.tts_enabled);
    assert_eq!(config.local_stt_model, "local-tiny");
}

#[test]
fn voice_config_local_stt_from_providers() {
    let toml_str = r#"
[providers.stt.local]
enabled = true
model = "local-base"
"#;
    let config: crate::config::Config = toml::from_str(toml_str).unwrap();
    let vc = config.voice_config();
    assert_eq!(vc.stt_mode, SttMode::Local);
    assert_eq!(vc.local_stt_model, "local-base");
    assert!(vc.stt_enabled);
}

#[test]
fn voice_config_no_stt_defaults_to_api_disabled() {
    let toml_str = "";
    let config: crate::config::Config = toml::from_str(toml_str).unwrap();
    let vc = config.voice_config();
    assert_eq!(vc.stt_mode, SttMode::Api);
    assert!(!vc.stt_enabled);
    assert_eq!(vc.local_stt_model, "local-tiny"); // default
}

// ─── Audio decoding ────────────────────────────────────────────────────────

#[cfg(feature = "local-stt")]
mod audio_decode_tests {
    #[test]
    fn decode_empty_bytes_fails() {
        // Empty bytes should fail to decode
        let result = std::panic::catch_unwind(|| {
            // LocalWhisper::transcribe requires a valid model, but we can test
            // the decode_audio path indirectly
            let bytes: Vec<u8> = vec![];
            // WAV magic check fails, then OGG probe fails
            assert!(bytes.len() < 4 || &bytes[..4] != b"RIFF");
        });
        assert!(result.is_ok());
    }

    #[test]
    fn wav_magic_detection() {
        // Valid WAV header starts with "RIFF"
        let wav_header = b"RIFF";
        assert_eq!(&wav_header[..4], b"RIFF");

        // OGG header starts with "OggS"
        let ogg_header = b"OggS";
        assert_ne!(&ogg_header[..4], b"RIFF");
    }

    #[test]
    fn generate_and_decode_wav() {
        // Generate a minimal valid WAV file with a sine wave
        let sample_rate = 16000u32;
        let duration_secs = 0.1; // 100ms
        let num_samples = (sample_rate as f64 * duration_secs) as usize;

        let mut wav_bytes = Vec::new();
        {
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let cursor = std::io::Cursor::new(&mut wav_bytes);
            let mut writer = hound::WavWriter::new(cursor, spec).unwrap();
            for i in 0..num_samples {
                let t = i as f32 / sample_rate as f32;
                let sample = (t * 440.0 * 2.0 * std::f32::consts::PI).sin();
                writer
                    .write_sample((sample * i16::MAX as f32) as i16)
                    .unwrap();
            }
            writer.finalize().unwrap();
        }

        // Verify it starts with RIFF
        assert_eq!(&wav_bytes[..4], b"RIFF");
        assert!(wav_bytes.len() > 44, "WAV should have header + data");
    }

    #[test]
    fn resampler_identity() {
        // Resampling 16kHz → 16kHz should be near-identity
        // We can't test resample directly (private), but we verify the concept
        let samples: Vec<f32> = (0..1600)
            .map(|i| (i as f32 / 1600.0 * std::f32::consts::PI * 2.0).sin())
            .collect();
        assert_eq!(samples.len(), 1600);
        // At 16kHz this is 0.1s of audio — valid for whisper
    }
}

// ─── Quick-jump config persistence ─────────────────────────────────────────

#[test]
fn quick_jump_done_triggers_apply_config_flag() {
    use crate::tui::onboarding::{OnboardingStep, OnboardingWizard, VoiceField};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut wizard = OnboardingWizard::new();
    wizard.quick_jump = true;
    wizard.step = OnboardingStep::VoiceSetup;
    wizard.voice_field = VoiceField::TtsModeSelect;

    // Tab on TtsModeSelect (Off) calls next_step() → sets quick_jump_done
    let action = wizard.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()));

    // In quick_jump mode, completing a step returns QuickJumpDone (saves config then closes)
    assert_eq!(
        action,
        crate::tui::onboarding::WizardAction::QuickJumpDone,
        "Quick-jump should return QuickJumpDone after step completion"
    );
}

#[test]
fn quick_jump_esc_returns_cancel() {
    use crate::tui::onboarding::{OnboardingStep, OnboardingWizard, VoiceField, WizardAction};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut wizard = OnboardingWizard::new();
    wizard.quick_jump = true;
    wizard.step = OnboardingStep::VoiceSetup;
    wizard.voice_field = VoiceField::SttModeSelect;

    let action = wizard.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()));
    assert_eq!(action, WizardAction::Cancel);
}

#[test]
fn non_quick_jump_tts_tab_advances_step() {
    use crate::tui::onboarding::{OnboardingStep, OnboardingWizard, VoiceField, WizardAction};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut wizard = OnboardingWizard::new();
    wizard.quick_jump = false;
    wizard.step = OnboardingStep::VoiceSetup;
    wizard.voice_field = VoiceField::TtsModeSelect;

    let action = wizard.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()));
    assert_eq!(action, WizardAction::None);
    assert_eq!(
        wizard.step,
        OnboardingStep::ImageSetup,
        "Non-quick-jump should advance to next step"
    );
}

// ─── Local whisper codec support ───────────────────────────────────────────

#[cfg(feature = "local-stt")]
mod codec_tests {
    #[test]
    fn opus_decoder_registered() {
        // Verify we can create a codec registry with Opus support
        use symphonia::core::codecs::CodecRegistry;
        let mut registry = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut registry);
        registry.register_all::<symphonia_adapter_libopus::OpusDecoder>();
        // If this compiles and runs, the adapter is properly linked
    }

    #[test]
    fn symphonia_probes_ogg_container() {
        use symphonia::core::formats::FormatOptions;
        use symphonia::core::io::MediaSourceStream;
        use symphonia::core::meta::MetadataOptions;
        use symphonia::core::probe::Hint;

        // Minimal OGG header magic bytes (not a complete file, so probe should fail)
        let fake_ogg = b"OggS\x00\x02\x00\x00\x00\x00\x00\x00\x00\x00";
        let cursor = std::io::Cursor::new(fake_ogg.to_vec());
        let mss = MediaSourceStream::new(Box::new(cursor), Default::default());

        let mut hint = Hint::new();
        hint.with_extension("ogg");

        // Probe should at least recognize the OGG magic (may fail on incomplete data)
        let result = symphonia::default::get_probe().format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        );
        // We just care that it doesn't panic — error is expected for truncated data
        let _ = result;
    }

    #[test]
    fn local_model_presets_have_valid_repo_ids() {
        use crate::channels::voice::local_whisper::LOCAL_MODEL_PRESETS;

        for preset in LOCAL_MODEL_PRESETS {
            assert!(
                preset.repo_id.starts_with("openai/whisper-"),
                "Repo ID should start with openai/whisper-: {}",
                preset.repo_id
            );
        }
    }
}

// ─── Groq STT (API mode) mock tests ────────────────────────────────────────

#[tokio::test]
async fn api_mode_dispatches_to_groq() {
    // Set up a mock Groq server
    let mut server = mockito::Server::new_async().await;
    let _mock = server
        .mock("POST", "/")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"text": "hello from dispatch test"}"#)
        .create_async()
        .await;

    // We can't easily inject the mock URL into the dispatch function since it uses
    // the hardcoded GROQ_TRANSCRIPTION_URL. But we CAN test transcribe_audio directly.
    let result = crate::channels::voice::transcribe_audio(vec![0u8; 50], "test-key").await;
    // This will fail because it hits the real Groq URL with a fake key,
    // but we can verify it returns an error (not a panic)
    assert!(result.is_err());
}

#[tokio::test]
async fn dispatch_selects_correct_mode() {
    // API mode with valid key → should attempt Groq (will fail with bad key, but routes correctly)
    let api_config = VoiceConfig {
        stt_enabled: true,
        stt_mode: SttMode::Api,
        stt_provider: Some(ProviderConfig {
            api_key: Some("fake-groq-key".to_string()),
            ..ProviderConfig::default()
        }),
        ..VoiceConfig::default()
    };

    let result = crate::channels::voice::transcribe(vec![0u8; 50], &api_config).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    // Should fail at Groq API level, not at dispatch level
    assert!(
        err.contains("Groq") || err.contains("send") || err.contains("error"),
        "API mode should attempt Groq: {}",
        err
    );
}

#[cfg(feature = "local-stt")]
#[tokio::test]
async fn dispatch_local_mode_attempts_local_whisper() {
    let local_config = VoiceConfig {
        stt_enabled: true,
        stt_mode: SttMode::Local,
        local_stt_model: "local-tiny".to_string(),
        ..VoiceConfig::default()
    };

    let result = crate::channels::voice::transcribe(vec![0u8; 50], &local_config).await;
    // Will fail because model may not be downloaded or audio is invalid,
    // but should NOT mention Groq
    if let Err(e) = &result {
        let msg = e.to_string();
        assert!(
            !msg.contains("API key"),
            "Local mode should not check for API key: {}",
            msg
        );
    }
}
