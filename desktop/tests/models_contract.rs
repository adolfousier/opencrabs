use opencrabs_desktop_ui::models::{
    ChannelStatus, CronJobRunInfo, DiagnosticsSnapshot, MessageInfo, SessionInfo, SkillInfo,
    UpdateInfo, VoiceConfigInfo,
};
use serde_json::json;

#[test]
fn channel_status_json_round_trip_preserves_error_field() {
    let value = json!({
        "name": "telegram",
        "display_name": "Telegram Bot",
        "enabled": true,
        "alive": false,
        "error": "Missing credentials/config"
    });

    let parsed: ChannelStatus =
        serde_json::from_value(value.clone()).expect("channel status deserialize");
    assert_eq!(parsed.name, "telegram");
    assert_eq!(parsed.error.as_deref(), Some("Missing credentials/config"));

    let encoded = serde_json::to_value(parsed).expect("channel status serialize");
    assert_eq!(encoded, value);
}

#[test]
fn update_info_json_round_trip_matches_backend_shape() {
    let value = json!({
        "version": "0.2.0",
        "current_version": "0.1.0",
        "release_notes": "Tighter CSP and safer bridge",
        "date": "2026-07-25"
    });

    let parsed: UpdateInfo =
        serde_json::from_value(value.clone()).expect("update info deserialize");
    assert_eq!(parsed.version, "0.2.0");
    assert_eq!(parsed.current_version, "0.1.0");

    let encoded = serde_json::to_value(parsed).expect("update info serialize");
    assert_eq!(encoded, value);
}

#[test]
fn diagnostics_snapshot_json_round_trip_preserves_safe_log_tail() {
    let value = json!({
        "app_version": "0.1.0",
        "config_present": true,
        "database_present": false,
        "log_path": "/safe/log",
        "log_tail": ["request completed"],
        "notes": ["Log preview is redacted"]
    });

    let parsed: DiagnosticsSnapshot =
        serde_json::from_value(value.clone()).expect("diagnostics deserialize");
    assert_eq!(parsed.log_tail, vec!["request completed"]);
    assert!(!parsed.database_present);
    assert_eq!(
        serde_json::to_value(parsed).expect("diagnostics serialize"),
        value
    );
}

#[test]
fn voice_contract_declares_unsupported_state() {
    let value = json!({
        "stt_enabled": false,
        "stt_provider": "unavailable",
        "tts_enabled": false,
        "tts_provider": "unavailable",
        "status": "unsupported",
        "message": "Desktop voice controls are unavailable until local STT/TTS is wired into the native shell."
    });

    let parsed: VoiceConfigInfo =
        serde_json::from_value(value.clone()).expect("voice config deserialize");
    assert_eq!(parsed.status, "unsupported");
    assert!(!parsed.stt_enabled && !parsed.tts_enabled);
    assert_eq!(
        serde_json::to_value(parsed).expect("voice config serialize"),
        value
    );
}

#[test]
fn session_json_round_trip_matches_backend_shape() {
    let value = json!({
        "id": "session-1",
        "title": "Desktop session",
        "model": "gpt-5.4",
        "provider_name": "surplus",
        "working_directory": "/workspace",
        "token_count": 42,
        "total_cost": 0.05,
        "created_at": "2026-07-25T12:00:00Z",
        "updated_at": "2026-07-25T12:01:00Z",
        "is_archived": false,
        "project_id": "project-1",
        "project_name": "Crabz Desktop"
    });

    let parsed: SessionInfo = serde_json::from_value(value.clone()).expect("session deserialize");
    assert_eq!(parsed.id, "session-1");
    assert_eq!(parsed.provider_name.as_deref(), Some("surplus"));
    assert_eq!(parsed.project_name.as_deref(), Some("Crabz Desktop"));
    assert_eq!(
        serde_json::to_value(parsed).expect("session serialize"),
        value
    );
}

#[test]
fn skill_json_round_trip_includes_enabled_state() {
    let value = json!({
        "name": "review",
        "description": "Review a repository",
        "source": "builtin",
        "review_gate": true,
        "enabled": false
    });

    let parsed: SkillInfo = serde_json::from_value(value.clone()).expect("skill deserialize");
    assert!(!parsed.enabled);
    assert!(parsed.review_gate);
    assert_eq!(
        serde_json::to_value(parsed).expect("skill serialize"),
        value
    );
}

#[test]
fn message_json_round_trip_preserves_persisted_thinking() {
    let value = json!({
        "id": "message-1",
        "role": "assistant",
        "content": "A concise answer",
        "sequence": 4,
        "token_count": 29,
        "cost": 0.004,
        "created_at": "2026-07-25T12:00:02Z",
        "thinking": "Inspect the request before responding."
    });

    let parsed: MessageInfo = serde_json::from_value(value.clone()).expect("message deserialize");
    assert_eq!(
        parsed.thinking.as_deref(),
        Some("Inspect the request before responding.")
    );
    assert_eq!(
        serde_json::to_value(parsed).expect("message serialize"),
        value
    );
}

#[test]
fn cron_run_json_round_trip_matches_backend_shape() {
    let value = json!({
        "id": "run-1",
        "job_id": "job-1",
        "job_name": "Daily summary",
        "status": "success",
        "content": "completed",
        "error": null,
        "input_tokens": 12,
        "output_tokens": 34,
        "cost": 0.01,
        "started_at": "2026-07-25T12:00:00Z",
        "completed_at": "2026-07-25T12:00:02Z"
    });

    let parsed: CronJobRunInfo =
        serde_json::from_value(value.clone()).expect("cron run deserialize");
    assert_eq!(parsed.job_id, "job-1");
    assert_eq!(parsed.input_tokens + parsed.output_tokens, 46);
    assert_eq!(
        serde_json::to_value(parsed).expect("cron run serialize"),
        value
    );
}
