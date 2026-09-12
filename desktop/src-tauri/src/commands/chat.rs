use crate::AppState;
use opencrabs::brain::agent::AgentService;
use opencrabs::brain::provider::create_provider;
use serde::Serialize;
use tauri::State;
use uuid::Uuid;

#[derive(Serialize)]
pub struct ChatResponse {
    pub message_id: String,
    pub content: String,
    pub model: String,
    pub provider_name: String,
    pub cost: f64,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub tokens_per_second: Option<f64>,
}

/// Parse and validate the session identifier at the IPC boundary, before any
/// provider or agent work begins. Chat uses a single completed request/response
/// command (no long-lived stream), so there is no mid-flight cancellation to
/// perform; the boundary itself is the cancellation gate — a malformed target
/// is rejected here rather than handed to the agent.
fn session_id_from(raw: &str) -> Result<Uuid, String> {
    Uuid::parse_str(raw).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn send_message(
    state: State<'_, AppState>,
    session_id: String,
    message: String,
    model: Option<String>,
) -> Result<ChatResponse, String> {
    let id = session_id_from(&session_id)?;

    let context = {
        let service_manager = state.service_manager.lock().await;
        service_manager
            .as_ref()
            .ok_or("Service not initialized")?
            .context()
            .clone()
    };
    let provider = {
        let config = state.config.read().await;
        create_provider(&config)
            .await
            .map_err(|error| error.to_string())?
    };
    let config = state.config.read().await.clone();
    let agent = AgentService::new(provider, context, &config).await;
    let response = agent
        .send_message(id, message, model)
        .await
        .map_err(|error| error.to_string())?;

    Ok(ChatResponse {
        message_id: response.message_id.to_string(),
        content: response.content,
        model: response.model,
        provider_name: response.provider_name,
        cost: response.cost,
        input_tokens: response.usage.input_tokens,
        output_tokens: response.usage.output_tokens,
        tokens_per_second: response.tokens_per_second,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_malformed_session_id_at_boundary() {
        // Cancellation in the request/response preview is the boundary itself:
        // a malformed target is refused before the agent is ever constructed.
        assert!(session_id_from("not-a-uuid").is_err());
        assert!(session_id_from("").is_err());
        assert!(session_id_from("../escape").is_err());
        assert!(session_id_from("00000000-0000-0000-0000-000000000000").is_ok());
    }
}
