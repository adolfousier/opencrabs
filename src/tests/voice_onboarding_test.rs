//! Voice Onboarding & Local STT Tests
//!
//! Tests for the voice setup step in the onboarding wizard:
//! STT mode selection (API vs Local), key handling, navigation,
//! config persistence, TuiEvent wiring, and local whisper presets.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::tui::onboarding::{OnboardingStep, OnboardingWizard, VoiceField, WizardAction};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::empty())
}

// ─── STT mode selection ─────────────────────────────────────────────────────

#[test]
fn voice_step_starts_on_stt_mode_select() {
    let mut wizard = OnboardingWizard::new();
    wizard.step = OnboardingStep::VoiceSetup;
    assert_eq!(wizard.voice_field, VoiceField::SttModeSelect);
    assert_eq!(wizard.stt_mode, 0); // API by default
}

#[test]
fn stt_mode_toggle_with_up_down() {
    let mut wizard = OnboardingWizard::new();
    wizard.step = OnboardingStep::VoiceSetup;
    wizard.voice_field = VoiceField::SttModeSelect;

    // Start at API (0), press Down -> Local (1)
    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::Down));
    assert_eq!(wizard.stt_mode, 1);

    // Press Down again -> back to API (0) — it toggles
    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::Down));
    assert_eq!(wizard.stt_mode, 0);

    // Press Up -> Local (1)
    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::Up));
    assert_eq!(wizard.stt_mode, 1);
}

#[test]
fn stt_mode_api_tab_goes_to_groq_key() {
    let mut wizard = OnboardingWizard::new();
    wizard.step = OnboardingStep::VoiceSetup;
    wizard.voice_field = VoiceField::SttModeSelect;
    wizard.stt_mode = 0; // API

    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::Tab));
    assert_eq!(wizard.voice_field, VoiceField::GroqApiKey);
}

#[test]
fn stt_mode_local_tab_goes_to_local_model() {
    let mut wizard = OnboardingWizard::new();
    wizard.step = OnboardingStep::VoiceSetup;
    wizard.voice_field = VoiceField::SttModeSelect;
    wizard.stt_mode = 1; // Local

    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::Tab));
    assert_eq!(wizard.voice_field, VoiceField::LocalModelSelect);
}

#[test]
fn stt_mode_enter_navigates_same_as_tab() {
    let mut wizard = OnboardingWizard::new();
    wizard.step = OnboardingStep::VoiceSetup;
    wizard.voice_field = VoiceField::SttModeSelect;
    wizard.stt_mode = 0;

    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::Enter));
    assert_eq!(wizard.voice_field, VoiceField::GroqApiKey);
}

// ─── Groq API key input ────────────────────────────────────────────────────

#[test]
fn groq_key_typing_appends_chars() {
    let mut wizard = OnboardingWizard::new();
    wizard.step = OnboardingStep::VoiceSetup;
    wizard.voice_field = VoiceField::GroqApiKey;

    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::Char('a')));
    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::Char('b')));
    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::Char('c')));
    assert_eq!(wizard.groq_api_key_input, "abc");
}

#[test]
fn groq_key_backspace_removes_char() {
    let mut wizard = OnboardingWizard::new();
    wizard.step = OnboardingStep::VoiceSetup;
    wizard.voice_field = VoiceField::GroqApiKey;
    wizard.groq_api_key_input = "hello".to_string();

    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::Backspace));
    assert_eq!(wizard.groq_api_key_input, "hell");
}

#[test]
fn groq_key_tab_goes_to_tts() {
    let mut wizard = OnboardingWizard::new();
    wizard.step = OnboardingStep::VoiceSetup;
    wizard.voice_field = VoiceField::GroqApiKey;

    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::Tab));
    assert_eq!(wizard.voice_field, VoiceField::TtsToggle);
}

#[test]
fn groq_key_backtab_goes_to_stt_mode() {
    let mut wizard = OnboardingWizard::new();
    wizard.step = OnboardingStep::VoiceSetup;
    wizard.voice_field = VoiceField::GroqApiKey;

    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::BackTab));
    assert_eq!(wizard.voice_field, VoiceField::SttModeSelect);
}

// ─── Local model selection ──────────────────────────────────────────────────

#[test]
fn local_model_tab_goes_to_tts() {
    let mut wizard = OnboardingWizard::new();
    wizard.step = OnboardingStep::VoiceSetup;
    wizard.voice_field = VoiceField::LocalModelSelect;

    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::Tab));
    assert_eq!(wizard.voice_field, VoiceField::TtsToggle);
}

#[test]
fn local_model_backtab_goes_to_stt_mode() {
    let mut wizard = OnboardingWizard::new();
    wizard.step = OnboardingStep::VoiceSetup;
    wizard.voice_field = VoiceField::LocalModelSelect;

    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::BackTab));
    assert_eq!(wizard.voice_field, VoiceField::SttModeSelect);
}

#[test]
fn local_model_enter_when_not_downloaded_returns_download_action() {
    let mut wizard = OnboardingWizard::new();
    wizard.step = OnboardingStep::VoiceSetup;
    wizard.voice_field = VoiceField::LocalModelSelect;
    wizard.stt_model_downloaded = false;
    wizard.stt_model_download_progress = None;

    let action = crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::Enter));
    assert_eq!(action, WizardAction::DownloadWhisperModel);
}

#[test]
fn local_model_enter_when_downloaded_goes_to_tts() {
    let mut wizard = OnboardingWizard::new();
    wizard.step = OnboardingStep::VoiceSetup;
    wizard.voice_field = VoiceField::LocalModelSelect;
    wizard.stt_model_downloaded = true;

    let action = crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::Enter));
    assert_eq!(action, WizardAction::None);
    assert_eq!(wizard.voice_field, VoiceField::TtsToggle);
}

#[test]
fn local_model_enter_during_download_does_nothing() {
    let mut wizard = OnboardingWizard::new();
    wizard.step = OnboardingStep::VoiceSetup;
    wizard.voice_field = VoiceField::LocalModelSelect;
    wizard.stt_model_downloaded = false;
    wizard.stt_model_download_progress = Some(0.5); // downloading

    let action = crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::Enter));
    assert_eq!(action, WizardAction::None);
    assert_eq!(wizard.voice_field, VoiceField::LocalModelSelect); // stays
}

// ─── TTS toggle ─────────────────────────────────────────────────────────────

#[test]
fn tts_toggle_space_toggles() {
    let mut wizard = OnboardingWizard::new();
    wizard.step = OnboardingStep::VoiceSetup;
    wizard.voice_field = VoiceField::TtsToggle;
    assert!(!wizard.tts_enabled);

    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::Char(' ')));
    assert!(wizard.tts_enabled);

    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::Char(' ')));
    assert!(!wizard.tts_enabled);
}

#[test]
fn tts_toggle_up_down_toggles() {
    let mut wizard = OnboardingWizard::new();
    wizard.step = OnboardingStep::VoiceSetup;
    wizard.voice_field = VoiceField::TtsToggle;

    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::Up));
    assert!(wizard.tts_enabled);

    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::Down));
    assert!(!wizard.tts_enabled);
}

#[test]
fn tts_enter_advances_to_next_step() {
    let mut wizard = OnboardingWizard::new();
    wizard.step = OnboardingStep::VoiceSetup;
    wizard.voice_field = VoiceField::TtsToggle;

    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::Enter));
    assert_eq!(wizard.step, OnboardingStep::ImageSetup);
}

#[test]
fn tts_backtab_goes_to_groq_key_in_api_mode() {
    let mut wizard = OnboardingWizard::new();
    wizard.step = OnboardingStep::VoiceSetup;
    wizard.voice_field = VoiceField::TtsToggle;
    wizard.stt_mode = 0; // API

    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::BackTab));
    assert_eq!(wizard.voice_field, VoiceField::GroqApiKey);
}

#[test]
fn tts_backtab_goes_to_local_model_in_local_mode() {
    let mut wizard = OnboardingWizard::new();
    wizard.step = OnboardingStep::VoiceSetup;
    wizard.voice_field = VoiceField::TtsToggle;
    wizard.stt_mode = 1; // Local

    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::BackTab));
    assert_eq!(wizard.voice_field, VoiceField::LocalModelSelect);
}

// ─── Full navigation flow ───────────────────────────────────────────────────

#[test]
fn full_api_flow_stt_to_tts_to_next_step() {
    let mut wizard = OnboardingWizard::new();
    wizard.step = OnboardingStep::VoiceSetup;
    wizard.voice_field = VoiceField::SttModeSelect;
    wizard.stt_mode = 0; // API mode

    // Tab → GroqApiKey
    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::Tab));
    assert_eq!(wizard.voice_field, VoiceField::GroqApiKey);

    // Type a key
    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::Char('x')));
    assert_eq!(wizard.groq_api_key_input, "x");

    // Tab → TtsToggle
    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::Tab));
    assert_eq!(wizard.voice_field, VoiceField::TtsToggle);

    // Enter → next step
    crate::tui::onboarding::voice::handle_key(&mut wizard, key(KeyCode::Enter));
    assert_eq!(wizard.step, OnboardingStep::ImageSetup);
}

#[test]
fn navigation_channels_to_voice_sets_stt_mode_select() {
    let mut wizard = OnboardingWizard::new();
    wizard.step = OnboardingStep::Channels;

    wizard.next_step();
    assert_eq!(wizard.step, OnboardingStep::VoiceSetup);
    assert_eq!(wizard.voice_field, VoiceField::SttModeSelect);
}

#[test]
fn navigation_voice_to_image() {
    let mut wizard = OnboardingWizard::new();
    wizard.step = OnboardingStep::VoiceSetup;

    wizard.next_step();
    assert_eq!(wizard.step, OnboardingStep::ImageSetup);
}

#[test]
fn navigation_image_back_to_voice() {
    let mut wizard = OnboardingWizard::new();
    wizard.step = OnboardingStep::ImageSetup;

    wizard.prev_step();
    assert_eq!(wizard.step, OnboardingStep::VoiceSetup);
    assert_eq!(wizard.voice_field, VoiceField::SttModeSelect);
}

// ─── Config persistence ────────────────────────────────────────────────────

#[test]
fn stt_mode_config_round_trip() {
    use crate::config::SttMode;

    // Default is API
    let mode = SttMode::default();
    assert_eq!(mode, SttMode::Api);

    // Serialize/deserialize
    let json = serde_json::to_string(&SttMode::Local).unwrap();
    assert_eq!(json, "\"local\"");

    let parsed: SttMode = serde_json::from_str("\"api\"").unwrap();
    assert_eq!(parsed, SttMode::Api);

    let parsed: SttMode = serde_json::from_str("\"local\"").unwrap();
    assert_eq!(parsed, SttMode::Local);
}

// ─── TuiEvent variants ────────────────────────────────────────────────────

#[test]
fn tui_event_whisper_progress_variant_exists() {
    use crate::tui::events::TuiEvent;

    let event = TuiEvent::WhisperDownloadProgress(0.5);
    match event {
        TuiEvent::WhisperDownloadProgress(p) => assert!((p - 0.5).abs() < f64::EPSILON),
        _ => panic!("wrong variant"),
    }
}

#[test]
fn tui_event_whisper_complete_ok() {
    use crate::tui::events::TuiEvent;

    let event = TuiEvent::WhisperDownloadComplete(Ok(()));
    match event {
        TuiEvent::WhisperDownloadComplete(Ok(())) => {}
        _ => panic!("wrong variant"),
    }
}

#[test]
fn tui_event_whisper_complete_err() {
    use crate::tui::events::TuiEvent;

    let event = TuiEvent::WhisperDownloadComplete(Err("network error".to_string()));
    match event {
        TuiEvent::WhisperDownloadComplete(Err(msg)) => {
            assert_eq!(msg, "network error");
        }
        _ => panic!("wrong variant"),
    }
}

// ─── Local whisper presets ──────────────────────────────────────────────────

#[cfg(feature = "local-stt")]
mod local_stt_tests {
    use crate::channels::voice::local_whisper::*;

    #[test]
    fn preset_count() {
        assert_eq!(LOCAL_MODEL_PRESETS.len(), 4);
    }

    #[test]
    fn preset_ids_unique() {
        let ids: Vec<&str> = LOCAL_MODEL_PRESETS.iter().map(|p| p.id).collect();
        let mut deduped = ids.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(ids.len(), deduped.len());
    }

    #[test]
    fn find_local_model_by_id() {
        let tiny = find_local_model("local-tiny");
        assert!(tiny.is_some());
        assert_eq!(tiny.unwrap().label, "Tiny");

        let medium = find_local_model("local-medium");
        assert!(medium.is_some());
        assert_eq!(medium.unwrap().label, "Medium");

        assert!(find_local_model("nonexistent").is_none());
    }

    #[test]
    fn model_path_contains_file_name() {
        let preset = &LOCAL_MODEL_PRESETS[0];
        let path = model_path(preset);
        assert!(path.ends_with(preset.file_name));
    }

    #[test]
    fn model_url_contains_huggingface() {
        let url = model_url("ggml-tiny.en.bin");
        assert!(url.contains("huggingface.co"));
        assert!(url.contains("ggml-tiny.en.bin"));
    }

    #[test]
    fn models_dir_is_inside_opencrabs() {
        let dir = models_dir();
        let dir_str = dir.to_string_lossy();
        assert!(dir_str.contains("opencrabs"));
        assert!(dir_str.contains("whisper"));
    }

    #[test]
    fn download_progress_struct_fields() {
        let progress = DownloadProgress {
            downloaded: 1024,
            total: Some(2048),
            done: false,
            error: None,
        };
        assert_eq!(progress.downloaded, 1024);
        assert_eq!(progress.total, Some(2048));
        assert!(!progress.done);
        assert!(progress.error.is_none());
    }
}

// ─── Wizard action enum ────────────────────────────────────────────────────

#[test]
fn wizard_action_download_whisper_model_variant() {
    let action = WizardAction::DownloadWhisperModel;
    assert_eq!(action, WizardAction::DownloadWhisperModel);
    assert_ne!(action, WizardAction::None);
}

// ─── Rendering smoke test ───────────────────────────────────────────────────

#[test]
fn voice_render_produces_lines() {
    use ratatui::text::Line;

    let wizard = OnboardingWizard::new();
    let mut lines: Vec<Line<'static>> = Vec::new();
    crate::tui::onboarding::voice::render(&mut lines, &wizard);
    assert!(!lines.is_empty(), "voice render should produce lines");
}

#[test]
fn voice_render_api_mode_shows_groq_field() {
    use ratatui::text::Line;

    let mut wizard = OnboardingWizard::new();
    wizard.stt_mode = 0; // API
    wizard.voice_field = VoiceField::GroqApiKey;
    let mut lines: Vec<Line<'static>> = Vec::new();
    crate::tui::onboarding::voice::render(&mut lines, &wizard);

    let text: String = lines
        .iter()
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("Groq Key"),
        "API mode should show Groq Key field"
    );
}

#[test]
fn voice_render_local_mode_shows_model_select() {
    use ratatui::text::Line;

    let mut wizard = OnboardingWizard::new();
    wizard.stt_mode = 1; // Local
    wizard.voice_field = VoiceField::LocalModelSelect;
    let mut lines: Vec<Line<'static>> = Vec::new();
    crate::tui::onboarding::voice::render(&mut lines, &wizard);

    let text: String = lines
        .iter()
        .map(|l| l.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("Select model size") || text.contains("local-stt"),
        "Local mode should show model selector or feature note"
    );
}
