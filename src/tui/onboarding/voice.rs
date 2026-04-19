//! Voice setup step — STT mode selection, API key input, local model picker,
//! TTS mode selection (API vs Local Piper), voice picker, download,
//! plus OpenAI-compatible and Voicebox provider configuration.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::types::{VoiceField, WizardAction};
use super::wizard::OnboardingWizard;

// Brand colors (match onboarding_render.rs)
const BRAND_BLUE: Color = Color::Rgb(60, 130, 246);
const BRAND_GOLD: Color = Color::Rgb(215, 100, 20);
const ACCENT_GOLD: Color = Color::Rgb(215, 100, 20);

// ─── Key handling ───────────────────────────────────────────────────────────

pub fn handle_key(wizard: &mut OnboardingWizard, event: KeyEvent) -> WizardAction {
    match wizard.voice_field {
        VoiceField::SttModeSelect => handle_stt_mode(wizard, event.code),
        VoiceField::GroqApiKey => handle_groq_key(wizard, event.code),
        VoiceField::LocalModelSelect => handle_local_model(wizard, event.code),
        VoiceField::TtsModeSelect => handle_tts_mode(wizard, event.code),
        VoiceField::TtsLocalVoiceSelect => handle_tts_voice(wizard, event.code),
        VoiceField::Continue => handle_continue(wizard, event.code),
        // OpenAI-compatible STT
        VoiceField::SttOpenaiCompatToggle => handle_stt_openai_compat_toggle(wizard, event.code),
        VoiceField::SttOpenaiCompatUrl => handle_text_field(wizard, event.code, |w| &mut w.stt_openai_compat_base_url, VoiceField::SttOpenaiCompatModel, VoiceField::SttOpenaiCompatToggle),
        VoiceField::SttOpenaiCompatModel => handle_text_field(wizard, event.code, |w| &mut w.stt_openai_compat_model, VoiceField::SttOpenaiCompatKey, VoiceField::SttOpenaiCompatUrl),
        VoiceField::SttOpenaiCompatKey => handle_text_field(wizard, event.code, |w| &mut w.stt_openai_compat_key_input, VoiceField::SttVoiceboxToggle, VoiceField::SttOpenaiCompatModel),
        // Voicebox STT
        VoiceField::SttVoiceboxToggle => handle_stt_voicebox_toggle(wizard, event.code),
        VoiceField::SttVoiceboxUrl => handle_text_field(wizard, event.code, |w| &mut w.stt_voicebox_base_url, VoiceField::TtsOpenaiCompatToggle, VoiceField::SttVoiceboxToggle),
        // OpenAI-compatible TTS
        VoiceField::TtsOpenaiCompatToggle => handle_tts_openai_compat_toggle(wizard, event.code),
        VoiceField::TtsOpenaiCompatUrl => handle_text_field(wizard, event.code, |w| &mut w.tts_openai_compat_base_url, VoiceField::TtsOpenaiCompatModel, VoiceField::TtsOpenaiCompatToggle),
        VoiceField::TtsOpenaiCompatModel => handle_text_field(wizard, event.code, |w| &mut w.tts_openai_compat_model, VoiceField::TtsOpenaiCompatVoice, VoiceField::TtsOpenaiCompatUrl),
        VoiceField::TtsOpenaiCompatVoice => handle_text_field(wizard, event.code, |w| &mut w.tts_openai_compat_voice, VoiceField::TtsOpenaiCompatKey, VoiceField::TtsOpenaiCompatModel),
        VoiceField::TtsOpenaiCompatKey => handle_text_field(wizard, event.code, |w| &mut w.tts_openai_compat_key_input, VoiceField::TtsVoiceboxToggle, VoiceField::TtsOpenaiCompatVoice),
        // Voicebox TTS
        VoiceField::TtsVoiceboxToggle => handle_tts_voicebox_toggle(wizard, event.code),
        VoiceField::TtsVoiceboxUrl => handle_text_field(wizard, event.code, |w| &mut w.tts_voicebox_base_url, VoiceField::TtsVoiceboxProfileId, VoiceField::TtsVoiceboxToggle),
        VoiceField::TtsVoiceboxProfileId => handle_text_field(wizard, event.code, |w| &mut w.tts_voicebox_profile_id, VoiceField::Continue, VoiceField::TtsVoiceboxUrl),
    }
}

fn handle_stt_mode(wizard: &mut OnboardingWizard, key: KeyCode) -> WizardAction {
    let max_mode = if crate::channels::voice::local_stt_available() { 3 } else { 2 };
    match key {
        KeyCode::Up | KeyCode::Down => {
            wizard.stt_mode = match key {
                KeyCode::Up => if wizard.stt_mode == 0 { max_mode - 1 } else { wizard.stt_mode - 1 },
                _ => (wizard.stt_mode + 1) % max_mode,
            };
        }
        KeyCode::Tab | KeyCode::Enter => {
            match wizard.stt_mode {
                1 => wizard.voice_field = VoiceField::GroqApiKey,
                2 => { wizard.voice_field = VoiceField::LocalModelSelect; refresh_stt_model_status(wizard); }
                _ => advance_to_next_stt_section(wizard),
            }
        }
        _ => {}
    }
    WizardAction::None
}

fn handle_groq_key(wizard: &mut OnboardingWizard, key: KeyCode) -> WizardAction {
    match key {
        KeyCode::Char(c) => {
            if wizard.has_existing_groq_key() { wizard.groq_api_key_input.clear(); }
            wizard.groq_api_key_input.push(c);
        }
        KeyCode::Backspace => {
            if wizard.has_existing_groq_key() { wizard.groq_api_key_input.clear(); }
            else { wizard.groq_api_key_input.pop(); }
        }
        KeyCode::Tab | KeyCode::Enter => advance_to_next_stt_section(wizard),
        KeyCode::BackTab => { wizard.voice_field = VoiceField::SttModeSelect; }
        _ => {}
    }
    WizardAction::None
}

fn handle_local_model(wizard: &mut OnboardingWizard, key: KeyCode) -> WizardAction {
    match key {
        KeyCode::Up if wizard.selected_local_stt_model > 0 => {
            wizard.selected_local_stt_model -= 1;
            wizard.stt_model_download_error = None;
            refresh_stt_model_status(wizard);
        }
        KeyCode::Down if wizard.selected_local_stt_model + 1 < local_stt_model_count() => {
            wizard.selected_local_stt_model += 1;
            wizard.stt_model_download_error = None;
            refresh_stt_model_status(wizard);
        }
        KeyCode::Enter => {
            if wizard.stt_model_downloaded { advance_to_next_stt_section(wizard); }
            else if wizard.stt_model_download_progress.is_none() {
                return WizardAction::DownloadWhisperModel;
            }
        }
        KeyCode::Tab => advance_to_next_stt_section(wizard),
        KeyCode::BackTab => { wizard.voice_field = VoiceField::SttModeSelect; }
        _ => {}
    }
    WizardAction::None
}

/// Advance past Groq/Local STT to the next section (advanced STT providers or TTS)
fn advance_to_next_stt_section(wizard: &mut OnboardingWizard) {
    // Skip to first enabled advanced STT provider, or TTS mode
    if wizard.stt_openai_compat_enabled {
        wizard.voice_field = VoiceField::SttOpenaiCompatUrl;
    } else if wizard.stt_voicebox_enabled {
        wizard.voice_field = VoiceField::SttVoiceboxUrl;
    } else {
        wizard.voice_field = VoiceField::TtsModeSelect;
    }
}

// ─── Advanced STT providers ─────────────────────────────────────────────────

fn handle_stt_openai_compat_toggle(wizard: &mut OnboardingWizard, key: KeyCode) -> WizardAction {
    match key {
        KeyCode::Char(' ') | KeyCode::Enter | KeyCode::Tab => {
            wizard.stt_openai_compat_enabled = !wizard.stt_openai_compat_enabled;
            wizard.voice_field = if wizard.stt_openai_compat_enabled {
                VoiceField::SttOpenaiCompatUrl
            } else {
                advance_to_next_stt_section_inner(wizard);
                return WizardAction::None;
            };
        }
        KeyCode::BackTab => { wizard.voice_field = last_standard_stt_field(wizard); }
        _ => {}
    }
    WizardAction::None
}

fn handle_stt_voicebox_toggle(wizard: &mut OnboardingWizard, key: KeyCode) -> WizardAction {
    match key {
        KeyCode::Char(' ') | KeyCode::Enter | KeyCode::Tab => {
            wizard.stt_voicebox_enabled = !wizard.stt_voicebox_enabled;
            wizard.voice_field = if wizard.stt_voicebox_enabled {
                VoiceField::SttVoiceboxUrl
            } else {
                if wizard.tts_mode == 0 { wizard.voice_field = VoiceField::TtsModeSelect; }
                else if wizard.tts_mode == 1 || !crate::channels::voice::local_tts_available() {
                    wizard.voice_field = VoiceField::TtsOpenaiCompatToggle;
                } else {
                    wizard.voice_field = VoiceField::TtsModeSelect;
                }
                return WizardAction::None;
            };
        }
        KeyCode::BackTab => { wizard.voice_field = VoiceField::SttOpenaiCompatToggle; }
        _ => {}
    }
    WizardAction::None
}

fn last_standard_stt_field(wizard: &OnboardingWizard) -> VoiceField {
    match wizard.stt_mode {
        1 => VoiceField::GroqApiKey,
        2 => VoiceField::LocalModelSelect,
        _ => VoiceField::SttModeSelect,
    }
}

fn advance_to_next_stt_section_inner(wizard: &OnboardingWizard) -> VoiceField {
    if wizard.stt_voicebox_enabled { VoiceField::SttVoiceboxToggle }
    else if wizard.tts_mode == 0 { VoiceField::TtsModeSelect }
    else { VoiceField::TtsOpenaiCompatToggle }
}

// ─── TTS mode ───────────────────────────────────────────────────────────────

fn handle_tts_mode(wizard: &mut OnboardingWizard, key: KeyCode) -> WizardAction {
    let max_mode = if crate::channels::voice::local_tts_available() { 3 } else { 2 };
    match key {
        KeyCode::Up | KeyCode::Down => {
            wizard.tts_mode = match key {
                KeyCode::Up => if wizard.tts_mode == 0 { max_mode - 1 } else { wizard.tts_mode - 1 },
                _ => (wizard.tts_mode + 1) % max_mode,
            };
            wizard.tts_enabled = wizard.tts_mode != 0;
        }
        KeyCode::Tab | KeyCode::Enter => {
            if wizard.tts_mode == 2 && crate::channels::voice::local_tts_available() {
                wizard.voice_field = VoiceField::TtsLocalVoiceSelect;
                refresh_tts_voice_status(wizard);
            } else {
                advance_to_next_tts_section(wizard);
            }
        }
        KeyCode::BackTab => {
            wizard.voice_field = match wizard.stt_mode {
                1 => VoiceField::GroqApiKey,
                2 => VoiceField::LocalModelSelect,
                _ => last_standard_stt_field(wizard),
            };
        }
        _ => {}
    }
    WizardAction::None
}

fn advance_to_next_tts_section(wizard: &mut OnboardingWizard) {
    if wizard.tts_openai_compat_enabled {
        wizard.voice_field = VoiceField::TtsOpenaiCompatUrl;
    } else if wizard.tts_voicebox_enabled {
        wizard.voice_field = VoiceField::TtsVoiceboxUrl;
    } else {
        wizard.voice_field = VoiceField::Continue;
    }
}

fn handle_tts_voice(wizard: &mut OnboardingWizard, key: KeyCode) -> WizardAction {
    match key {
        KeyCode::Up if wizard.selected_tts_voice > 0 => {
            wizard.selected_tts_voice -= 1;
            wizard.tts_voice_download_error = None;
            wizard.tts_voice_download_progress = None;
            refresh_tts_voice_status(wizard);
        }
        KeyCode::Down if wizard.selected_tts_voice + 1 < tts_voice_count() => {
            wizard.selected_tts_voice += 1;
            wizard.tts_voice_download_error = None;
            wizard.tts_voice_download_progress = None;
            refresh_tts_voice_status(wizard);
        }
        KeyCode::Enter => {
            if wizard.tts_voice_download_progress.is_some() {}
            else if !wizard.tts_voice_downloaded {
                return WizardAction::DownloadPiperVoice;
            }
        }
        KeyCode::Tab => advance_to_next_tts_section(wizard),
        KeyCode::BackTab => { wizard.voice_field = VoiceField::TtsModeSelect; }
        _ => {}
    }
    WizardAction::None
}

// ─── Advanced TTS providers ─────────────────────────────────────────────────

fn handle_tts_openai_compat_toggle(wizard: &mut OnboardingWizard, key: KeyCode) -> WizardAction {
    match key {
        KeyCode::Char(' ') | KeyCode::Enter | KeyCode::Tab => {
            wizard.tts_openai_compat_enabled = !wizard.tts_openai_compat_enabled;
            wizard.voice_field = if wizard.tts_openai_compat_enabled {
                VoiceField::TtsOpenaiCompatUrl
            } else {
                if wizard.tts_voicebox_enabled { wizard.voice_field = VoiceField::TtsVoiceboxToggle; }
                else { wizard.voice_field = VoiceField::Continue; }
                return WizardAction::None;
            };
        }
        KeyCode::BackTab => {
            wizard.voice_field = if wizard.tts_mode == 2 { VoiceField::TtsLocalVoiceSelect }
            else { VoiceField::TtsModeSelect };
        }
        _ => {}
    }
    WizardAction::None
}

fn handle_tts_voicebox_toggle(wizard: &mut OnboardingWizard, key: KeyCode) -> WizardAction {
    match key {
        KeyCode::Char(' ') | KeyCode::Enter | KeyCode::Tab => {
            wizard.tts_voicebox_enabled = !wizard.tts_voicebox_enabled;
            wizard.voice_field = if wizard.tts_voicebox_enabled {
                VoiceField::TtsVoiceboxUrl
            } else {
                wizard.voice_field = VoiceField::Continue;
                return WizardAction::None;
            };
        }
        KeyCode::BackTab => { wizard.voice_field = VoiceField::TtsOpenaiCompatToggle; }
        _ => {}
    }
    WizardAction::None
}

// ─── Generic text field handler ─────────────────────────────────────────────

fn handle_text_field<F>(
    wizard: &mut OnboardingWizard,
    key: KeyCode,
    get_field: F,
    forward: VoiceField,
    back: VoiceField,
) -> WizardAction
where
    F: FnOnce(&mut OnboardingWizard) -> &mut String,
{
    match key {
        KeyCode::Char(c) => { get_field(wizard).push(c); }
        KeyCode::Backspace => { get_field(wizard).pop(); }
        KeyCode::Tab | KeyCode::Enter => { wizard.voice_field = forward; }
        KeyCode::BackTab => { wizard.voice_field = back; }
        _ => {}
    }
    WizardAction::None
}

// ─── Continue ───────────────────────────────────────────────────────────────

fn handle_continue(wizard: &mut OnboardingWizard, key: KeyCode) -> WizardAction {
    match key {
        KeyCode::Enter => { wizard.next_step(); }
        KeyCode::Tab => { wizard.voice_field = VoiceField::SttModeSelect; }
        KeyCode::BackTab => {
            if wizard.tts_voicebox_enabled { wizard.voice_field = VoiceField::TtsVoiceboxProfileId; }
            else if wizard.tts_openai_compat_enabled { wizard.voice_field = VoiceField::TtsOpenaiCompatKey; }
            else if wizard.tts_mode == 2 && crate::channels::voice::local_tts_available() {
                wizard.voice_field = VoiceField::TtsLocalVoiceSelect;
            } else { wizard.voice_field = VoiceField::TtsModeSelect; }
        }
        _ => {}
    }
    WizardAction::None
}

// ─── Status helpers ─────────────────────────────────────────────────────────

fn refresh_stt_model_status(wizard: &mut OnboardingWizard) {
    #[cfg(feature = "local-stt")]
    {
        use crate::channels::voice::local_whisper::{LOCAL_MODEL_PRESETS, is_model_downloaded};
        if wizard.selected_local_stt_model < LOCAL_MODEL_PRESETS.len() {
            wizard.stt_model_downloaded = is_model_downloaded(&LOCAL_MODEL_PRESETS[wizard.selected_local_stt_model]);
        }
    }
    #[cfg(not(feature = "local-stt"))]
    { let _ = wizard; }
}

fn local_stt_model_count() -> usize {
    #[cfg(feature = "local-stt")]
    { crate::channels::voice::local_whisper::LOCAL_MODEL_PRESETS.len() }
    #[cfg(not(feature = "local-stt"))]
    { 0 }
}

fn refresh_tts_voice_status(wizard: &mut OnboardingWizard) {
    #[cfg(feature = "local-tts")]
    {
        use crate::channels::voice::local_tts::{PIPER_VOICES, piper_voice_exists};
        if wizard.selected_tts_voice < PIPER_VOICES.len() {
            wizard.tts_voice_downloaded = piper_voice_exists(PIPER_VOICES[wizard.selected_tts_voice].id);
        }
    }
    #[cfg(not(feature = "local-tts"))]
    { let _ = wizard; }
}

fn tts_voice_count() -> usize {
    #[cfg(feature = "local-tts")]
    { crate::channels::voice::local_tts::PIPER_VOICES.len() }
    #[cfg(not(feature = "local-tts"))]
    { 0 }
}

// ─── Rendering ──────────────────────────────────────────────────────────────

pub fn render(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    if wizard.quick_jump {
        lines.push(Line::from(Span::styled(
            "  Voice Superpowers",
            Style::default().fg(BRAND_GOLD).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "  Talk to me, literally",
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
        )));
        lines.push(Line::from(""));
    }

    render_stt_mode_selector(lines, wizard);
    lines.push(Line::from(""));

    match wizard.stt_mode {
        1 => render_api_fields(lines, wizard),
        2 => render_local_stt_fields(lines, wizard),
        _ => {}
    }

    // Advanced STT providers
    lines.push(Line::from(""));
    render_advanced_stt(lines, wizard);

    lines.push(Line::from(""));
    render_tts_mode_selector(lines, wizard);

    if wizard.tts_mode == 2 {
        lines.push(Line::from(""));
        render_local_tts_fields(lines, wizard);
    }

    // Advanced TTS providers
    lines.push(Line::from(""));
    render_advanced_tts(lines, wizard);

    // Continue button
    lines.push(Line::from(""));
    let continue_focused = wizard.voice_field == VoiceField::Continue;
    if continue_focused {
        lines.push(Line::from(vec![
            Span::styled("  > ", Style::default().fg(ACCENT_GOLD).add_modifier(Modifier::BOLD)),
            Span::styled("Continue", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ]));
    } else {
        lines.push(Line::from(Span::styled("    Continue", Style::default().fg(Color::DarkGray))));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  \u{2191}\u{2193}: select \u{b7} Tab: next field \u{b7} Esc: back \u{b7} Enter: confirm",
        Style::default().fg(Color::DarkGray),
    )));
}

// ─── STT rendering ──────────────────────────────────────────────────────────

fn render_stt_mode_selector(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    let focused = wizard.voice_field == VoiceField::SttModeSelect;

    lines.push(Line::from(Span::styled(
        "  Speech-to-Text",
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        "  Transcribes voice notes from channels",
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));

    render_radio(lines, focused, wizard.stt_mode == 0, "Off");
    render_radio(lines, focused, wizard.stt_mode == 1, "API (Groq Whisper)");
    if crate::channels::voice::local_stt_available() {
        render_radio(lines, focused, wizard.stt_mode == 2, "Local (Whisper \u{2014} runs on device)");
    }
}

fn render_api_fields(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    let focused = wizard.voice_field == VoiceField::GroqApiKey;
    let (masked, hint) = if wizard.has_existing_groq_key() {
        ("**************************", " (already configured)")
    } else if wizard.groq_api_key_input.is_empty() {
        ("get key from console.groq.com", "")
    } else { ("", "") };

    let display = if !wizard.has_existing_groq_key() && !wizard.groq_api_key_input.is_empty() {
        "*".repeat(wizard.groq_api_key_input.len().min(30))
    } else { masked.to_string() };

    let cursor = if focused && !wizard.has_existing_groq_key() { "\u{2588}" } else { "" };

    lines.push(Line::from(vec![
        Span::styled("  Groq Key: ", Style::default().fg(if focused { BRAND_BLUE } else { Color::DarkGray })),
        Span::styled(format!("{}{}", display, cursor), Style::default().fg(if wizard.has_existing_groq_key() { Color::Cyan } else if focused { Color::White } else { Color::DarkGray })),
    ]));

    if !hint.is_empty() && focused {
        lines.push(Line::from(Span::styled(
            format!("  {}", hint.trim()),
            Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
        )));
    }
}

#[allow(unused_variables)]
fn render_local_stt_fields(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    let focused = wizard.voice_field == VoiceField::LocalModelSelect;

    lines.push(Line::from(Span::styled(
        "  Select model size:",
        Style::default().fg(if focused { BRAND_BLUE } else { Color::DarkGray }),
    )));

    #[cfg(feature = "local-stt")]
    {
        use crate::channels::voice::local_whisper::{LOCAL_MODEL_PRESETS, is_model_downloaded};
        for (i, preset) in LOCAL_MODEL_PRESETS.iter().enumerate() {
            let selected = i == wizard.selected_local_stt_model;
            let downloaded = is_model_downloaded(preset);
            let label = format!("{} ({}){}", preset.label, preset.size_label, if downloaded { " \u{2713}" } else { "" });
            render_radio(lines, focused, selected, &label);
        }
    }

    #[cfg(not(feature = "local-stt"))]
    lines.push(Line::from(Span::styled(
        "  Not available (build with --features local-stt)",
        Style::default().fg(Color::Red),
    )));

    if let Some(progress) = wizard.stt_model_download_progress {
        render_progress_bar(lines, progress);
    } else if wizard.stt_model_downloaded {
        lines.push(Line::from(Span::styled(
            "  Model ready \u{2014} press Enter to continue",
            Style::default().fg(Color::Cyan),
        )));
    } else if let Some(ref err) = wizard.stt_model_download_error {
        lines.push(Line::from(Span::styled(
            format!("  Download failed: {}", err),
            Style::default().fg(Color::Red),
        )));
    } else if focused {
        lines.push(Line::from(Span::styled("  Press Enter to download", Style::default().fg(Color::DarkGray))));
    }
}

// ─── Advanced STT providers rendering ───────────────────────────────────────

fn render_advanced_stt(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    lines.push(Line::from(Span::styled(
        "  Additional STT Providers",
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        "  Priority: Voicebox \u{2192} OpenAI-compatible \u{2192} Groq \u{2192} Local",
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));

    // OpenAI-compatible STT
    let toggle_focused = wizard.voice_field == VoiceField::SttOpenaiCompatToggle;
    render_toggle(lines, toggle_focused, wizard.stt_openai_compat_enabled, "OpenAI-compatible STT");

    if wizard.stt_openai_compat_enabled {
        render_text_field(lines, "  Base URL: ", &wizard.stt_openai_compat_base_url,
            "https://api.openai.com/v1",
            wizard.voice_field == VoiceField::SttOpenaiCompatUrl);
        render_text_field(lines, "  Model: ", &wizard.stt_openai_compat_model,
            "whisper-1",
            wizard.voice_field == VoiceField::SttOpenaiCompatModel);
        render_text_field(lines, "  API Key: ", &mask_if_not_empty(&wizard.stt_openai_compat_key_input),
            "",
            wizard.voice_field == VoiceField::SttOpenaiCompatKey);
    }

    lines.push(Line::from(""));

    // Voicebox STT
    let toggle_focused = wizard.voice_field == VoiceField::SttVoiceboxToggle;
    render_toggle(lines, toggle_focused, wizard.stt_voicebox_enabled, "Voicebox STT");

    if wizard.stt_voicebox_enabled {
        render_text_field(lines, "  Base URL: ", &wizard.stt_voicebox_base_url,
            "http://localhost:8000",
            wizard.voice_field == VoiceField::SttVoiceboxUrl);
    }
}

// ─── TTS rendering ──────────────────────────────────────────────────────────

fn render_tts_mode_selector(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    let focused = wizard.voice_field == VoiceField::TtsModeSelect;

    lines.push(Line::from(Span::styled(
        "  Text-to-Speech",
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        "  Reply with voice notes on channels",
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));

    render_radio(lines, focused, wizard.tts_mode == 0, "Off");
    render_radio(lines, focused, wizard.tts_mode == 1, "API (OpenAI TTS \u{2014} uses OpenAI key)");
    if crate::channels::voice::local_tts_available() {
        render_radio(lines, focused, wizard.tts_mode == 2, "Local (Piper \u{2014} runs on device, free)");
    }
}

#[allow(unused_variables)]
fn render_local_tts_fields(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    let focused = wizard.voice_field == VoiceField::TtsLocalVoiceSelect;

    lines.push(Line::from(Span::styled(
        "  Select voice:",
        Style::default().fg(if focused { BRAND_BLUE } else { Color::DarkGray }),
    )));

    #[cfg(feature = "local-tts")]
    {
        use crate::channels::voice::local_tts::{PIPER_VOICES, piper_voice_exists};
        for (i, voice) in PIPER_VOICES.iter().enumerate() {
            let selected = i == wizard.selected_tts_voice;
            let downloaded = piper_voice_exists(voice.id);
            let label = format!("{}{}", voice.label, if downloaded { " \u{2713}" } else { "" });
            render_radio(lines, focused, selected, &label);
        }
    }

    #[cfg(not(feature = "local-tts"))]
    lines.push(Line::from(Span::styled(
        "  Not available (build with --features local-tts)",
        Style::default().fg(Color::Red),
    )));

    if let Some(progress) = wizard.tts_voice_download_progress {
        render_progress_bar(lines, progress);
    } else if wizard.tts_voice_downloaded {
        lines.push(Line::from(Span::styled(
            "  Voice ready \u{2014} press Enter to continue",
            Style::default().fg(Color::Cyan),
        )));
    } else if let Some(ref err) = wizard.tts_voice_download_error {
        lines.push(Line::from(Span::styled(
            format!("  Download failed: {}", err),
            Style::default().fg(Color::Red),
        )));
    } else if focused {
        lines.push(Line::from(Span::styled(
            "  Press Enter to download voice model",
            Style::default().fg(Color::DarkGray),
        )));
    }
}

// ─── Advanced TTS providers rendering ───────────────────────────────────────

fn render_advanced_tts(lines: &mut Vec<Line<'static>>, wizard: &OnboardingWizard) {
    lines.push(Line::from(Span::styled(
        "  Additional TTS Providers",
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        "  Priority: Voicebox \u{2192} OpenAI-compatible \u{2192} OpenAI \u{2192} Local",
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
    )));
    lines.push(Line::from(""));

    // OpenAI-compatible TTS
    let toggle_focused = wizard.voice_field == VoiceField::TtsOpenaiCompatToggle;
    render_toggle(lines, toggle_focused, wizard.tts_openai_compat_enabled, "OpenAI-compatible TTS");

    if wizard.tts_openai_compat_enabled {
        render_text_field(lines, "  Base URL: ", &wizard.tts_openai_compat_base_url,
            "https://api.openai.com/v1",
            wizard.voice_field == VoiceField::TtsOpenaiCompatUrl);
        render_text_field(lines, "  Model: ", &wizard.tts_openai_compat_model,
            "tts-1",
            wizard.voice_field == VoiceField::TtsOpenaiCompatModel);
        render_text_field(lines, "  Voice: ", &wizard.tts_openai_compat_voice,
            "alloy",
            wizard.voice_field == VoiceField::TtsOpenaiCompatVoice);
        render_text_field(lines, "  API Key: ", &mask_if_not_empty(&wizard.tts_openai_compat_key_input),
            "",
            wizard.voice_field == VoiceField::TtsOpenaiCompatKey);
    }

    lines.push(Line::from(""));

    // Voicebox TTS
    let toggle_focused = wizard.voice_field == VoiceField::TtsVoiceboxToggle;
    render_toggle(lines, toggle_focused, wizard.tts_voicebox_enabled, "Voicebox TTS");

    if wizard.tts_voicebox_enabled {
        render_text_field(lines, "  Base URL: ", &wizard.tts_voicebox_base_url,
            "",
            wizard.voice_field == VoiceField::TtsVoiceboxUrl);
        render_text_field(lines, "  Profile ID: ", &wizard.tts_voicebox_profile_id,
            "",
            wizard.voice_field == VoiceField::TtsVoiceboxProfileId);
    }
}

// ─── Shared rendering helpers ───────────────────────────────────────────────

fn render_radio(lines: &mut Vec<Line<'static>>, focused: bool, selected: bool, label: &str) {
    lines.push(Line::from(vec![
        Span::styled(
            if focused && selected { " > " } else { "   " },
            Style::default().fg(ACCENT_GOLD),
        ),
        Span::styled(
            if selected { "(*)" } else { "( )" },
            Style::default().fg(if selected { BRAND_GOLD } else { Color::DarkGray }),
        ),
        Span::styled(
            format!(" {}", label),
            Style::default().fg(if focused && selected { Color::White } else { Color::DarkGray })
                .add_modifier(if focused && selected { Modifier::BOLD } else { Modifier::empty() }),
        ),
    ]));
}

fn render_toggle(lines: &mut Vec<Line<'static>>, focused: bool, enabled: bool, label: &str) {
    let status = if enabled { "ON" } else { "OFF" };
    let status_color = if enabled { Color::Cyan } else { Color::DarkGray };
    lines.push(Line::from(vec![
        Span::styled(
            if focused { " > " } else { "   " },
            Style::default().fg(ACCENT_GOLD),
        ),
        Span::styled(
            format!("[{}] ", status),
            Style::default().fg(status_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {}", label),
            Style::default().fg(if focused { Color::White } else { Color::DarkGray })
                .add_modifier(if focused { Modifier::BOLD } else { Modifier::empty() }),
        ),
    ]));
}

fn mask_if_not_empty(s: &str) -> String {
    if s.is_empty() { String::new() }
    else { "*".repeat(s.len().min(20)) }
}

fn render_text_field(lines: &mut Vec<Line<'static>>, label: &'static str, value: &str, placeholder: &'static str, focused: bool) {
    let display = if value.is_empty() { placeholder.to_string() } else { value.to_string() };
    let color = if focused { BRAND_BLUE } else { Color::DarkGray };
    let text_color = if value.is_empty() { Color::DarkGray } else if focused { Color::White } else { Color::Gray };
    let cursor = if focused && !value.is_empty() { "\u{2588}" } else { "" };

    lines.push(Line::from(vec![
        Span::styled(label, Style::default().fg(color)),
        Span::styled(format!("{}{}", display, cursor), Style::default().fg(text_color)),
    ]));
}

fn render_progress_bar(lines: &mut Vec<Line<'static>>, progress: f64) {
    let pct = (progress * 100.0) as u32;
    let bar_width = 20;
    let filled = (progress * bar_width as f64) as usize;
    let empty = bar_width - filled;
    lines.push(Line::from(vec![
        Span::styled("  ", Style::default()),
        Span::styled("\u{2588}".repeat(filled), Style::default().fg(BRAND_GOLD)),
        Span::styled("\u{2591}".repeat(empty), Style::default().fg(Color::DarkGray)),
        Span::styled(format!(" {}%", pct), Style::default().fg(Color::White)),
    ]));
}
