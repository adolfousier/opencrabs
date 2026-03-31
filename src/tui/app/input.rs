//! Input handling — keyboard events, history, and approval interception.

use super::events::{AppMode, ToolApprovalResponse, TuiEvent};
use super::*;
use anyhow::Result;
use tokio::sync::mpsc;
use uuid::Uuid;

/// Find the byte offset in `text` where the cumulative display width reaches `target_col`.
fn byte_offset_at_display_col(text: &str, target_col: usize) -> usize {
    let mut width = 0usize;
    for (idx, ch) in text.char_indices() {
        if width >= target_col {
            return idx;
        }
        width += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
    }
    text.len()
}

impl App {
    /// Returns (line_start_byte, column_bytes) for the cursor's current logical line.
    /// `line_start_byte` is the byte offset where the current line begins (after `\n`).
    /// `column_bytes` is how many bytes into the line the cursor is.
    fn cursor_line_position(&self) -> (usize, usize) {
        let line_start = self.input_buffer[..self.cursor_position]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        let col = self.cursor_position - line_start;
        (line_start, col)
    }

    /// Get the effective text width per visual line in the input box.
    /// Accounts for borders (2) and prefix/padding (2).
    fn input_visual_line_width() -> usize {
        let term_width = crossterm::terminal::size()
            .map(|(w, _)| w as usize)
            .unwrap_or(80);
        // input_content_width = term_width - 2 (borders)
        // text area = input_content_width - 2 (prefix "❯ " or padding "  ")
        term_width.saturating_sub(4).max(10)
    }

    /// Move cursor up one visual line within a logical line, accounting for soft wrapping.
    /// Returns true if the cursor moved, false if already on the first visual line.
    fn cursor_visual_up(&mut self) -> bool {
        use unicode_width::UnicodeWidthStr;

        let (line_start, _col_bytes) = self.cursor_line_position();
        let text_before = &self.input_buffer[line_start..self.cursor_position];
        let display_col = text_before.width();
        let vw = Self::input_visual_line_width();

        let visual_row = display_col / vw;
        if visual_row == 0 {
            // Already on first visual line of this logical line
            return false;
        }

        // Target: same display column but one visual row up
        let target_display_col = (visual_row - 1) * vw + (display_col % vw);
        // Find byte offset at that display column within this logical line
        let line_text = &self.input_buffer[line_start..];
        let line_end = line_text.find('\n').unwrap_or(line_text.len());
        let line_text = &line_text[..line_end];
        self.cursor_position =
            line_start + byte_offset_at_display_col(line_text, target_display_col);
        true
    }

    /// Move cursor down one visual line within a logical line, accounting for soft wrapping.
    /// Returns true if the cursor moved, false if already on the last visual line.
    fn cursor_visual_down(&mut self) -> bool {
        use unicode_width::UnicodeWidthStr;

        let (line_start, _col_bytes) = self.cursor_line_position();
        let text_before = &self.input_buffer[line_start..self.cursor_position];
        let display_col = text_before.width();
        let vw = Self::input_visual_line_width();

        // Get full logical line
        let line_text = &self.input_buffer[line_start..];
        let line_end = line_text.find('\n').unwrap_or(line_text.len());
        let line_text = &line_text[..line_end];
        let total_width = line_text.width();

        let visual_row = display_col / vw;
        let max_visual_row = if total_width == 0 {
            0
        } else {
            (total_width.saturating_sub(1)) / vw
        };

        if visual_row >= max_visual_row {
            // Already on last visual line of this logical line
            return false;
        }

        // Target: same display column but one visual row down
        let target_display_col = ((visual_row + 1) * vw + (display_col % vw)).min(total_width);
        self.cursor_position =
            line_start + byte_offset_at_display_col(line_text, target_display_col);
        true
    }

    /// Map terminal row to message index using the render-time line mapping
    fn row_to_msg_idx(&self, row: u16) -> Option<usize> {
        let row_in_chat = row.saturating_sub(self.chat_area_y + 1) as usize;
        let line_idx = self.chat_render_scroll + row_in_chat;
        self.chat_line_to_msg.get(line_idx).copied().flatten()
    }

    /// Left-click: select/highlight a message
    pub(crate) fn handle_click_select(&mut self, row: u16) {
        let msg_idx = self.row_to_msg_idx(row);
        // Toggle: click same message deselects, click different selects
        if self.selected_message_idx == msg_idx {
            self.selected_message_idx = None;
        } else {
            self.selected_message_idx = msg_idx;
        }
    }

    /// Right-click: copy the clicked (or selected) message to clipboard
    pub(crate) fn handle_right_click_copy(&mut self, row: u16) {
        // Use clicked message, or fall back to already-selected message
        let msg_idx = self.row_to_msg_idx(row).or(self.selected_message_idx);

        let Some(idx) = msg_idx else { return };
        let content = match self.messages.get(idx) {
            Some(msg) if !msg.content.trim().is_empty() => msg.content.clone(),
            _ => return,
        };

        if Self::copy_to_clipboard(&content) {
            self.notification = Some("Copied to clipboard".to_string());
            self.notification_shown_at = Some(std::time::Instant::now());
            self.selected_message_idx = None;
        }
    }

    /// Copy text to system clipboard
    fn copy_to_clipboard(text: &str) -> bool {
        use std::io::Write;
        use std::process::{Command, Stdio};

        // Try pbcopy (macOS)
        if let Ok(mut child) = Command::new("pbcopy")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(ref mut stdin) = child.stdin {
                let _ = stdin.write_all(text.as_bytes());
            }
            return child.wait().is_ok_and(|s| s.success());
        }

        // Try xclip (Linux)
        if let Ok(mut child) = Command::new("xclip")
            .args(["-selection", "clipboard"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(ref mut stdin) = child.stdin {
                let _ = stdin.write_all(text.as_bytes());
            }
            return child.wait().is_ok_and(|s| s.success());
        }

        // Try xsel (Linux fallback)
        if let Ok(mut child) = Command::new("xsel")
            .args(["--clipboard", "--input"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(ref mut stdin) = child.stdin {
                let _ = stdin.write_all(text.as_bytes());
            }
            return child.wait().is_ok_and(|s| s.success());
        }

        false
    }

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
                            crate::utils::persist_auto_session_policy();
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

        // When emoji picker is active, intercept navigation keys
        if self.emoji_picker_active {
            if keys::is_up(&event) {
                self.emoji_selected_index = self.emoji_selected_index.saturating_sub(1);
                return Ok(());
            } else if keys::is_down(&event) {
                if !self.emoji_filtered.is_empty() {
                    self.emoji_selected_index =
                        (self.emoji_selected_index + 1).min(self.emoji_filtered.len() - 1);
                }
                return Ok(());
            } else if event.code == KeyCode::Tab
                || keys::is_enter(&event)
                || keys::is_submit(&event)
            {
                self.accept_emoji();
                return Ok(());
            } else if keys::is_cancel(&event) {
                self.dismiss_emoji_picker();
                return Ok(());
            }
            // Other keys fall through to normal handling (typing more chars refines the filter)
        }

        // Any key other than Escape resets escape confirmation
        if !keys::is_cancel(&event) {
            self.escape_pending_at = None;
        }

        // --- Attachment focus navigation ---
        // When an attachment is focused, Up/Down navigate between attachments,
        // Backspace/Delete removes the focused one, any other key returns to input.
        if self.focused_attachment.is_some() {
            if keys::is_up(&event) {
                // Move to previous attachment (or stay at first)
                if let Some(idx) = self.focused_attachment
                    && idx > 0
                {
                    self.focused_attachment = Some(idx - 1);
                }
                return Ok(());
            } else if keys::is_down(&event) {
                // Move to next attachment, or return to input if at last
                if let Some(idx) = self.focused_attachment {
                    if idx + 1 < self.attachments.len() {
                        self.focused_attachment = Some(idx + 1);
                    } else {
                        self.focused_attachment = None; // back to input
                    }
                }
                return Ok(());
            } else if event.code == KeyCode::Backspace || event.code == KeyCode::Delete {
                // Remove the focused attachment
                if let Some(idx) = self.focused_attachment
                    && idx < self.attachments.len()
                {
                    self.attachments.remove(idx);
                    // Adjust focus: stay on same index if more remain, else move back
                    if self.attachments.is_empty() {
                        self.focused_attachment = None;
                    } else if idx >= self.attachments.len() {
                        self.focused_attachment = Some(self.attachments.len() - 1);
                    }
                }
                return Ok(());
            } else if keys::is_cancel(&event) {
                // Escape returns to input without removing
                self.focused_attachment = None;
                return Ok(());
            } else {
                // Any other key returns to input
                self.focused_attachment = None;
                // Fall through to handle the key normally
            }
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
                self.dismiss_emoji_picker();
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
            self.focused_attachment = None;
            self.slash_suggestions_active = false;
            self.dismiss_emoji_picker();

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
                        // Cancel the active cancel token
                        if let Some(token) = &self.cancel_token {
                            token.cancel();
                        }
                        // Also cancel any stashed session token (e.g. from session switch)
                        if let Some(ref session) = self.current_session {
                            if let Some(stashed) = self.session_cancel_tokens.remove(&session.id) {
                                stashed.cancel();
                            }
                            self.processing_sessions.remove(&session.id);
                        }
                        self.is_processing = false;
                        self.processing_started_at = None;
                        self.streaming_response = None;
                        self.streaming_reasoning = None;
                        self.cancel_token = None;
                        self.escape_pending_at = None;
                        self.active_tool_group = None;
                        self.streaming_output_tokens = 0;
                        self.intermediate_text_received = false;
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
                        // Reload session from DB so tool calls appear inline
                        // (matching the persisted order from expand_message) instead
                        // of being stacked at the bottom from in-memory finalization.
                        if let Some(ref session) = self.current_session {
                            let session_id = session.id;
                            self.load_session(session_id).await?;
                        }
                        self.push_system_message("Operation cancelled.".to_string());
                    } else {
                        self.escape_pending_at = Some(std::time::Instant::now());
                        self.error_message = Some("Press Esc again to abort".to_string());
                        self.error_message_shown_at = Some(std::time::Instant::now());
                    }
                } else {
                    self.escape_pending_at = Some(std::time::Instant::now());
                    self.error_message = Some("Press Esc again to abort".to_string());
                    self.error_message_shown_at = Some(std::time::Instant::now());
                }
            } else if !self.auto_scroll {
                // User is scrolled up — scroll to bottom first
                self.scroll_offset = 0;
                self.auto_scroll = true;
                self.error_message = None;
                self.error_message_shown_at = None;
                self.escape_pending_at = None;
            } else if self.input_buffer.is_empty() {
                // Nothing to clear, just dismiss error
                self.error_message = None;
                self.error_message_shown_at = None;
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
                    self.focused_attachment = None;
                    self.error_message = None;
                    self.error_message_shown_at = None;
                    self.escape_pending_at = None;
                    self.slash_suggestions_active = false;
                    self.dismiss_emoji_picker();
                } else {
                    // Expired — treat as first Escape again
                    self.escape_pending_at = Some(std::time::Instant::now());
                    self.error_message = Some("Press Esc again to clear input".to_string());
                    self.error_message_shown_at = Some(std::time::Instant::now());
                }
            } else {
                // First Escape — show confirmation hint
                self.escape_pending_at = Some(std::time::Instant::now());
                self.error_message = Some("Press Esc again to clear input".to_string());
                self.error_message_shown_at = Some(std::time::Instant::now());
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
            && self.input_history_index.is_none()
            && self.cursor_position > 0
        {
            // Arrow Up — try visual line first, then logical line, then start
            if !self.cursor_visual_up() {
                // Already on first visual line of this logical line
                let line_start = self.cursor_line_position().0;
                if line_start == 0 {
                    // First logical line — move to start
                    self.cursor_position = 0;
                } else {
                    // Move to previous logical line, try to land on its last visual row
                    // at the same column offset
                    use unicode_width::UnicodeWidthStr;
                    let prev_line_end = line_start - 1;
                    let prev_line_start = self.input_buffer[..prev_line_end]
                        .rfind('\n')
                        .map(|i| i + 1)
                        .unwrap_or(0);
                    let prev_line = &self.input_buffer[prev_line_start..prev_line_end];
                    let prev_total_width = prev_line.width();
                    let vw = Self::input_visual_line_width();
                    let last_row = if prev_total_width == 0 {
                        0
                    } else {
                        (prev_total_width.saturating_sub(1)) / vw
                    };
                    let current_col = self.input_buffer[line_start..self.cursor_position].width();
                    let target_col = last_row * vw
                        + (current_col % vw).min(prev_total_width.saturating_sub(last_row * vw));
                    self.cursor_position =
                        prev_line_start + byte_offset_at_display_col(prev_line, target_col);
                }
            }
        } else if keys::is_down(&event)
            && !self.slash_suggestions_active
            && self.input_history_index.is_none()
            && self.cursor_position < self.input_buffer.len()
        {
            // Arrow Down — try visual line first, then logical line, then end
            if !self.cursor_visual_down() {
                // Already on last visual line of this logical line
                let line_start = self.cursor_line_position().0;
                let line_end = self.input_buffer[line_start..]
                    .find('\n')
                    .map(|i| line_start + i)
                    .unwrap_or(self.input_buffer.len());
                if line_end == self.input_buffer.len() {
                    // Last logical line — move to end
                    self.cursor_position = self.input_buffer.len();
                } else {
                    // Move to next logical line, first visual row, same column offset
                    use unicode_width::UnicodeWidthStr;
                    let next_line_start = line_end + 1;
                    let next_line_end = self.input_buffer[next_line_start..]
                        .find('\n')
                        .map(|i| next_line_start + i)
                        .unwrap_or(self.input_buffer.len());
                    let next_line = &self.input_buffer[next_line_start..next_line_end];
                    let current_col = self.input_buffer[line_start..self.cursor_position].width();
                    let vw = Self::input_visual_line_width();
                    let target_col = (current_col % vw).min(next_line.width());
                    self.cursor_position =
                        next_line_start + byte_offset_at_display_col(next_line, target_col);
                }
            }
        } else if keys::is_up(&event)
            && !self.slash_suggestions_active
            && !self.attachments.is_empty()
            && self.cursor_position == 0
            && self.input_history_index.is_none()
        {
            // Arrow Up at start of input with attachments — focus last attachment.
            // User can then Up/Down to navigate, Backspace/Delete to remove.
            self.focused_attachment = Some(self.attachments.len() - 1);
        } else if keys::is_up(&event)
            && !self.slash_suggestions_active
            && self.queued_message_preview.is_some()
            && self.input_history_index.is_none()
        {
            // Arrow Up while a message is queued — dequeue it for editing.
            // The queue puts the text in input_buffer too, so this works
            // regardless of whether the buffer is empty or matches the queued text.
            // Removes from the queue so the user can modify and re-send via Enter.
            self.queued_message_preview.take();
            *self.message_queue.lock().await = None;
            // Keep current input_buffer as-is (already has the queued text)
            self.cursor_position = self.input_buffer.len();
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
                    if !event.modifiers.contains(KeyModifiers::CONTROL)
                        || event.modifiers.contains(KeyModifiers::ALT) =>
                {
                    // Accept character input when:
                    // - No CONTROL modifier (normal typing, SHIFT, SUPER, etc.)
                    // - CONTROL+ALT together (AltGr on Windows — needed for / ? @ \
                    //   on non-US keyboard layouts)
                    // This excludes CONTROL-only combos (Ctrl+C, Ctrl+N, etc.)
                    // which are handled earlier in the input chain.
                    self.input_buffer.insert(self.cursor_position, c);
                    self.cursor_position += c.len_utf8();
                }
                KeyCode::Backspace if event.modifiers.is_empty() && self.cursor_position > 0 => {
                    // Find the previous char boundary
                    let prev = self.input_buffer[..self.cursor_position]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    self.input_buffer.remove(prev);
                    self.cursor_position = prev;
                }
                KeyCode::Delete
                    if event.modifiers.is_empty()
                        && self.cursor_position < self.input_buffer.len() =>
                {
                    self.input_buffer.remove(self.cursor_position);
                }
                KeyCode::Left
                    if event.modifiers.is_empty()
                    // Move cursor left one character
                    && self.cursor_position > 0 =>
                {
                    let prev = self.input_buffer[..self.cursor_position]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    self.cursor_position = prev;
                }
                KeyCode::Right
                    if event.modifiers.is_empty()
                    // Move cursor right one character
                    && self.cursor_position < self.input_buffer.len() =>
                {
                    let next = self.input_buffer[self.cursor_position..]
                        .char_indices()
                        .nth(1)
                        .map(|(i, _)| self.cursor_position + i)
                        .unwrap_or(self.input_buffer.len());
                    self.cursor_position = next;
                }
                KeyCode::Home => {
                    // Jump to start of current line (not absolute start)
                    let line_start = self.input_buffer[..self.cursor_position]
                        .rfind('\n')
                        .map(|i| i + 1)
                        .unwrap_or(0);
                    self.cursor_position = line_start;
                }
                KeyCode::End => {
                    // Jump to end of current line (not absolute end)
                    let line_end = self.input_buffer[self.cursor_position..]
                        .find('\n')
                        .map(|i| self.cursor_position + i)
                        .unwrap_or(self.input_buffer.len());
                    self.cursor_position = line_end;
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

        // Update emoji picker after any keystroke that modifies input
        if !self.slash_suggestions_active {
            self.update_emoji_picker();
        }

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
                let session_id = session.id;

                // If this session is already in another pane, switch focus there
                let existing_pane = self.pane_manager.panes.iter().find(|p| {
                    p.session_id == Some(session_id) && p.id != self.pane_manager.focused
                });
                if let Some(pane) = existing_pane {
                    let target_id = pane.id;
                    self.pane_manager.focused = target_id;
                }

                self.load_session(session_id).await?;
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
        } else if event.code == KeyCode::Char('|') {
            // Split horizontal (left | right)
            // Ensure current session is pinned to the original pane before splitting
            if let Some(ref session) = self.current_session {
                let sid = session.id;
                if let Some(pane) = self.pane_manager.focused_pane_mut() {
                    pane.session_id = Some(sid);
                }
            }
            self.pane_manager
                .split(crate::tui::pane::SplitDirection::Horizontal);
            self.pane_manager.save_layout();
            // Stay on sessions screen — user picks which session goes in the new pane.
            // When they press Enter, load_session assigns it to the focused (new) pane.
        } else if event.code == KeyCode::Char('_') {
            // Split vertical (top / bottom)
            if let Some(ref session) = self.current_session {
                let sid = session.id;
                if let Some(pane) = self.pane_manager.focused_pane_mut() {
                    pane.session_id = Some(sid);
                }
            }
            self.pane_manager
                .split(crate::tui::pane::SplitDirection::Vertical);
            self.pane_manager.save_layout();
            // Stay on sessions screen — user picks which session goes in the new pane.
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
                // Clean up all cached state for this session
                self.session_context_cache.remove(&session_id);
                self.pane_message_cache.remove(&session_id);
                self.session_cancel_tokens.remove(&session_id);
                self.processing_sessions.remove(&session_id);
                if is_current {
                    self.current_session = None;
                    self.messages.clear();
                    self.render_cache.clear();
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
}
