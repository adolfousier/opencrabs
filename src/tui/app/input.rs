//! Input handling — keyboard events, history, and approval interception.

use super::events::{AppMode, ToolApprovalResponse, TuiEvent};
use super::*;
use anyhow::Result;
use tokio::sync::mpsc;
use uuid::Uuid;

impl App {
    /// Delete the word before the cursor (for Ctrl+Backspace and Alt+Backspace)
    pub(crate) fn delete_last_word(&mut self) {
        if self.cursor_position == 0 {
            return;
        }
        let before = &self.input_buffer[..self.cursor_position];
        // Skip trailing whitespace
        let trimmed = before.trim_end();
        // Find the last whitespace boundary in the trimmed portion
        let word_start = trimmed
            .rfind(char::is_whitespace)
            .map(|pos| pos + 1)
            .unwrap_or(0);
        // Remove from word_start to cursor_position
        self.input_buffer.drain(word_start..self.cursor_position);
        self.cursor_position = word_start;
    }

    /// History file path: ~/.opencrabs/history.txt
    fn history_path() -> Option<std::path::PathBuf> {
        Some(crate::config::opencrabs_home().join("history.txt"))
    }

    /// Load input history from disk (one entry per line, most recent last)
    pub(crate) fn load_history() -> Vec<String> {
        let Some(path) = Self::history_path() else {
            return Vec::new();
        };
        match std::fs::read_to_string(&path) {
            Ok(content) => content
                .lines()
                .filter(|l| !l.is_empty())
                .map(|l| l.to_string())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Append a single entry to the history file (and trim to 500 entries)
    fn save_history_entry(&self, entry: &str) {
        let Some(path) = Self::history_path() else {
            return;
        };
        // Ensure directory exists
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Cap at 500 entries: keep last 499 + new entry
        let max_entries = 500;
        if self.input_history.len() > max_entries {
            // Rewrite the whole file with only the last max_entries
            let start = self.input_history.len().saturating_sub(max_entries);
            let trimmed: Vec<&str> = self.input_history[start..]
                .iter()
                .map(|s| s.as_str())
                .collect();
            let _ = std::fs::write(&path, trimmed.join("\n") + "\n");
        } else {
            // Just append
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                let _ = writeln!(f, "{}", entry);
            }
        }
    }

    pub fn has_pending_approval(&self) -> bool {
        self.messages.iter().rev().any(|msg| {
            msg.approval
                .as_ref()
                .is_some_and(|a| a.state == ApprovalState::Pending)
        })
    }

    pub fn has_pending_plan_approval(&self) -> bool {
        self.messages.iter().rev().any(|msg| {
            msg.plan_approval
                .as_ref()
                .is_some_and(|p| p.state == PlanApprovalState::Pending)
        })
    }

    pub(crate) fn has_pending_approve_menu(&self) -> bool {
        self.messages.iter().rev().any(|msg| {
            msg.approve_menu
                .as_ref()
                .is_some_and(|m| m.state == ApproveMenuState::Pending)
        })
    }

    /// Handle keys in chat mode
    pub(crate) async fn handle_chat_key(
        &mut self,
        event: crossterm::event::KeyEvent,
    ) -> Result<()> {
        use super::events::keys;
        use crossterm::event::{KeyCode, KeyModifiers};

        // Intercept keys when /approve menu is pending
        if self.has_pending_approve_menu() {
            if keys::is_up(&event) {
                if let Some(menu) = self
                    .messages
                    .iter_mut()
                    .rev()
                    .find_map(|m| m.approve_menu.as_mut())
                    .filter(|m| m.state == ApproveMenuState::Pending)
                {
                    menu.selected_option = menu.selected_option.saturating_sub(1);
                }
                return Ok(());
            } else if keys::is_down(&event) {
                if let Some(menu) = self
                    .messages
                    .iter_mut()
                    .rev()
                    .find_map(|m| m.approve_menu.as_mut())
                    .filter(|m| m.state == ApproveMenuState::Pending)
                {
                    menu.selected_option = (menu.selected_option + 1).min(2);
                }
                return Ok(());
            } else if keys::is_enter(&event) || keys::is_submit(&event) {
                let selected = self
                    .messages
                    .iter()
                    .rev()
                    .find_map(|m| m.approve_menu.as_ref())
                    .filter(|m| m.state == ApproveMenuState::Pending)
                    .map(|m| m.selected_option)
                    .unwrap_or(0);

                // Apply policy
                match selected {
                    0 => {
                        // Reset to approve-only
                        self.approval_auto_session = false;
                        self.approval_auto_always = false;
                    }
                    1 => {
                        // Allow all for this session
                        self.approval_auto_session = true;
                        self.approval_auto_always = false;
                    }
                    _ => {
                        // Yolo mode
                        self.approval_auto_session = false;
                        self.approval_auto_always = true;
                    }
                }

                let label = match selected {
                    0 => "Approve-only (always ask)",
                    1 => "Allow all for this session",
                    _ => "Yolo mode (execute without approval)",
                };

                // Mark menu as resolved
                if let Some(menu) = self
                    .messages
                    .iter_mut()
                    .rev()
                    .find_map(|m| m.approve_menu.as_mut())
                    .filter(|m| m.state == ApproveMenuState::Pending)
                {
                    menu.state = ApproveMenuState::Selected(selected);
                }

                // Persist to config.toml
                let policy_str = match selected {
                    0 => "ask",
                    1 => "auto-session",
                    _ => "auto-always",
                };
                if let Err(e) =
                    crate::config::Config::write_key("agent", "approval_policy", policy_str)
                {
                    tracing::warn!("Failed to persist approval policy: {}", e);
                }

                self.push_system_message(format!("Approval policy set to: {}", label));
                return Ok(());
            } else if keys::is_cancel(&event) {
                // Cancel — dismiss menu without changing policy
                if let Some(menu) = self
                    .messages
                    .iter_mut()
                    .rev()
                    .find_map(|m| m.approve_menu.as_mut())
                    .filter(|m| m.state == ApproveMenuState::Pending)
                {
                    menu.state = ApproveMenuState::Selected(99); // sentinel for cancelled
                }
                self.push_system_message("Approval policy unchanged.".to_string());
                return Ok(());
            }
            return Ok(());
        }

        // Intercept keys when an inline approval is pending
        // Options: Yes(0), Always(1), No(2)
        if self.has_pending_approval() {
            if keys::is_left(&event) || keys::is_up(&event) {
                // Navigate options left
                if let Some(approval) = self
                    .messages
                    .iter_mut()
                    .rev()
                    .find_map(|m| m.approval.as_mut())
                    .filter(|a| a.state == ApprovalState::Pending)
                {
                    approval.selected_option = approval.selected_option.saturating_sub(1);
                }
                return Ok(());
            } else if keys::is_right(&event) || keys::is_down(&event) {
                // Navigate options right
                if let Some(approval) = self
                    .messages
                    .iter_mut()
                    .rev()
                    .find_map(|m| m.approval.as_mut())
                    .filter(|a| a.state == ApprovalState::Pending)
                {
                    approval.selected_option = (approval.selected_option + 1).min(2);
                }
                return Ok(());
            } else if keys::is_enter(&event) || keys::is_submit(&event) {
                // Confirm: Yes(0)=approve once, Always(1)=approve always, No(2)=deny
                let approval_data: Option<(
                    Uuid,
                    usize,
                    mpsc::UnboundedSender<ToolApprovalResponse>,
                )> = self
                    .messages
                    .iter()
                    .rev()
                    .find_map(|m| m.approval.as_ref())
                    .filter(|a| a.state == ApprovalState::Pending)
                    .map(|a| (a.request_id, a.selected_option, a.response_tx.clone()));

                if let Some((request_id, selected, response_tx)) = approval_data {
                    if selected == 2 {
                        // "No" — deny
                        let response = ToolApprovalResponse {
                            request_id,
                            approved: false,
                            reason: Some("User denied permission".to_string()),
                        };
                        if let Err(e) = response_tx.send(response.clone()) {
                            tracing::error!(
                                "Failed to send denial response back to agent: {:?}",
                                e
                            );
                        }
                        let _ = self
                            .event_sender()
                            .send(TuiEvent::ToolApprovalResponse(response));
                    } else {
                        // "Yes" (0) or "Always" (1)
                        let option = if selected == 1 {
                            ApprovalOption::AllowAlways
                        } else {
                            ApprovalOption::AllowOnce
                        };
                        if matches!(option, ApprovalOption::AllowAlways) {
                            self.approval_auto_session = true;
                            self.push_system_message(
                                "Auto-approve enabled for this session. Use /approve to reset."
                                    .to_string(),
                            );
                        }
                        let response = ToolApprovalResponse {
                            request_id,
                            approved: true,
                            reason: None,
                        };
                        if let Err(e) = response_tx.send(response.clone()) {
                            tracing::error!(
                                "Failed to send approval response back to agent: {:?}",
                                e
                            );
                        }
                        let _ = self
                            .event_sender()
                            .send(TuiEvent::ToolApprovalResponse(response));
                    }
                    // Remove resolved approval messages to prevent channel accumulation
                    self.messages.retain(|m| {
                        m.approval
                            .as_ref()
                            .is_none_or(|a| a.request_id != request_id)
                    });
                }
                return Ok(());
            } else if keys::is_deny(&event) || keys::is_cancel(&event) {
                // D/Esc shortcut — deny directly
                let approval_data: Option<(Uuid, mpsc::UnboundedSender<ToolApprovalResponse>)> =
                    self.messages
                        .iter()
                        .rev()
                        .find_map(|m| m.approval.as_ref())
                        .filter(|a| a.state == ApprovalState::Pending)
                        .map(|a| (a.request_id, a.response_tx.clone()));

                if let Some((request_id, response_tx)) = approval_data {
                    let response = ToolApprovalResponse {
                        request_id,
                        approved: false,
                        reason: Some("User denied permission".to_string()),
                    };
                    if let Err(e) = response_tx.send(response.clone()) {
                        tracing::error!("Failed to send denial response back to agent: {:?}", e);
                    }
                    let _ = self
                        .event_sender()
                        .send(TuiEvent::ToolApprovalResponse(response));
                    // Remove resolved approval message
                    self.messages.retain(|m| {
                        m.approval
                            .as_ref()
                            .is_none_or(|a| a.request_id != request_id)
                    });
                }
                return Ok(());
            } else if keys::is_view_details(&event) {
                // V key — toggle details
                if let Some(approval) = self
                    .messages
                    .iter_mut()
                    .rev()
                    .find_map(|m| m.approval.as_mut())
                    .filter(|a| a.state == ApprovalState::Pending)
                {
                    approval.show_details = !approval.show_details;
                }
                return Ok(());
            } else if event.code == KeyCode::Char('o') && event.modifiers == KeyModifiers::CONTROL {
                // Allow Ctrl+O during approval so user can collapse tool groups to see the approval
                let target = if let Some(ref group) = self.active_tool_group {
                    !group.expanded
                } else if let Some(msg) =
                    self.messages.iter().rev().find(|m| m.tool_group.is_some())
                {
                    !msg.tool_group.as_ref().expect("checked").expanded
                } else {
                    true
                };
                if let Some(ref mut group) = self.active_tool_group {
                    group.expanded = target;
                }
                for msg in self.messages.iter_mut() {
                    if let Some(ref mut group) = msg.tool_group {
                        group.expanded = target;
                    }
                }
                return Ok(());
            }
            // Other keys ignored while approval pending
            return Ok(());
        }

        // Intercept keys when an inline plan approval is pending
        // Options: Approve(0), Reject(1), Request Changes(2), View Plan(3)
        if self.has_pending_plan_approval() {
            if keys::is_left(&event) || keys::is_up(&event) {
                if let Some(pa) = self
                    .messages
                    .iter_mut()
                    .rev()
                    .find_map(|m| m.plan_approval.as_mut())
                    .filter(|p| p.state == PlanApprovalState::Pending)
                {
                    pa.selected_option = pa.selected_option.saturating_sub(1);
                }
                return Ok(());
            } else if keys::is_right(&event) || keys::is_down(&event) {
                if let Some(pa) = self
                    .messages
                    .iter_mut()
                    .rev()
                    .find_map(|m| m.plan_approval.as_mut())
                    .filter(|p| p.state == PlanApprovalState::Pending)
                {
                    pa.selected_option = (pa.selected_option + 1).min(3);
                }
                return Ok(());
            } else if keys::is_enter(&event) || keys::is_submit(&event) {
                let selected = self
                    .messages
                    .iter()
                    .rev()
                    .find_map(|m| m.plan_approval.as_ref())
                    .filter(|p| p.state == PlanApprovalState::Pending)
                    .map(|p| p.selected_option);

                if let Some(selected) = selected {
                    match selected {
                        0 => {
                            // Approve — same as Ctrl+A
                            if let Some(pa) = self
                                .messages
                                .iter_mut()
                                .rev()
                                .find_map(|m| m.plan_approval.as_mut())
                                .filter(|p| p.state == PlanApprovalState::Pending)
                            {
                                pa.state = PlanApprovalState::Approved;
                            }
                            if let Some(plan) = &mut self.current_plan {
                                plan.approve();
                                plan.start_execution();
                                self.export_plan_to_markdown("PLAN.md").await?;
                                self.save_plan().await?;
                                self.execute_plan_tasks().await?;
                            }
                        }
                        1 => {
                            // Reject — same as Ctrl+R
                            if let Some(pa) = self
                                .messages
                                .iter_mut()
                                .rev()
                                .find_map(|m| m.plan_approval.as_mut())
                                .filter(|p| p.state == PlanApprovalState::Pending)
                            {
                                pa.state = PlanApprovalState::Rejected;
                            }
                            if let Some(plan) = &mut self.current_plan {
                                plan.reject();
                                self.save_plan().await?;
                            }
                            self.current_plan = None;
                        }
                        2 => {
                            // Request changes — same as Ctrl+I
                            if let Some(pa) = self
                                .messages
                                .iter_mut()
                                .rev()
                                .find_map(|m| m.plan_approval.as_mut())
                                .filter(|p| p.state == PlanApprovalState::Pending)
                            {
                                pa.state = PlanApprovalState::RevisionRequested;
                            }
                            if let Some(plan) = &self.current_plan {
                                let plan_summary = format!(
                                    "Current plan '{}' has {} tasks:\n{}",
                                    plan.title,
                                    plan.tasks.len(),
                                    plan.tasks
                                        .iter()
                                        .enumerate()
                                        .map(|(i, t)| format!(
                                            "  {}. {} ({})",
                                            i + 1,
                                            t.title,
                                            t.task_type
                                        ))
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                );
                                self.input_buffer = format!(
                                    "Please revise this plan:\n\n{}\n\nRequested changes: ",
                                    plan_summary
                                );
                                self.cursor_position = self.input_buffer.len();
                            }
                        }
                        3 => {
                            // View plan — switch to Plan Mode
                            self.load_plan_for_viewing().await?;
                            if self.current_plan.is_some() {
                                self.switch_mode(AppMode::Plan).await?;
                            }
                        }
                        _ => {}
                    }
                }
                return Ok(());
            } else if keys::is_view_details(&event) {
                // V key — toggle task list
                if let Some(pa) = self
                    .messages
                    .iter_mut()
                    .rev()
                    .find_map(|m| m.plan_approval.as_mut())
                    .filter(|p| p.state == PlanApprovalState::Pending)
                {
                    pa.show_details = !pa.show_details;
                }
                return Ok(());
            }
            // Other keys fall through — plan approval is non-blocking, user can still type
        }

        // When slash suggestions are active, intercept navigation keys
        if self.slash_suggestions_active {
            if keys::is_up(&event) {
                self.slash_selected_index = self.slash_selected_index.saturating_sub(1);
                return Ok(());
            } else if keys::is_down(&event) {
                if !self.slash_filtered.is_empty() {
                    self.slash_selected_index =
                        (self.slash_selected_index + 1).min(self.slash_filtered.len() - 1);
                }
                return Ok(());
            } else if keys::is_enter(&event) || keys::is_submit(&event) {
                // Select the highlighted command and execute it
                if let Some(&cmd_idx) = self.slash_filtered.get(self.slash_selected_index) {
                    let cmd_name = self.slash_command_name(cmd_idx).unwrap_or("").to_string();
                    self.input_buffer.clear();
                    self.cursor_position = 0;
                    self.slash_suggestions_active = false;
                    self.handle_slash_command(&cmd_name).await;
                }
                return Ok(());
            } else if keys::is_cancel(&event) {
                // Dismiss dropdown but keep input
                self.slash_suggestions_active = false;
                return Ok(());
            }
            // Other keys fall through to normal handling
        }

        // Any key other than Escape resets escape confirmation
        if !keys::is_cancel(&event) {
            self.escape_pending_at = None;
        }

        if keys::is_newline(&event) {
            // Alt+Enter or Shift+Enter = insert newline for multi-line input
            self.input_buffer.insert(self.cursor_position, '\n');
            self.cursor_position += 1;
        } else if keys::is_submit(&event)
            && (!self.input_buffer.trim().is_empty() || !self.attachments.is_empty())
        {
            // Check for slash commands before sending to LLM
            let content = self.input_buffer.clone();
            if self.handle_slash_command(content.trim()).await {
                self.input_buffer.clear();
                self.cursor_position = 0;
                self.slash_suggestions_active = false;
                return Ok(());
            }

            // Also scan typed input for image paths at submit time
            let (clean_text, typed_attachments) = Self::extract_image_paths(&content);
            let mut all_attachments = std::mem::take(&mut self.attachments);
            all_attachments.extend(typed_attachments);

            let final_content =
                if !all_attachments.is_empty() && clean_text.trim() != content.trim() {
                    clean_text
                } else {
                    content.clone()
                };

            // Enter = send message
            // Save to input history (dedup consecutive) and persist to disk
            let trimmed = content.trim().to_string();
            if self.input_history.last() != Some(&trimmed) {
                self.input_history.push(trimmed.clone());
                self.save_history_entry(&trimmed);
            }
            self.input_history_index = None;
            self.input_history_stash.clear();

            self.input_buffer.clear();
            self.cursor_position = 0;
            self.attachments.clear();
            self.slash_suggestions_active = false;

            // Build message content with attachment markers for the agent.
            // Format: <<IMG:/path/to/file.png>> — handles spaces in paths.
            let send_content = if all_attachments.is_empty() {
                final_content
            } else {
                let mut msg = final_content.clone();
                for att in &all_attachments {
                    msg.push_str(&format!(" <<IMG:{}>>", att.path));
                }
                msg
            };
            self.send_message(send_content).await?;
        } else if keys::is_cancel(&event) {
            // When processing, double-Escape aborts the operation
            if self.is_processing {
                if let Some(pending_at) = self.escape_pending_at {
                    if pending_at.elapsed() < std::time::Duration::from_secs(3) {
                        // Second Escape within 3 seconds — abort
                        if let Some(token) = &self.cancel_token {
                            token.cancel();
                        }
                        self.is_processing = false;
                        self.processing_started_at = None;
                        // Preserve partial streaming response as a message before clearing
                        if let Some(text) = self.streaming_response.take()
                            && !text.trim().is_empty()
                        {
                            self.messages.push(DisplayMessage {
                                id: Uuid::new_v4(),
                                role: "assistant".to_string(),
                                content: text,
                                timestamp: chrono::Utc::now(),
                                token_count: None,
                                cost: None,
                                approval: None,
                                approve_menu: None,
                                details: None,
                                expanded: false,
                                tool_group: None,
                                plan_approval: None,
                            });
                        }
                        self.streaming_reasoning = None;
                        self.cancel_token = None;
                        self.escape_pending_at = None;
                        // Deny any pending approvals so agent callbacks don't hang
                        for msg in &mut self.messages {
                            if let Some(ref mut approval) = msg.approval
                                && approval.state == ApprovalState::Pending
                            {
                                let _ = approval.response_tx.send(ToolApprovalResponse {
                                    request_id: approval.request_id,
                                    approved: false,
                                    reason: Some("Operation cancelled".to_string()),
                                });
                                approval.state =
                                    ApprovalState::Denied("Operation cancelled".to_string());
                            }
                        }
                        // Finalize any active tool group
                        if let Some(group) = self.active_tool_group.take() {
                            let count = group.calls.len();
                            self.messages.push(DisplayMessage {
                                id: Uuid::new_v4(),
                                role: "tool_group".to_string(),
                                content: format!(
                                    "{} tool call{}",
                                    count,
                                    if count == 1 { "" } else { "s" }
                                ),
                                timestamp: chrono::Utc::now(),
                                token_count: None,
                                cost: None,
                                approval: None,
                                approve_menu: None,
                                details: None,
                                expanded: false,
                                tool_group: Some(group),
                                plan_approval: None,
                            });
                        }
                        self.push_system_message("Operation cancelled.".to_string());
                    } else {
                        self.escape_pending_at = Some(std::time::Instant::now());
                        self.error_message = Some("Press Esc again to abort".to_string());
                    }
                } else {
                    self.escape_pending_at = Some(std::time::Instant::now());
                    self.error_message = Some("Press Esc again to abort".to_string());
                }
            } else if !self.auto_scroll {
                // User is scrolled up — scroll to bottom first
                self.scroll_offset = 0;
                self.auto_scroll = true;
                self.error_message = None;
                self.escape_pending_at = None;
            } else if self.input_buffer.is_empty() {
                // Nothing to clear, just dismiss error
                self.error_message = None;
                self.escape_pending_at = None;
            } else if let Some(pending_at) = self.escape_pending_at {
                if pending_at.elapsed() < std::time::Duration::from_secs(3) {
                    // Second Escape within 3 seconds — stash input then clear
                    // Arrow Up will recover the stashed text
                    if !self.input_buffer.is_empty() {
                        self.input_history_stash = self.input_buffer.clone();
                    }
                    self.input_buffer.clear();
                    self.cursor_position = 0;
                    self.attachments.clear();
                    self.error_message = None;
                    self.escape_pending_at = None;
                    self.slash_suggestions_active = false;
                } else {
                    // Expired — treat as first Escape again
                    self.escape_pending_at = Some(std::time::Instant::now());
                    self.error_message = Some("Press Esc again to clear input".to_string());
                }
            } else {
                // First Escape — show confirmation hint
                self.escape_pending_at = Some(std::time::Instant::now());
                self.error_message = Some("Press Esc again to clear input".to_string());
            }
        } else if event.code == KeyCode::Char('o') && event.modifiers == KeyModifiers::CONTROL {
            if self.hidden_older_messages > 0 && self.display_token_count < 300_000 {
                // Load more history from DB
                self.load_more_history().await?;
            } else {
                // Ctrl+O — toggle expand/collapse on ALL tool groups in the session
                // Determine target state from the active group or most recent group
                let target = if let Some(ref group) = self.active_tool_group {
                    !group.expanded
                } else if let Some(msg) =
                    self.messages.iter().rev().find(|m| m.tool_group.is_some())
                {
                    !msg.tool_group
                        .as_ref()
                        .expect("tool_group checked is_some above")
                        .expanded
                } else {
                    true
                };
                if let Some(ref mut group) = self.active_tool_group {
                    group.expanded = target;
                }
                for msg in self.messages.iter_mut() {
                    if let Some(ref mut group) = msg.tool_group {
                        group.expanded = target;
                    }
                    // Also toggle expanded on messages with reasoning details
                    if msg.details.is_some() {
                        msg.expanded = target;
                    }
                }
            }
        } else if keys::is_page_up(&event) {
            self.scroll_offset = self.scroll_offset.saturating_add(10);
            self.auto_scroll = false;
        } else if keys::is_page_down(&event) {
            self.scroll_offset = self.scroll_offset.saturating_sub(10);
            if self.scroll_offset == 0 {
                self.auto_scroll = true;
            }
        } else if event.code == KeyCode::Backspace && event.modifiers.contains(KeyModifiers::ALT) {
            // Alt+Backspace — delete last word
            self.delete_last_word();
        } else if keys::is_up(&event)
            && !self.slash_suggestions_active
            && self.input_buffer.is_empty()
            && !self.input_history_stash.is_empty()
            && self.input_history_index.is_none()
        {
            // Arrow Up on empty input — restore stashed text first (cleared via Esc)
            self.input_buffer = std::mem::take(&mut self.input_history_stash);
            self.cursor_position = self.input_buffer.len();
        } else if keys::is_up(&event)
            && !self.slash_suggestions_active
            && !self.input_history.is_empty()
        {
            // Arrow Up — browse input history (older)
            match self.input_history_index {
                None => {
                    // Entering history — stash current input
                    self.input_history_stash = self.input_buffer.clone();
                    let idx = self.input_history.len() - 1;
                    self.input_history_index = Some(idx);
                    self.input_buffer = self.input_history[idx].clone();
                    self.cursor_position = self.input_buffer.len();
                }
                Some(idx) if idx > 0 => {
                    let idx = idx - 1;
                    self.input_history_index = Some(idx);
                    self.input_buffer = self.input_history[idx].clone();
                    self.cursor_position = self.input_buffer.len();
                }
                _ => {} // already at oldest
            }
        } else if keys::is_down(&event)
            && !self.slash_suggestions_active
            && self.input_history_index.is_some()
        {
            // Arrow Down — browse input history (newer)
            let idx = self.input_history_index.expect("checked is_some");
            if idx + 1 < self.input_history.len() {
                let idx = idx + 1;
                self.input_history_index = Some(idx);
                self.input_buffer = self.input_history[idx].clone();
                self.cursor_position = self.input_buffer.len();
            } else {
                // Past newest — restore stashed input
                self.input_history_index = None;
                self.input_buffer = std::mem::take(&mut self.input_history_stash);
                self.cursor_position = self.input_buffer.len();
            }
        } else {
            // Regular character input
            match event.code {
                KeyCode::Char('@') => {
                    self.open_file_picker().await?;
                }
                KeyCode::Char(c)
                    if event.modifiers.is_empty() || event.modifiers == KeyModifiers::SHIFT =>
                {
                    self.input_buffer.insert(self.cursor_position, c);
                    self.cursor_position += c.len_utf8();
                }
                KeyCode::Backspace if event.modifiers.is_empty() => {
                    if self.cursor_position > 0 {
                        // Find the previous char boundary
                        let prev = self.input_buffer[..self.cursor_position]
                            .char_indices()
                            .last()
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                        self.input_buffer.remove(prev);
                        self.cursor_position = prev;
                    }
                }
                KeyCode::Delete if event.modifiers.is_empty() => {
                    if self.cursor_position < self.input_buffer.len() {
                        self.input_buffer.remove(self.cursor_position);
                    }
                }
                KeyCode::Left if event.modifiers.is_empty() => {
                    // Move cursor left one character
                    if self.cursor_position > 0 {
                        let prev = self.input_buffer[..self.cursor_position]
                            .char_indices()
                            .last()
                            .map(|(i, _)| i)
                            .unwrap_or(0);
                        self.cursor_position = prev;
                    }
                }
                KeyCode::Right if event.modifiers.is_empty() => {
                    // Move cursor right one character
                    if self.cursor_position < self.input_buffer.len() {
                        let next = self.input_buffer[self.cursor_position..]
                            .char_indices()
                            .nth(1)
                            .map(|(i, _)| self.cursor_position + i)
                            .unwrap_or(self.input_buffer.len());
                        self.cursor_position = next;
                    }
                }
                KeyCode::Home => {
                    self.cursor_position = 0;
                }
                KeyCode::End => {
                    self.cursor_position = self.input_buffer.len();
                }
                KeyCode::Enter => {
                    // Fallback — if Enter didn't match is_submit (e.g., empty input)
                    // do nothing
                }
                _ => {}
            }
        }

        // Update slash autocomplete after any keystroke that modifies input
        self.update_slash_suggestions();

        Ok(())
    }

    /// Handle keys in sessions mode
    pub(crate) async fn handle_sessions_key(
        &mut self,
        event: crossterm::event::KeyEvent,
    ) -> Result<()> {
        use super::events::keys;
        use crossterm::event::KeyCode;

        // Rename mode: typing the new name
        if self.session_renaming {
            match event.code {
                KeyCode::Enter => {
                    // Save the new name
                    if let Some(session) = self.sessions.get(self.selected_session_index) {
                        let new_title = if self.session_rename_buffer.trim().is_empty() {
                            None
                        } else {
                            Some(self.session_rename_buffer.trim().to_string())
                        };
                        let session_id = session.id;
                        self.session_service
                            .update_session_title(session_id, new_title)
                            .await?;
                        // Update current session if it's the one being renamed
                        if let Some(ref mut current) = self.current_session
                            && current.id == session_id
                        {
                            current.title = if self.session_rename_buffer.trim().is_empty() {
                                None
                            } else {
                                Some(self.session_rename_buffer.trim().to_string())
                            };
                        }
                        self.load_sessions().await?;
                    }
                    self.session_renaming = false;
                    self.session_rename_buffer.clear();
                }
                KeyCode::Esc => {
                    self.session_renaming = false;
                    self.session_rename_buffer.clear();
                }
                KeyCode::Backspace => {
                    self.session_rename_buffer.pop();
                }
                KeyCode::Char(c) => {
                    self.session_rename_buffer.push(c);
                }
                _ => {}
            }
            return Ok(());
        }

        // Normal sessions mode
        if keys::is_cancel(&event) {
            self.switch_mode(AppMode::Chat).await?;
        } else if keys::is_up(&event) {
            self.selected_session_index = self.selected_session_index.saturating_sub(1);
        } else if keys::is_down(&event) {
            self.selected_session_index =
                (self.selected_session_index + 1).min(self.sessions.len().saturating_sub(1));
        } else if keys::is_enter(&event) {
            if let Some(session) = self.sessions.get(self.selected_session_index) {
                self.load_session(session.id).await?;
                self.switch_mode(AppMode::Chat).await?;
            }
        } else if event.code == KeyCode::Char('r') || event.code == KeyCode::Char('R') {
            // Start renaming the selected session
            if let Some(session) = self.sessions.get(self.selected_session_index) {
                self.session_renaming = true;
                self.session_rename_buffer = session.title.clone().unwrap_or_default();
            }
        } else if event.code == KeyCode::Char('n') || event.code == KeyCode::Char('N') {
            // Create a new session and switch to it
            self.create_new_session().await?;
            self.switch_mode(AppMode::Chat).await?;
        } else if event.code == KeyCode::Char('d') || event.code == KeyCode::Char('D') {
            // Delete the selected session
            if let Some(session) = self.sessions.get(self.selected_session_index) {
                let session_id = session.id;
                let is_current = self
                    .current_session
                    .as_ref()
                    .map(|s| s.id == session_id)
                    .unwrap_or(false);
                self.session_service.delete_session(session_id).await?;
                if is_current {
                    self.current_session = None;
                    self.messages.clear();
                    *self.shared_session_id.lock().await = None;
                }
                self.load_sessions().await?;
                // Adjust index if it's now out of bounds
                if self.selected_session_index >= self.sessions.len() {
                    self.selected_session_index = self.sessions.len().saturating_sub(1);
                }
            }
        }

        Ok(())
    }

    /// Handle keys in plan mode
    pub(crate) async fn handle_plan_key(
        &mut self,
        event: crossterm::event::KeyEvent,
    ) -> Result<()> {
        use super::events::keys;
        use crossterm::event::{KeyCode, KeyModifiers};

        // Cancel/Escape - return to chat
        if keys::is_cancel(&event) {
            self.switch_mode(AppMode::Chat).await?;
            return Ok(());
        }

        // Ctrl+A - Approve plan
        if event.code == KeyCode::Char('a') && event.modifiers.contains(KeyModifiers::CONTROL) {
            tracing::info!("✅ Ctrl+A pressed - Approving plan");
            if let Some(plan) = &mut self.current_plan {
                plan.approve();
                plan.start_execution();

                // Export plan to markdown file
                self.export_plan_to_markdown("PLAN.md").await?;

                // Save plan to file
                self.save_plan().await?;
                self.switch_mode(AppMode::Chat).await?;
                // Start executing tasks sequentially
                self.execute_plan_tasks().await?;
            }
            return Ok(());
        }

        // Ctrl+R - Reject plan
        if event.code == KeyCode::Char('r') && event.modifiers.contains(KeyModifiers::CONTROL) {
            tracing::info!("❌ Ctrl+R pressed - Rejecting plan");
            if let Some(plan) = &mut self.current_plan {
                plan.reject();
                // Save plan to file
                self.save_plan().await?;
                // Clear the plan from memory and return to chat
                self.current_plan = None;
                self.switch_mode(AppMode::Chat).await?;
            }
            return Ok(());
        }

        // Ctrl+I - Request plan revision
        if event.code == KeyCode::Char('i') && event.modifiers.contains(KeyModifiers::CONTROL) {
            tracing::info!("🔄 Ctrl+I pressed - Requesting plan revision");
            if let Some(plan) = &self.current_plan {
                // Build plan summary for context
                let plan_summary = format!(
                    "Current plan '{}' has {} tasks:\n{}",
                    plan.title,
                    plan.tasks.len(),
                    plan.tasks
                        .iter()
                        .enumerate()
                        .map(|(i, t)| format!("  {}. {} ({})", i + 1, t.title, t.task_type))
                        .collect::<Vec<_>>()
                        .join("\n")
                );

                // Switch back to chat mode
                self.switch_mode(AppMode::Chat).await?;

                // Pre-fill input with revision request
                self.input_buffer = format!(
                    "Please revise this plan:\n\n{}\n\nRequested changes: ",
                    plan_summary
                );
                self.cursor_position = self.input_buffer.len();

                // Keep plan in memory for reference (don't clear it)
            }
            return Ok(());
        }

        // Arrow keys for scrolling tasks
        match event.code {
            KeyCode::Up => {
                self.plan_scroll_offset = self.plan_scroll_offset.saturating_sub(1);
            }
            KeyCode::Down => {
                if let Some(plan) = &self.current_plan {
                    let max_scroll = plan.tasks.len().saturating_sub(1);
                    self.plan_scroll_offset = (self.plan_scroll_offset + 1).min(max_scroll);
                }
            }
            KeyCode::PageUp => {
                self.plan_scroll_offset = self.plan_scroll_offset.saturating_sub(10);
            }
            KeyCode::PageDown => {
                if let Some(plan) = &self.current_plan {
                    let max_scroll = plan.tasks.len().saturating_sub(1);
                    self.plan_scroll_offset = (self.plan_scroll_offset + 10).min(max_scroll);
                }
            }
            _ => {}
        }

        Ok(())
    }
}
