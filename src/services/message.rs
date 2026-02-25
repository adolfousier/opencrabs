//! Message Service
//!
//! Provides business logic for message management operations.

use crate::db::{models::Message, repository::MessageRepository};
use crate::services::ServiceContext;
use anyhow::{Context, Result};
use chrono::Utc;
use uuid::Uuid;

/// Service for managing messages
#[derive(Clone)]
pub struct MessageService {
    context: ServiceContext,
}

impl MessageService {
    /// Create a new message service
    pub fn new(context: ServiceContext) -> Self {
        Self { context }
    }

    /// Create a new message
    pub async fn create_message(
        &self,
        session_id: Uuid,
        role: String,
        content: String,
    ) -> Result<Message> {
        let repo = MessageRepository::new(self.context.pool());

        // Get the next sequence number for this session
        let sequence = self.get_next_sequence(session_id).await?;

        let message = Message {
            id: Uuid::new_v4(),
            session_id,
            role,
            content,
            sequence,
            created_at: Utc::now(),
            token_count: None,
            cost: None,
        };

        repo.create(&message)
            .await
            .context("Failed to create message")?;

        tracing::debug!(
            "Created new message: {} in session {} (seq: {})",
            message.id,
            session_id,
            sequence
        );
        Ok(message)
    }

    /// Get a message by ID
    pub async fn get_message(&self, id: Uuid) -> Result<Option<Message>> {
        let repo = MessageRepository::new(self.context.pool());
        repo.find_by_id(id).await.context("Failed to get message")
    }

    /// Get a message by ID, returning an error if not found
    pub async fn get_message_required(&self, id: Uuid) -> Result<Message> {
        self.get_message(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Message not found: {}", id))
    }

    /// List all messages for a session
    pub async fn list_messages_for_session(&self, session_id: Uuid) -> Result<Vec<Message>> {
        let repo = MessageRepository::new(self.context.pool());
        repo.find_by_session(session_id)
            .await
            .context("Failed to list messages for session")
    }

    /// Update a message
    pub async fn update_message(&self, message: &Message) -> Result<()> {
        let repo = MessageRepository::new(self.context.pool());
        repo.update(message)
            .await
            .context("Failed to update message")?;

        tracing::debug!("Updated message: {}", message.id);
        Ok(())
    }

    /// Update message usage statistics
    pub async fn update_message_usage(&self, id: Uuid, token_count: i32, cost: f64) -> Result<()> {
        let mut message = self.get_message_required(id).await?;
        message.token_count = Some(token_count);
        message.cost = Some(cost);

        let repo = MessageRepository::new(self.context.pool());
        repo.update(&message)
            .await
            .context("Failed to update message usage")?;

        tracing::debug!(
            "Updated message usage: {} ({} tokens, ${:.4})",
            id,
            token_count,
            cost
        );
        Ok(())
    }

    /// Append content to an existing message (for real-time history persistence)
    pub async fn append_content(&self, id: Uuid, content_to_append: &str) -> Result<()> {
        let repo = MessageRepository::new(self.context.pool());
        repo.append_content(id, content_to_append)
            .await
            .context("Failed to append to message")?;
        Ok(())
    }

    /// Delete a message
    pub async fn delete_message(&self, id: Uuid) -> Result<()> {
        let repo = MessageRepository::new(self.context.pool());
        repo.delete(id).await.context("Failed to delete message")?;

        tracing::debug!("Deleted message: {}", id);
        Ok(())
    }

    /// Delete all messages for a session
    pub async fn delete_messages_for_session(&self, session_id: Uuid) -> Result<()> {
        let repo = MessageRepository::new(self.context.pool());
        repo.delete_by_session(session_id)
            .await
            .context("Failed to delete messages for session")?;

        tracing::info!("Deleted messages for session {}", session_id);
        Ok(())
    }

    /// Count messages in a session
    pub async fn count_messages_in_session(&self, session_id: Uuid) -> Result<i64> {
        let repo = MessageRepository::new(self.context.pool());
        repo.count_by_session(session_id)
            .await
            .context("Failed to count messages in session")
    }

    /// Get the next sequence number for a session
    async fn get_next_sequence(&self, session_id: Uuid) -> Result<i32> {
        let count = self.count_messages_in_session(session_id).await?;
        Ok((count + 1) as i32)
    }

    /// Get the last message in a session
    pub async fn get_last_message(&self, session_id: Uuid) -> Result<Option<Message>> {
        let messages = self.list_messages_for_session(session_id).await?;
        Ok(messages.into_iter().last())
    }

    /// Get messages by role
    pub async fn get_messages_by_role(&self, session_id: Uuid, role: &str) -> Result<Vec<Message>> {
        let messages = self.list_messages_for_session(session_id).await?;
        Ok(messages.into_iter().filter(|m| m.role == role).collect())
    }

    /// Calculate total tokens for a session
    pub async fn calculate_total_tokens(&self, session_id: Uuid) -> Result<i32> {
        let messages = self.list_messages_for_session(session_id).await?;
        let total = messages.iter().filter_map(|m| m.token_count).sum();
        Ok(total)
    }

    /// Calculate total cost for a session
    pub async fn calculate_total_cost(&self, session_id: Uuid) -> Result<f64> {
        let messages = self.list_messages_for_session(session_id).await?;
        let total = messages.iter().filter_map(|m| m.cost).sum();
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::SessionService;

    async fn create_test_service() -> (MessageService, SessionService) {
        use crate::db::Database;

        let db = Database::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        let pool = db.pool().clone();

        let context = ServiceContext::new(pool);
        (
            MessageService::new(context.clone()),
            SessionService::new(context),
        )
    }

    #[tokio::test]
    async fn test_create_message() {
        let (message_service, session_service) = create_test_service().await;
        let session = session_service
            .create_session(Some("Test".to_string()))
            .await
            .unwrap();

        let message = message_service
            .create_message(session.id, "user".to_string(), "Hello".to_string())
            .await
            .unwrap();

        assert_eq!(message.session_id, session.id);
        assert_eq!(message.role, "user");
        assert_eq!(message.content, "Hello");
        assert_eq!(message.sequence, 1);
    }

    #[tokio::test]
    async fn test_get_message() {
        let (message_service, session_service) = create_test_service().await;
        let session = session_service
            .create_session(Some("Test".to_string()))
            .await
            .unwrap();

        let created = message_service
            .create_message(session.id, "user".to_string(), "Test".to_string())
            .await
            .unwrap();

        let found = message_service.get_message(created.id).await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, created.id);
    }

    #[tokio::test]
    async fn test_list_messages_for_session() {
        let (message_service, session_service) = create_test_service().await;
        let session = session_service
            .create_session(Some("Test".to_string()))
            .await
            .unwrap();

        message_service
            .create_message(session.id, "user".to_string(), "Message 1".to_string())
            .await
            .unwrap();
        message_service
            .create_message(session.id, "assistant".to_string(), "Message 2".to_string())
            .await
            .unwrap();

        let messages = message_service
            .list_messages_for_session(session.id)
            .await
            .unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].sequence, 1);
        assert_eq!(messages[1].sequence, 2);
    }

    #[tokio::test]
    async fn test_update_message_usage() {
        let (message_service, session_service) = create_test_service().await;
        let session = session_service
            .create_session(Some("Test".to_string()))
            .await
            .unwrap();

        let message = message_service
            .create_message(session.id, "user".to_string(), "Test".to_string())
            .await
            .unwrap();

        message_service
            .update_message_usage(message.id, 100, 0.05)
            .await
            .unwrap();

        let updated = message_service
            .get_message_required(message.id)
            .await
            .unwrap();
        assert_eq!(updated.token_count, Some(100));
        assert_eq!(updated.cost, Some(0.05));
    }

    #[tokio::test]
    async fn test_delete_message() {
        let (message_service, session_service) = create_test_service().await;
        let session = session_service
            .create_session(Some("Test".to_string()))
            .await
            .unwrap();

        let message = message_service
            .create_message(session.id, "user".to_string(), "Test".to_string())
            .await
            .unwrap();

        message_service.delete_message(message.id).await.unwrap();

        let result = message_service.get_message(message.id).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_delete_messages_for_session() {
        let (message_service, session_service) = create_test_service().await;
        let session = session_service
            .create_session(Some("Test".to_string()))
            .await
            .unwrap();

        message_service
            .create_message(session.id, "user".to_string(), "Message 1".to_string())
            .await
            .unwrap();
        message_service
            .create_message(session.id, "assistant".to_string(), "Message 2".to_string())
            .await
            .unwrap();

        message_service
            .delete_messages_for_session(session.id)
            .await
            .unwrap();

        let messages = message_service
            .list_messages_for_session(session.id)
            .await
            .unwrap();
        assert_eq!(messages.len(), 0);
    }

    #[tokio::test]
    async fn test_count_messages_in_session() {
        let (message_service, session_service) = create_test_service().await;
        let session = session_service
            .create_session(Some("Test".to_string()))
            .await
            .unwrap();

        message_service
            .create_message(session.id, "user".to_string(), "Message 1".to_string())
            .await
            .unwrap();
        message_service
            .create_message(session.id, "assistant".to_string(), "Message 2".to_string())
            .await
            .unwrap();

        let count = message_service
            .count_messages_in_session(session.id)
            .await
            .unwrap();
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn test_get_last_message() {
        let (message_service, session_service) = create_test_service().await;
        let session = session_service
            .create_session(Some("Test".to_string()))
            .await
            .unwrap();

        message_service
            .create_message(session.id, "user".to_string(), "First".to_string())
            .await
            .unwrap();
        let last = message_service
            .create_message(session.id, "assistant".to_string(), "Last".to_string())
            .await
            .unwrap();

        let result = message_service.get_last_message(session.id).await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, last.id);
    }

    #[tokio::test]
    async fn test_get_messages_by_role() {
        let (message_service, session_service) = create_test_service().await;
        let session = session_service
            .create_session(Some("Test".to_string()))
            .await
            .unwrap();

        message_service
            .create_message(session.id, "user".to_string(), "User 1".to_string())
            .await
            .unwrap();
        message_service
            .create_message(
                session.id,
                "assistant".to_string(),
                "Assistant 1".to_string(),
            )
            .await
            .unwrap();
        message_service
            .create_message(session.id, "user".to_string(), "User 2".to_string())
            .await
            .unwrap();

        let user_messages = message_service
            .get_messages_by_role(session.id, "user")
            .await
            .unwrap();
        assert_eq!(user_messages.len(), 2);

        let assistant_messages = message_service
            .get_messages_by_role(session.id, "assistant")
            .await
            .unwrap();
        assert_eq!(assistant_messages.len(), 1);
    }

    #[tokio::test]
    async fn test_calculate_totals() {
        let (message_service, session_service) = create_test_service().await;
        let session = session_service
            .create_session(Some("Test".to_string()))
            .await
            .unwrap();

        let msg1 = message_service
            .create_message(session.id, "user".to_string(), "Message 1".to_string())
            .await
            .unwrap();
        message_service
            .update_message_usage(msg1.id, 100, 0.05)
            .await
            .unwrap();

        let msg2 = message_service
            .create_message(session.id, "assistant".to_string(), "Message 2".to_string())
            .await
            .unwrap();
        message_service
            .update_message_usage(msg2.id, 200, 0.10)
            .await
            .unwrap();

        let total_tokens = message_service
            .calculate_total_tokens(session.id)
            .await
            .unwrap();
        let total_cost = message_service
            .calculate_total_cost(session.id)
            .await
            .unwrap();

        assert_eq!(total_tokens, 300);
        assert!((total_cost - 0.15).abs() < 0.0001);
    }
}
