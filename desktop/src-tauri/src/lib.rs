use tauri::Manager;

mod commands;

use commands::{
    brain, channels, chat, config_cmd, cron, diagnostics, files, mcp, onboarding, panes, session,
    skills, tools, update, usage, voice,
};

pub struct AppState {
    pub service_manager: tokio::sync::Mutex<Option<opencrabs::services::ServiceManager>>,
    pub config: std::sync::Arc<tokio::sync::RwLock<opencrabs::config::Config>>,
}

fn init_service_manager(
    db_path: &std::path::Path,
) -> Result<opencrabs::services::ServiceManager, Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| {
            tracing::error!("Failed to create Tokio runtime for app setup: {}", e);
            Box::new(std::io::Error::other(e.to_string())) as Box<dyn std::error::Error>
        })?;

    let db = runtime
        .block_on(opencrabs::db::Database::connect(db_path))
        .map_err(|e| {
            tracing::error!("Database init failed: {}", e);
            Box::new(std::io::Error::other(e.to_string())) as Box<dyn std::error::Error>
        })?;

    Ok(opencrabs::services::ServiceManager::new(db.pool().clone()))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Env-gated: when OPENCRABS_DESKTOP_SMOKE is set, emit a deterministic
    // backend-ready line on stderr so the release smoke procedure can prove the
    // packaged binary reached IPC-readiness (config + db + state + handler).
    // Off in normal use — nothing changes for end users.
    let smoke_marker = std::env::var("OPENCRABS_DESKTOP_SMOKE").is_ok();

    tauri::Builder::default()
        .setup(move |app| {
            let config = opencrabs::config::Config::load().map_err(|e| {
                tracing::error!("Failed to load config: {}", e);
                Box::new(std::io::Error::other(e.to_string())) as Box<dyn std::error::Error>
            })?;

            let service_manager = init_service_manager(&config.database.path)?;

            let app_state = AppState {
                service_manager: tokio::sync::Mutex::new(Some(service_manager)),
                config: std::sync::Arc::new(tokio::sync::RwLock::new(config)),
            };

            app.manage(app_state);
            if smoke_marker {
                eprintln!("desktop_smoke: backend_ready config_loaded db_open state_managed");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            chat::send_message,
            session::list_sessions,
            session::create_session,
            session::rename_session,
            session::delete_session,
            session::get_session_messages,
            config_cmd::get_config,
            config_cmd::get_providers,
            config_cmd::select_model,
            config_cmd::update_config,
            brain::list_brain_files,
            brain::read_brain_file,
            brain::write_brain_file,
            tools::list_tools,
            tools::get_tool_details,
            tools::approve_tool,
            skills::list_skills,
            skills::get_skill_details,
            skills::toggle_skill,
            cron::list_cron_jobs,
            cron::create_cron_job,
            cron::delete_cron_job,
            cron::toggle_cron_job,
            cron::trigger_cron_job,
            cron::list_cron_runs,
            channels::get_channel_statuses,
            channels::toggle_channel,
            mcp::list_dynamic_tools,
            mcp::add_dynamic_tool,
            mcp::remove_dynamic_tool,
            usage::get_usage_data,
            files::list_directory,
            files::read_file_content,
            files::get_workspace_root,
            onboarding::is_first_time_setup,
            onboarding::get_available_providers,
            onboarding::validate_api_key,
            onboarding::save_onboarding_config,
            onboarding::run_health_check,
            diagnostics::get_diagnostics,
            update::check_for_updates,
            update::install_update,
            panes::get_pane_layout,
            panes::split_pane,
            panes::close_pane,
            panes::set_pane_session,
            panes::get_desktop_state,
            panes::save_desktop_state,
            voice::get_voice_config,
            voice::transcribe_audio,
            voice::synthesize_speech,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
