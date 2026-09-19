use serde::Serialize;

#[derive(Serialize)]
pub struct VoiceConfigInfo {
    pub stt_enabled: bool,
    pub stt_provider: String,
    pub tts_enabled: bool,
    pub tts_provider: String,
    pub status: String,
    pub message: String,
}

fn unsupported_message(kind: &str) -> String {
    format!("{kind} is not configured in the desktop app yet")
}

#[tauri::command]
pub async fn get_voice_config() -> Result<VoiceConfigInfo, String> {
    Ok(VoiceConfigInfo {
        stt_enabled: false,
        stt_provider: "unavailable".to_string(),
        tts_enabled: false,
        tts_provider: "unavailable".to_string(),
        status: "unsupported".to_string(),
        message: "Desktop voice controls are unavailable until local STT/TTS is wired into the native shell.".to_string(),
    })
}

#[tauri::command]
pub async fn transcribe_audio(_audio_path: String) -> Result<String, String> {
    Err(unsupported_message("Speech-to-text"))
}

#[tauri::command]
pub async fn synthesize_speech(_text: String) -> Result<String, String> {
    Err(unsupported_message("Text-to-speech"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_message_is_honest() {
        assert!(unsupported_message("Speech-to-text").contains("not configured"));
    }
}
