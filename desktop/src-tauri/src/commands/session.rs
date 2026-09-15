use crate::AppState;
use serde::Serialize;
use std::collections::HashMap;
use tauri::State;
use uuid::Uuid;

#[derive(Serialize)]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    pub model: Option<String>,
    pub provider_name: Option<String>,
    pub working_directory: Option<String>,
    pub token_count: i64,
    pub total_cost: f64,
    pub created_at: String,
    pub updated_at: String,
    pub is_archived: bool,
    /// Stable project identity when the session is assigned to a project.
    pub project_id: Option<String>,
    /// Resolved project label; absent for unassigned or deleted projects.
    pub project_name: Option<String>,
}

#[derive(Serialize)]
pub struct MessageInfo {
    pub id: String,
    pub role: String,
    pub content: String,
    pub sequence: i32,
    pub token_count: Option<i64>,
    pub cost: Option<f64>,
    pub created_at: String,
    pub thinking: Option<String>,
}

fn session_to_info(
    session: &opencrabs::db::models::Session,
    project_names: &HashMap<Uuid, String>,
) -> SessionInfo {
    SessionInfo {
        id: session.id.to_string(),
        title: session.title.clone().unwrap_or_else(|| "Untitled".into()),
        model: session.model.clone(),
        provider_name: session.provider_name.clone(),
        working_directory: session.working_directory.clone(),
        token_count: session.token_count,
        total_cost: session.total_cost,
        created_at: session.created_at.to_rfc3339(),
        updated_at: session.updated_at.to_rfc3339(),
        is_archived: session.is_archived(),
        project_id: session.project_id.map(|id| id.to_string()),
        project_name: session
            .project_id
            .and_then(|id| project_names.get(&id).cloned()),
    }
}

fn message_to_info(message: &opencrabs::db::models::Message) -> MessageInfo {
    MessageInfo {
        id: message.id.to_string(),
        role: message.role.clone(),
        content: message.content.clone(),
        sequence: message.sequence,
        token_count: message.token_count,
        cost: message.cost,
        created_at: message.created_at.to_rfc3339(),
        thinking: message.thinking.clone(),
    }
}

async fn session_metadata(
    state: &State<'_, AppState>,
) -> Result<(Vec<opencrabs::db::models::Session>, HashMap<Uuid, String>), String> {
    let service_manager = state.service_manager.lock().await;
    let service_manager = service_manager.as_ref().ok_or("Service not initialized")?;

    let sessions = service_manager
        .sessions()
        .list_sessions(opencrabs::db::repository::SessionListOptions {
            include_archived: false,
            limit: Some(100),
            offset: 0,
            query: None,
        })
        .await
        .map_err(|error| error.to_string())?;

    let project_names = service_manager
        .projects()
        .list_projects()
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|project| (project.id, project.name))
        .collect();

    Ok((sessions, project_names))
}

#[tauri::command]
pub async fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionInfo>, String> {
    let (sessions, project_names) = session_metadata(&state).await?;
    Ok(sessions
        .iter()
        .map(|session| session_to_info(session, &project_names))
        .collect())
}

#[tauri::command]
pub async fn create_session(
    state: State<'_, AppState>,
    title: String,
) -> Result<SessionInfo, String> {
    let service_manager = state.service_manager.lock().await;
    let service_manager = service_manager.as_ref().ok_or("Service not initialized")?;
    let session = service_manager
        .sessions()
        .create_session(Some(title))
        .await
        .map_err(|error| error.to_string())?;
    Ok(session_to_info(&session, &HashMap::new()))
}

#[tauri::command]
pub async fn rename_session(
    state: State<'_, AppState>,
    session_id: String,
    title: String,
) -> Result<(), String> {
    let service_manager = state.service_manager.lock().await;
    let service_manager = service_manager.as_ref().ok_or("Service not initialized")?;
    let id = Uuid::parse_str(&session_id).map_err(|error| error.to_string())?;
    service_manager
        .sessions()
        .update_session_title(id, Some(title))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn delete_session(state: State<'_, AppState>, session_id: String) -> Result<(), String> {
    let service_manager = state.service_manager.lock().await;
    let service_manager = service_manager.as_ref().ok_or("Service not initialized")?;
    let id = Uuid::parse_str(&session_id).map_err(|error| error.to_string())?;
    service_manager
        .sessions()
        .delete_session(id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_session_messages(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<MessageInfo>, String> {
    let service_manager = state.service_manager.lock().await;
    let service_manager = service_manager.as_ref().ok_or("Service not initialized")?;
    let id = Uuid::parse_str(&session_id).map_err(|error| error.to_string())?;
    let messages = service_manager
        .messages()
        .list_messages_for_session(id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(messages.iter().map(message_to_info).collect())
}
