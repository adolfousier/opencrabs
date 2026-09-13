//! Agent Context Management
//!
//! Manages conversation context including messages, system brain,
//! and token tracking.

use crate::brain::provider::{ContentBlock, Message, Role};
use crate::brain::tokenizer;
use crate::db::models::Message as DbMessage;
use std::path::PathBuf;
use uuid::Uuid;

/// Agent context for a conversation
#[derive(Debug, Clone)]
pub struct AgentContext {
    /// Session ID
    pub session_id: Uuid,

    /// System brain
    pub system_brain: Option<String>,

    /// Conversation messages
    pub messages: Vec<Message>,

    /// Tracked files in the conversation
    pub tracked_files: Vec<TrackedFile>,

    /// Current token count estimate
    pub token_count: usize,

    /// Maximum context tokens
    pub max_tokens: usize,

    /// The provider's own count of the last request it received, when it
    /// reported one.
    ///
    /// `token_count` is a tiktoken estimate of the system prompt plus the
    /// messages. It leaves out the tool schemas the provider also receives,
    /// and it disagrees with the provider's tokenizer on code and JSON: a
    /// 2.4MB Claude CLI request we estimated at ~660k came back counted as
    /// ~1.03M, over a 1M limit we believed we were at 66% of. Compaction
    /// measured against the estimate fired eight times without ever getting
    /// under the ceiling, because the ceiling was never where we thought.
    ///
    /// Once the provider tells us a real number we anchor on it and track our
    /// own estimate's movement since, which keeps the budget honest between
    /// calls without pretending we can tokenize the way they do. Stored as
    /// (their count, our estimate at that moment) so the delta works in both
    /// directions: trimming and compaction lower it, appending raises it.
    pub provider_anchor: Option<(usize, usize)>,
}

/// A file tracked in the conversation
#[derive(Debug, Clone)]
pub struct TrackedFile {
    pub id: Uuid,
    pub path: PathBuf,
    pub content: Option<String>,
    pub token_count: usize,
}

impl AgentContext {
    /// Create a new agent context for a session
    pub fn new(session_id: Uuid, max_tokens: usize) -> Self {
        Self {
            session_id,
            system_brain: None,
            messages: Vec::new(),
            tracked_files: Vec::new(),
            token_count: 0,
            max_tokens,
            provider_anchor: None,
        }
    }

    /// Set the system brain
    pub fn with_system_brain(mut self, prompt: String) -> Self {
        self.token_count += Self::estimate_tokens(&prompt);
        self.system_brain = Some(prompt);
        self
    }

    /// Add a message to the context
    pub fn add_message(&mut self, message: Message) {
        // Estimate tokens for the message
        let tokens = self.estimate_message_tokens(&message);
        self.token_count += tokens;
        self.messages.push(message);
    }

    /// Convert database messages to LLM messages
    pub fn from_db_messages(
        session_id: Uuid,
        db_messages: Vec<DbMessage>,
        max_tokens: usize,
    ) -> Self {
        let mut context = Self::new(session_id, max_tokens);

        for db_msg in db_messages {
            // Skip messages with empty content AND no captured reasoning —
            // Anthropic rejects empty text blocks. A non-empty thinking
            // column alone still justifies keeping the row so downstream
            // providers (e.g. Moonshot kimi) see the reasoning context.
            let has_content = !db_msg.content.is_empty();
            let has_thinking = db_msg
                .thinking
                .as_deref()
                .is_some_and(|t| !t.trim().is_empty());
            if !has_content && !has_thinking {
                continue;
            }

            let role = match db_msg.role.as_str() {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                "system" => Role::System,
                _ => Role::User, // Default fallback
            };

            // Rehydrate reasoning as a leading ContentBlock::Thinking so
            // the OpenAI-compatible encoder can emit it as
            // `reasoning_content` on assistant tool_call messages.
            // Without this, Moonshot kimi 400s on any resumed turn because
            // the required `reasoning_content` field is missing.
            let mut content: Vec<ContentBlock> = Vec::new();
            if role == Role::Assistant
                && has_thinking
                && let Some(thinking) = db_msg.thinking.as_deref()
            {
                content.push(ContentBlock::Thinking {
                    thinking: thinking.to_string(),
                    signature: None,
                });
            }
            if has_content {
                content.push(ContentBlock::Text {
                    text: db_msg.content,
                });
            }

            let message = Message { role, content };

            context.add_message(message);
        }

        context
    }

    /// Track a file in the conversation
    pub fn track_file(&mut self, file: TrackedFile) {
        self.token_count += file.token_count;
        self.tracked_files.push(file);
    }

    /// Check if context would exceed limit with additional tokens
    pub fn would_exceed_limit(&self, additional_tokens: usize) -> bool {
        self.token_count + additional_tokens > self.max_tokens
    }

    /// Estimate tokens for a message
    pub(crate) fn estimate_message_tokens(&self, message: &Message) -> usize {
        let mut tokens = 0;

        for content in &message.content {
            match content {
                ContentBlock::Text { text } => {
                    tokens += Self::estimate_tokens(text);
                }
                ContentBlock::ToolUse { name, input, .. } => {
                    tokens += Self::estimate_tokens(name);
                    tokens += Self::estimate_tokens(&input.to_string());
                }
                ContentBlock::ToolResult { content, .. } => {
                    tokens += Self::estimate_tokens(content);
                }
                ContentBlock::Image { .. } => {
                    // Images use a fixed token count (approximate)
                    tokens += 1000;
                }
                ContentBlock::Thinking { thinking, .. } => {
                    tokens += Self::estimate_tokens(thinking);
                }
            }
        }

        // Add overhead for message structure
        tokens + 4
    }

    /// Token estimation using tiktoken cl100k_base BPE encoding.
    /// No more chars/N guessing — this gives real token counts.
    pub fn estimate_tokens(text: &str) -> usize {
        tokenizer::count_tokens(text)
    }

    /// Static version of estimate_message_tokens — usable without a &self reference.
    pub fn estimate_tokens_static(message: &Message) -> usize {
        let mut tokens = 0;
        for content in &message.content {
            match content {
                ContentBlock::Text { text } => {
                    tokens += Self::estimate_tokens(text);
                }
                ContentBlock::ToolUse { name, input, .. } => {
                    tokens += Self::estimate_tokens(name);
                    tokens += Self::estimate_tokens(&input.to_string());
                }
                ContentBlock::ToolResult { content, .. } => {
                    tokens += Self::estimate_tokens(content);
                }
                ContentBlock::Image { .. } => {
                    tokens += 1000;
                }
                ContentBlock::Thinking { thinking, .. } => {
                    tokens += Self::estimate_tokens(thinking);
                }
            }
        }
        tokens + 4
    }

    /// The size of the next request, preferring the provider's count.
    ///
    /// Falls back to the local estimate until a provider has reported one, so
    /// a first turn behaves exactly as before.
    pub fn effective_token_count(&self) -> usize {
        match self.provider_anchor {
            // Their count, moved by however much our own estimate has shifted
            // since we took it. Every path that trims or appends already
            // maintains `token_count`, so none of them need to know about the
            // anchor for the budget to follow along.
            Some((reported, estimated_at_anchor)) => reported
                .saturating_add(self.token_count)
                .saturating_sub(estimated_at_anchor),
            None => self.token_count,
        }
    }

    /// Anchor the budget on a size the provider actually reported.
    ///
    /// Callers must reject implausible reports first: an endpoint that adds a
    /// flat overhead to every call would otherwise drag the budget up and
    /// compact a context that never needed it.
    pub fn record_provider_reported_tokens(&mut self, reported: usize) {
        self.provider_anchor = Some((reported, self.token_count));
    }

    /// Get the current token usage percentage
    pub fn usage_percentage(&self) -> f64 {
        (self.effective_token_count() as f64 / self.max_tokens as f64) * 100.0
    }

    /// Returns true if a message consists entirely of ToolResult blocks.
    /// Such a message is "orphaned" if the preceding assistant(ToolUse) message
    /// was removed, and will cause the API to reject the conversation.
    pub(crate) fn is_orphaned_tool_result_msg(msg: &Message) -> bool {
        msg.role == Role::User
            && !msg.content.is_empty()
            && msg
                .content
                .iter()
                .all(|b| matches!(b, ContentBlock::ToolResult { .. }))
    }

    /// Remove any leading user messages that consist solely of ToolResult blocks.
    /// Called after trimming to prevent orphaned tool results at the start of history.
    fn drop_leading_orphan_tool_results(&mut self) {
        while self
            .messages
            .first()
            .is_some_and(Self::is_orphaned_tool_result_msg)
        {
            let tokens = self.estimate_message_tokens(&self.messages[0]);
            self.token_count = self.token_count.saturating_sub(tokens);
            self.messages.remove(0);
        }
    }

    /// Trim old messages if context is too large
    pub fn trim_to_fit(&mut self, required_space: usize) {
        while self.would_exceed_limit(required_space) && !self.messages.is_empty() {
            // Remove the oldest user/assistant message pair
            if let Some(first_msg) = self.messages.first() {
                let tokens = self.estimate_message_tokens(first_msg);
                self.token_count = self.token_count.saturating_sub(tokens);
                self.messages.remove(0);
            }
        }
        // Removing an assistant(tool_use) exposes an orphaned user(tool_result) — drop it
        self.drop_leading_orphan_tool_results();
    }

    /// Hard-truncate old messages until token count is at or below `target_tokens`.
    /// Keeps at least 2 messages (the most recent pair) to maintain conversation validity.
    pub fn hard_truncate_to(&mut self, target_tokens: usize) {
        while self.token_count > target_tokens && self.messages.len() > 2 {
            let tokens = self.estimate_message_tokens(&self.messages[0]);
            self.token_count = self.token_count.saturating_sub(tokens);
            self.messages.remove(0);
        }
        self.drop_leading_orphan_tool_results();
    }

    /// Compact the context by replacing old messages with a summary.
    ///
    /// Keeps the most recent messages that fit within the token budget
    /// and prepends a summary of everything that was trimmed.
    /// `keep_token_budget` is the max tokens for kept messages (excluding the summary).
    pub fn compact_with_summary(&mut self, summary: String, keep_token_budget: usize) {
        // Walk backwards from end, keeping messages until we hit the budget
        let summary_tokens = Self::estimate_tokens(&summary) + 50; // +50 for the marker text
        let available = keep_token_budget.saturating_sub(summary_tokens);
        let mut running = 0usize;
        let mut keep_count = 0usize;
        for msg in self.messages.iter().rev() {
            let t = self.estimate_message_tokens(msg);
            if running + t > available {
                break;
            }
            running += t;
            keep_count += 1;
        }
        // Caller can request a clean compaction (only the summary survives)
        // by passing `keep_token_budget == 0` — in that case we honour zero
        // kept messages. Otherwise keep at least the most recent pair so
        // valid API request structure is preserved.
        if keep_token_budget > 0 {
            keep_count = keep_count.max(2.min(self.messages.len()));
        }
        let mut keep_start = self.messages.len().saturating_sub(keep_count);

        // Advance past any leading orphaned tool_result messages in the kept slice.
        // If the assistant(tool_use) that precedes them is being dropped, they'd be invalid.
        while keep_start < self.messages.len()
            && Self::is_orphaned_tool_result_msg(&self.messages[keep_start])
        {
            keep_start += 1;
        }

        let kept_messages: Vec<Message> = self.messages.drain(keep_start..).collect();

        // Clear all old messages
        self.messages.clear();

        // Prepend the compaction summary as a user message (so the LLM sees the context)
        let summary_msg = Message {
            role: Role::User,
            content: vec![ContentBlock::Text {
                text: format!(
                    "[CONTEXT COMPACTION — The conversation was automatically compacted. \
                     Below is a structured summary of everything before this point.]\n\n{}",
                    summary
                ),
            }],
        };
        self.messages.push(summary_msg);

        // Re-add kept messages
        self.messages.extend(kept_messages);

        // Recalculate token count
        self.token_count = 0;
        if let Some(brain) = &self.system_brain {
            self.token_count += Self::estimate_tokens(brain);
        }
        for msg in &self.messages {
            self.token_count += self.estimate_message_tokens(msg);
        }

        // Drop the pre-compaction provider anchor: the anchor was taken against
        // the uncompacted prompt and would inflate the post-compaction budget
        // with a stale delta (#211).
        self.provider_anchor = None;
    }
}
