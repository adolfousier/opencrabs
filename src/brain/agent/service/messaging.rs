use super::builder::AgentService;
use super::types::*;
use crate::brain::agent::error::{AgentError, Result};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

impl AgentService {
    /// Send a message and get a response
    ///
    /// This will:
    /// 1. Load conversation context from the database
    /// 2. Add the new user message
    /// 3. Send to the LLM provider
    /// 4. Save the response to the database
    /// 5. Update token usage
    pub async fn send_message(
        &self,
        session_id: Uuid,
        user_message: String,
        model: Option<String>,
    ) -> Result<AgentResponse> {
        // Prepare message context (common setup logic)
        let (_model_name, request, message_service, session_service) = self
            .prepare_message_context(session_id, user_message, model)
            .await?;

        // Send to provider
        let provider = self
            .provider
            .read()
            .expect("provider lock poisoned")
            .clone();
        let response = provider
            .complete(request)
            .await
            .map_err(AgentError::Provider)?;

        // Extract text from response
        let assistant_text = Self::extract_text_from_response(&response);

        // Save assistant response to database
        let assistant_db_msg = message_service
            .create_message(session_id, "assistant".to_string(), assistant_text.clone())
            .await
            .map_err(|e| AgentError::Database(e.to_string()))?;

        // Calculate total tokens and cost for this message
        let total_tokens = response.usage.input_tokens + response.usage.output_tokens;
        let cost = self
            .provider
            .read()
            .expect("provider lock poisoned")
            .calculate_cost(
                &response.model,
                response.usage.input_tokens,
                response.usage.output_tokens,
            );

        // Update message with usage info
        message_service
            .update_message_usage(assistant_db_msg.id, total_tokens as i32, cost)
            .await
            .map_err(|e| AgentError::Database(e.to_string()))?;

        // Update session token usage
        session_service
            .update_session_usage(session_id, total_tokens as i32, cost)
            .await
            .map_err(|e| AgentError::Database(e.to_string()))?;

        Ok(AgentResponse {
            message_id: assistant_db_msg.id,
            content: assistant_text,
            stop_reason: response.stop_reason,
            context_tokens: response.usage.input_tokens,
            usage: response.usage,
            cost,
            model: response.model,
        })
    }

    /// Send a message and get a streaming response
    ///
    /// Returns a stream of response chunks that can be consumed incrementally.
    pub async fn send_message_streaming(
        &self,
        session_id: Uuid,
        user_message: String,
        model: Option<String>,
    ) -> Result<AgentStreamResponse> {
        // Prepare message context (common setup logic)
        let (model_name, request, _message_service, _session_service) = self
            .prepare_message_context(session_id, user_message, model)
            .await?;

        // Add streaming flag to request
        let request = request.with_streaming();

        // Get streaming response from provider
        let provider = self
            .provider
            .read()
            .expect("provider lock poisoned")
            .clone();
        let stream = provider
            .stream(request)
            .await
            .map_err(AgentError::Provider)?;

        Ok(AgentStreamResponse {
            session_id,
            message_id: Uuid::new_v4(),
            stream,
            model: model_name,
        })
    }

    /// Send a message with automatic tool execution
    ///
    /// This method implements a tool execution loop:
    /// 1. Send message to LLM
    /// 2. If LLM requests tool use, execute the tool
    /// 3. Send tool results back to LLM
    /// 4. Repeat until LLM finishes or max iterations reached
    pub async fn send_message_with_tools(
        &self,
        session_id: Uuid,
        user_message: String,
        model: Option<String>,
    ) -> Result<AgentResponse> {
        self.send_message_with_tools_and_mode(session_id, user_message, model, None)
            .await
    }

    /// Shim: send with tools + optional cancellation token.
    /// Delegates to `run_tool_loop` with service-level callbacks.
    pub async fn send_message_with_tools_and_mode(
        &self,
        session_id: Uuid,
        user_message: String,
        model: Option<String>,
        cancel_token: Option<CancellationToken>,
    ) -> Result<AgentResponse> {
        self.run_tool_loop(session_id, user_message, model, cancel_token, None, None)
            .await
    }

    /// Send a message with per-call callback overrides.
    ///
    /// `override_approval_callback` and `override_progress_callback` take
    /// precedence over the service-level callbacks (used by Telegram, Discord, etc.).
    /// Pass `None` to fall back to the service-level callback.
    pub async fn send_message_with_tools_and_callback(
        &self,
        session_id: Uuid,
        user_message: String,
        model: Option<String>,
        cancel_token: Option<CancellationToken>,
        override_approval_callback: Option<ApprovalCallback>,
        override_progress_callback: Option<ProgressCallback>,
    ) -> Result<AgentResponse> {
        self.run_tool_loop(
            session_id,
            user_message,
            model,
            cancel_token,
            override_approval_callback,
            override_progress_callback,
        )
        .await
    }
}
