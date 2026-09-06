use serde::Serialize;

#[derive(Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
    pub release_notes: String,
    pub date: String,
}

fn manual_install_message() -> String {
    "Desktop update install is intentionally unsupported; use a signed release artifact or /evolve from OpenCrabs itself.".to_string()
}

#[tauri::command]
pub async fn check_for_updates() -> Result<Option<UpdateInfo>, String> {
    match opencrabs::brain::tools::evolve::check_for_update().await {
        Some(version) => Ok(Some(UpdateInfo {
            version,
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            release_notes: "Manual install only: download the signed release artifact or use /evolve from the running OpenCrabs runtime.".to_string(),
            date: "unknown".to_string(),
        })),
        None => Ok(None),
    }
}

#[tauri::command]
pub async fn install_update() -> Result<(), String> {
    Err(manual_install_message())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_update_message_is_explicitly_manual() {
        let msg = manual_install_message();
        assert!(msg.contains("unsupported"));
        assert!(msg.contains("signed release artifact"));
    }
}
