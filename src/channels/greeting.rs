//! Channel connection greeting generator.
//!
//! Generates personalized connection confirmation messages via the LLM.
//! If brain/persona files exist (returning user), the greeting is personalized
//! with the user's name. Otherwise (first onboard), it's a casual first-person
//! confirmation.

use crate::brain::agent::AgentService;
use crate::brain::provider::types::{ContentBlock, LLMRequest};

/// Generate a personalized connection greeting via the LLM.
pub async fn generate_connection_greeting(agent: &AgentService, channel_name: &str) -> String {
    let brain_path = crate::brain::BrainLoader::resolve_path();
    let has_brain =
        brain_path.join("persona.md").exists() || brain_path.join("system.md").exists();

    let prompt = if has_brain {
        format!(
            "You just successfully connected to {channel}. \
             Generate a short, warm, first-person greeting message (1-2 sentences max) \
             confirming the connection. Be personal — use the user's name if you know it. \
             Be yourself based on your persona. No markdown, no emojis unless it fits your style. \
             Reply with ONLY the greeting text, nothing else.",
            channel = channel_name,
        )
    } else {
        format!(
            "You just successfully connected to {channel}. \
             Generate a short, cool, first-person greeting message (1-2 sentences max) \
             confirming the connection succeeded. Be casual and confident. \
             No markdown. Reply with ONLY the greeting text, nothing else.",
            channel = channel_name,
        )
    };

    let model = agent.provider_model();
    let request = LLMRequest::new(
        model,
        vec![crate::brain::provider::types::Message::user(prompt)],
    )
    .with_system("You are OpenCrabs, an AI assistant. Respond with only the requested text.");

    let provider = agent.provider();
    match provider.complete(request).await {
        Ok(response) => {
            let text = response
                .content
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("")
                .trim()
                .to_string();
            if text.is_empty() {
                fallback(channel_name)
            } else {
                text
            }
        }
        Err(e) => {
            tracing::warn!("Failed to generate channel greeting: {}", e);
            fallback(channel_name)
        }
    }
}

fn fallback(channel_name: &str) -> String {
    format!("Connected to {} — I'm here and ready.", channel_name)
}
