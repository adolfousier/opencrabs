//! Trello Send Tool
//!
//! Agent-callable tool for proactive Trello card and board operations.
//! Uses the shared `TrelloState` to build a client from stored credentials.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use crate::channels::trello::TrelloState;
use crate::channels::trello::client::TrelloClient;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// Tool that performs Trello operations on behalf of the agent.
pub struct TrelloSendTool {
    trello_state: Arc<TrelloState>,
}

impl TrelloSendTool {
    pub fn new(trello_state: Arc<TrelloState>) -> Self {
        Self { trello_state }
    }
}

#[async_trait]
impl Tool for TrelloSendTool {
    fn name(&self) -> &str {
        "trello_send"
    }

    fn description(&self) -> &str {
        "Full Trello control: read cards/comments/notifications, create/update/archive cards, \
         manage checklists, assign members, add/remove labels, move cards, search across boards, \
         list boards/lists/members, and mark notifications read. \
         Requires Trello to be connected first via trello_connect. \
         Always use this tool instead of http_request for Trello — credentials are \
         handled securely without exposing them in URLs."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "add_comment", "get_card", "get_card_comments", "update_card",
                        "archive_card", "create_card", "move_card", "find_cards",
                        "add_member_to_card", "remove_member_from_card",
                        "add_label_to_card", "remove_label_from_card",
                        "add_checklist", "add_checklist_item", "complete_checklist_item",
                        "list_boards", "list_lists", "get_board_members",
                        "search", "get_notifications", "mark_notifications_read"
                    ],
                    "description": "Operation to perform"
                },
                "board_id": {
                    "type": "string",
                    "description": "Board ID or name (required for create_card, find_cards)"
                },
                "list_name": {
                    "type": "string",
                    "description": "List name within the board (required for create_card; optional for move_card)"
                },
                "card_id": {
                    "type": "string",
                    "description": "Card ID (required for add_comment, move_card)"
                },
                "title": {
                    "type": "string",
                    "description": "Card title (required for create_card)"
                },
                "description": {
                    "type": "string",
                    "description": "Card description (optional for create_card)"
                },
                "text": {
                    "type": "string",
                    "description": "Comment text (required for add_comment)"
                },
                "position": {
                    "type": "string",
                    "enum": ["top", "bottom"],
                    "description": "Card position in list (default: bottom)"
                },
                "pattern": {
                    "type": "string",
                    "description": "Search pattern for find_cards (case-insensitive substring match on card name)"
                },
                "member_id": {
                    "type": "string",
                    "description": "Trello member ID (for add/remove_member_to_card, get_board_members resolution)"
                },
                "label_id": {
                    "type": "string",
                    "description": "Trello label ID (for add/remove_label_to_card)"
                },
                "due_date": {
                    "type": "string",
                    "description": "ISO-8601 due date for update_card (e.g. '2026-03-15T09:00:00Z'), or 'null' to clear"
                },
                "due_complete": {
                    "type": "boolean",
                    "description": "Mark due date as complete/incomplete for update_card"
                },
                "checklist_id": {
                    "type": "string",
                    "description": "Checklist ID (for add_checklist_item, complete_checklist_item)"
                },
                "item_id": {
                    "type": "string",
                    "description": "Checklist item ID (for complete_checklist_item)"
                },
                "complete": {
                    "type": "boolean",
                    "description": "true = mark checklist item complete, false = incomplete"
                },
                "query": {
                    "type": "string",
                    "description": "Search query for the 'search' action"
                },
                "read_filter": {
                    "type": "string",
                    "enum": ["unread", "read", "all"],
                    "description": "Notification filter for get_notifications (default: unread)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max results to return (get_notifications: default 50 max 1000; search: default 20; get_card_comments: default 50)"
                }
            },
            "required": ["action"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::Network]
    }

    async fn execute(&self, input: Value, _context: &ToolExecutionContext) -> Result<ToolResult> {
        let (api_key, api_token) = match self.trello_state.credentials().await {
            Some(creds) => creds,
            None => {
                return Ok(ToolResult::error(
                    "Trello is not connected. Use trello_connect first.".to_string(),
                ));
            }
        };

        let client = TrelloClient::new(&api_key, &api_token);

        let action = match input.get("action").and_then(|v| v.as_str()) {
            Some(a) => a,
            None => {
                return Ok(ToolResult::error(
                    "Missing required 'action' parameter.".to_string(),
                ));
            }
        };

        match action {
            "add_comment" => {
                let card_id = match input.get("card_id").and_then(|v| v.as_str()) {
                    Some(id) if !id.is_empty() => id,
                    _ => {
                        return Ok(ToolResult::error(
                            "add_comment requires 'card_id'.".to_string(),
                        ));
                    }
                };
                let text = match input.get("text").and_then(|v| v.as_str()) {
                    Some(t) if !t.is_empty() => t,
                    _ => {
                        return Ok(ToolResult::error(
                            "add_comment requires 'text'.".to_string(),
                        ));
                    }
                };

                // Split long comments
                let chunks = crate::channels::trello::handler::split_comment(text, 4000);
                for chunk in &chunks {
                    if let Err(e) = client.add_comment_to_card(card_id, chunk).await {
                        return Ok(ToolResult::error(format!("Failed to add comment: {}", e)));
                    }
                }
                Ok(ToolResult::success(format!(
                    "Comment posted to card {} ({} chunk(s)).",
                    card_id,
                    chunks.len()
                )))
            }

            "create_card" => {
                let board_query = match input.get("board_id").and_then(|v| v.as_str()) {
                    Some(b) if !b.is_empty() => b,
                    _ => {
                        return Ok(ToolResult::error(
                            "create_card requires 'board_id'.".to_string(),
                        ));
                    }
                };
                let list_name = match input.get("list_name").and_then(|v| v.as_str()) {
                    Some(l) if !l.is_empty() => l,
                    _ => {
                        return Ok(ToolResult::error(
                            "create_card requires 'list_name'.".to_string(),
                        ));
                    }
                };
                let title = match input.get("title").and_then(|v| v.as_str()) {
                    Some(t) if !t.is_empty() => t,
                    _ => {
                        return Ok(ToolResult::error(
                            "create_card requires 'title'.".to_string(),
                        ));
                    }
                };
                let desc = input
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let pos = input.get("position").and_then(|v| v.as_str());

                let board_id = match client.resolve_board(board_query).await {
                    Ok(id) => id,
                    Err(e) => {
                        return Ok(ToolResult::error(format!(
                            "Could not find board '{}': {}",
                            board_query, e
                        )));
                    }
                };

                let list_id = match client.resolve_list(&board_id, list_name).await {
                    Ok(id) => id,
                    Err(e) => {
                        return Ok(ToolResult::error(format!(
                            "Could not find list '{}': {}",
                            list_name, e
                        )));
                    }
                };

                match client.create_card(&list_id, title, desc, pos).await {
                    Ok(card) => Ok(ToolResult::success(format!(
                        "Card '{}' created (id: {}).",
                        card.name, card.id
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to create card: {}", e))),
                }
            }

            "move_card" => {
                let card_id = match input.get("card_id").and_then(|v| v.as_str()) {
                    Some(id) if !id.is_empty() => id,
                    _ => {
                        return Ok(ToolResult::error(
                            "move_card requires 'card_id'.".to_string(),
                        ));
                    }
                };
                let board_query = match input.get("board_id").and_then(|v| v.as_str()) {
                    Some(b) if !b.is_empty() => b,
                    _ => {
                        return Ok(ToolResult::error(
                            "move_card requires 'board_id'.".to_string(),
                        ));
                    }
                };
                let list_name = match input.get("list_name").and_then(|v| v.as_str()) {
                    Some(l) if !l.is_empty() => l,
                    _ => {
                        return Ok(ToolResult::error(
                            "move_card requires 'list_name'.".to_string(),
                        ));
                    }
                };
                let pos = input.get("position").and_then(|v| v.as_str());

                let board_id = match client.resolve_board(board_query).await {
                    Ok(id) => id,
                    Err(e) => {
                        return Ok(ToolResult::error(format!(
                            "Could not find board '{}': {}",
                            board_query, e
                        )));
                    }
                };

                let list_id = match client.resolve_list(&board_id, list_name).await {
                    Ok(id) => id,
                    Err(e) => {
                        return Ok(ToolResult::error(format!(
                            "Could not find list '{}': {}",
                            list_name, e
                        )));
                    }
                };

                match client.move_card(card_id, &list_id, pos).await {
                    Ok(()) => Ok(ToolResult::success(format!(
                        "Card {} moved to list '{}'.",
                        card_id, list_name
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to move card: {}", e))),
                }
            }

            "find_cards" => {
                let board_query = match input.get("board_id").and_then(|v| v.as_str()) {
                    Some(b) if !b.is_empty() => b,
                    _ => {
                        return Ok(ToolResult::error(
                            "find_cards requires 'board_id'.".to_string(),
                        ));
                    }
                };
                let pattern = input
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_lowercase();

                let board_id = match client.resolve_board(board_query).await {
                    Ok(id) => id,
                    Err(e) => {
                        return Ok(ToolResult::error(format!(
                            "Could not find board '{}': {}",
                            board_query, e
                        )));
                    }
                };

                let cards = match client.get_board_cards(&board_id).await {
                    Ok(c) => c,
                    Err(e) => {
                        return Ok(ToolResult::error(format!("Failed to fetch cards: {}", e)));
                    }
                };

                let matched: Vec<String> = cards
                    .iter()
                    .filter(|c| pattern.is_empty() || c.name.to_lowercase().contains(&pattern))
                    .map(|c| format!("{}: {} (list: {})", c.id, c.name, c.id_list))
                    .collect();

                if matched.is_empty() {
                    Ok(ToolResult::success(format!(
                        "No cards found matching '{}'.",
                        pattern
                    )))
                } else {
                    Ok(ToolResult::success(format!(
                        "Found {} card(s):\n{}",
                        matched.len(),
                        matched.join("\n")
                    )))
                }
            }

            "list_boards" => match client.get_member_boards().await {
                Ok(boards) => {
                    let list: Vec<String> = boards
                        .iter()
                        .map(|b| format!("{}: {}", b.id, b.name))
                        .collect();
                    Ok(ToolResult::success(format!(
                        "{} board(s):\n{}",
                        boards.len(),
                        list.join("\n")
                    )))
                }
                Err(e) => Ok(ToolResult::error(format!("Failed to list boards: {}", e))),
            },

            "get_card" => {
                let card_id = match input.get("card_id").and_then(|v| v.as_str()) {
                    Some(id) if !id.is_empty() => id,
                    _ => {
                        return Ok(ToolResult::error(
                            "get_card requires 'card_id'.".to_string(),
                        ));
                    }
                };
                match client.get_card(card_id).await {
                    Ok(card) => {
                        let due = card.due.as_deref().unwrap_or("none");
                        let labels: Vec<&str> =
                            card.labels.iter().map(|l| l.name.as_str()).collect();
                        let mut out = format!(
                            "Card: {}\nID: {}\nDesc: {}\nDue: {} (complete: {})\nArchived: {}\nLabels: {}\nMembers: {}\n",
                            card.name,
                            card.id,
                            if card.desc.is_empty() {
                                "(none)"
                            } else {
                                &card.desc
                            },
                            due,
                            card.due_complete,
                            card.closed,
                            if labels.is_empty() {
                                "none".to_string()
                            } else {
                                labels.join(", ")
                            },
                            if card.id_members.is_empty() {
                                "none".to_string()
                            } else {
                                card.id_members.join(", ")
                            },
                        );
                        if !card.checklists.is_empty() {
                            out.push_str("\nChecklists:\n");
                            for cl in &card.checklists {
                                out.push_str(&format!("  [{}] {} (id: {})\n", cl.name, "", cl.id));
                                for item in &cl.check_items {
                                    let mark = if item.state == "complete" { "x" } else { " " };
                                    out.push_str(&format!(
                                        "    [{}] {} (id: {})\n",
                                        mark, item.name, item.id
                                    ));
                                }
                            }
                        }
                        Ok(ToolResult::success(out))
                    }
                    Err(e) => Ok(ToolResult::error(format!("Failed to get card: {}", e))),
                }
            }

            "get_card_comments" => {
                let card_id = match input.get("card_id").and_then(|v| v.as_str()) {
                    Some(id) if !id.is_empty() => id,
                    _ => {
                        return Ok(ToolResult::error(
                            "get_card_comments requires 'card_id'.".to_string(),
                        ));
                    }
                };
                let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as u32;
                match client.get_card_comments(card_id, limit).await {
                    Ok(actions) => {
                        if actions.is_empty() {
                            return Ok(ToolResult::success(
                                "No comments on this card.".to_string(),
                            ));
                        }
                        let lines: Vec<String> = actions
                            .iter()
                            .map(|a| {
                                format!(
                                    "[{}] @{}: {}",
                                    &a.date[..10],
                                    a.member_creator.username,
                                    a.data.text
                                )
                            })
                            .collect();
                        Ok(ToolResult::success(format!(
                            "{} comment(s):\n{}",
                            actions.len(),
                            lines.join("\n")
                        )))
                    }
                    Err(e) => Ok(ToolResult::error(format!("Failed to get comments: {}", e))),
                }
            }

            "update_card" => {
                let card_id = match input.get("card_id").and_then(|v| v.as_str()) {
                    Some(id) if !id.is_empty() => id,
                    _ => {
                        return Ok(ToolResult::error(
                            "update_card requires 'card_id'.".to_string(),
                        ));
                    }
                };
                let name = input.get("title").and_then(|v| v.as_str());
                let desc = input.get("description").and_then(|v| v.as_str());
                let due = input.get("due_date").and_then(|v| v.as_str());
                let due_complete = input.get("due_complete").and_then(|v| v.as_bool());
                match client
                    .update_card(card_id, name, desc, due, due_complete, None)
                    .await
                {
                    Ok(card) => Ok(ToolResult::success(format!(
                        "Card '{}' updated.",
                        card.name
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to update card: {}", e))),
                }
            }

            "archive_card" => {
                let card_id = match input.get("card_id").and_then(|v| v.as_str()) {
                    Some(id) if !id.is_empty() => id,
                    _ => {
                        return Ok(ToolResult::error(
                            "archive_card requires 'card_id'.".to_string(),
                        ));
                    }
                };
                match client
                    .update_card(card_id, None, None, None, None, Some(true))
                    .await
                {
                    Ok(card) => Ok(ToolResult::success(format!(
                        "Card '{}' archived.",
                        card.name
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to archive card: {}", e))),
                }
            }

            "add_member_to_card" => {
                let card_id = match input.get("card_id").and_then(|v| v.as_str()) {
                    Some(id) if !id.is_empty() => id,
                    _ => {
                        return Ok(ToolResult::error(
                            "add_member_to_card requires 'card_id'.".to_string(),
                        ));
                    }
                };
                let member_id = match input.get("member_id").and_then(|v| v.as_str()) {
                    Some(id) if !id.is_empty() => id,
                    _ => {
                        return Ok(ToolResult::error(
                            "add_member_to_card requires 'member_id'.".to_string(),
                        ));
                    }
                };
                match client.add_member_to_card(card_id, member_id).await {
                    Ok(()) => Ok(ToolResult::success(format!(
                        "Member {} added to card {}.",
                        member_id, card_id
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to add member: {}", e))),
                }
            }

            "remove_member_from_card" => {
                let card_id = match input.get("card_id").and_then(|v| v.as_str()) {
                    Some(id) if !id.is_empty() => id,
                    _ => {
                        return Ok(ToolResult::error(
                            "remove_member_from_card requires 'card_id'.".to_string(),
                        ));
                    }
                };
                let member_id = match input.get("member_id").and_then(|v| v.as_str()) {
                    Some(id) if !id.is_empty() => id,
                    _ => {
                        return Ok(ToolResult::error(
                            "remove_member_from_card requires 'member_id'.".to_string(),
                        ));
                    }
                };
                match client.remove_member_from_card(card_id, member_id).await {
                    Ok(()) => Ok(ToolResult::success(format!(
                        "Member {} removed from card {}.",
                        member_id, card_id
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to remove member: {}", e))),
                }
            }

            "add_label_to_card" => {
                let card_id = match input.get("card_id").and_then(|v| v.as_str()) {
                    Some(id) if !id.is_empty() => id,
                    _ => {
                        return Ok(ToolResult::error(
                            "add_label_to_card requires 'card_id'.".to_string(),
                        ));
                    }
                };
                let label_id = match input.get("label_id").and_then(|v| v.as_str()) {
                    Some(id) if !id.is_empty() => id,
                    _ => {
                        return Ok(ToolResult::error(
                            "add_label_to_card requires 'label_id'.".to_string(),
                        ));
                    }
                };
                match client.add_label_to_card(card_id, label_id).await {
                    Ok(()) => Ok(ToolResult::success(format!(
                        "Label {} added to card {}.",
                        label_id, card_id
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to add label: {}", e))),
                }
            }

            "remove_label_from_card" => {
                let card_id = match input.get("card_id").and_then(|v| v.as_str()) {
                    Some(id) if !id.is_empty() => id,
                    _ => {
                        return Ok(ToolResult::error(
                            "remove_label_from_card requires 'card_id'.".to_string(),
                        ));
                    }
                };
                let label_id = match input.get("label_id").and_then(|v| v.as_str()) {
                    Some(id) if !id.is_empty() => id,
                    _ => {
                        return Ok(ToolResult::error(
                            "remove_label_from_card requires 'label_id'.".to_string(),
                        ));
                    }
                };
                match client.remove_label_from_card(card_id, label_id).await {
                    Ok(()) => Ok(ToolResult::success(format!(
                        "Label {} removed from card {}.",
                        label_id, card_id
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to remove label: {}", e))),
                }
            }

            "add_checklist" => {
                let card_id = match input.get("card_id").and_then(|v| v.as_str()) {
                    Some(id) if !id.is_empty() => id,
                    _ => {
                        return Ok(ToolResult::error(
                            "add_checklist requires 'card_id'.".to_string(),
                        ));
                    }
                };
                let name = input
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Checklist");
                match client.create_checklist(card_id, name).await {
                    Ok(cl) => Ok(ToolResult::success(format!(
                        "Checklist '{}' created (id: {}) on card {}.",
                        cl.name, cl.id, card_id
                    ))),
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to create checklist: {}",
                        e
                    ))),
                }
            }

            "add_checklist_item" => {
                let checklist_id = match input.get("checklist_id").and_then(|v| v.as_str()) {
                    Some(id) if !id.is_empty() => id,
                    _ => {
                        return Ok(ToolResult::error(
                            "add_checklist_item requires 'checklist_id'.".to_string(),
                        ));
                    }
                };
                let name = match input.get("title").and_then(|v| v.as_str()) {
                    Some(n) if !n.is_empty() => n,
                    _ => {
                        return Ok(ToolResult::error(
                            "add_checklist_item requires 'title'.".to_string(),
                        ));
                    }
                };
                match client.add_checklist_item(checklist_id, name).await {
                    Ok(item) => Ok(ToolResult::success(format!(
                        "Item '{}' added to checklist (id: {}).",
                        item.name, item.id
                    ))),
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to add checklist item: {}",
                        e
                    ))),
                }
            }

            "complete_checklist_item" => {
                let card_id = match input.get("card_id").and_then(|v| v.as_str()) {
                    Some(id) if !id.is_empty() => id,
                    _ => {
                        return Ok(ToolResult::error(
                            "complete_checklist_item requires 'card_id'.".to_string(),
                        ));
                    }
                };
                let item_id = match input.get("item_id").and_then(|v| v.as_str()) {
                    Some(id) if !id.is_empty() => id,
                    _ => {
                        return Ok(ToolResult::error(
                            "complete_checklist_item requires 'item_id'.".to_string(),
                        ));
                    }
                };
                let complete = input
                    .get("complete")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                match client
                    .set_checklist_item_state(card_id, item_id, complete)
                    .await
                {
                    Ok(()) => Ok(ToolResult::success(format!(
                        "Checklist item {} marked as {}.",
                        item_id,
                        if complete { "complete" } else { "incomplete" }
                    ))),
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to update checklist item: {}",
                        e
                    ))),
                }
            }

            "list_lists" => {
                let board_query = match input.get("board_id").and_then(|v| v.as_str()) {
                    Some(b) if !b.is_empty() => b,
                    _ => {
                        return Ok(ToolResult::error(
                            "list_lists requires 'board_id'.".to_string(),
                        ));
                    }
                };
                let board_id = match client.resolve_board(board_query).await {
                    Ok(id) => id,
                    Err(e) => {
                        return Ok(ToolResult::error(format!(
                            "Could not find board '{}': {}",
                            board_query, e
                        )));
                    }
                };
                match client.get_board_lists(&board_id).await {
                    Ok(lists) => {
                        let lines: Vec<String> = lists
                            .iter()
                            .map(|l| format!("{}: {}", l.id, l.name))
                            .collect();
                        Ok(ToolResult::success(format!(
                            "{} list(s) on '{}':\n{}",
                            lists.len(),
                            board_query,
                            lines.join("\n")
                        )))
                    }
                    Err(e) => Ok(ToolResult::error(format!("Failed to list lists: {}", e))),
                }
            }

            "get_board_members" => {
                let board_query = match input.get("board_id").and_then(|v| v.as_str()) {
                    Some(b) if !b.is_empty() => b,
                    _ => {
                        return Ok(ToolResult::error(
                            "get_board_members requires 'board_id'.".to_string(),
                        ));
                    }
                };
                let board_id = match client.resolve_board(board_query).await {
                    Ok(id) => id,
                    Err(e) => {
                        return Ok(ToolResult::error(format!(
                            "Could not find board '{}': {}",
                            board_query, e
                        )));
                    }
                };
                match client.get_board_members(&board_id).await {
                    Ok(members) => {
                        let lines: Vec<String> = members
                            .iter()
                            .map(|m| format!("{}: @{} ({})", m.id, m.username, m.full_name))
                            .collect();
                        Ok(ToolResult::success(format!(
                            "{} member(s) on '{}':\n{}",
                            members.len(),
                            board_query,
                            lines.join("\n")
                        )))
                    }
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to get board members: {}",
                        e
                    ))),
                }
            }

            "search" => {
                let query = match input.get("query").and_then(|v| v.as_str()) {
                    Some(q) if !q.is_empty() => q,
                    _ => return Ok(ToolResult::error("search requires 'query'.".to_string())),
                };
                let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as u32;
                match client.search(query, limit).await {
                    Ok(results) => {
                        let mut out = format!(
                            "Search results for '{}':\n\nCards ({}):\n",
                            query,
                            results.cards.len()
                        );
                        if results.cards.is_empty() {
                            out.push_str("  (none)\n");
                        } else {
                            for card in &results.cards {
                                out.push_str(&format!(
                                    "  {} | {} | board: {}\n",
                                    card.id, card.name, card.id_board
                                ));
                            }
                        }
                        out.push_str(&format!("\nBoards ({}):\n", results.boards.len()));
                        if results.boards.is_empty() {
                            out.push_str("  (none)\n");
                        } else {
                            for board in &results.boards {
                                out.push_str(&format!("  {} | {}\n", board.id, board.name));
                            }
                        }
                        Ok(ToolResult::success(out))
                    }
                    Err(e) => Ok(ToolResult::error(format!("Search failed: {}", e))),
                }
            }

            "get_notifications" => {
                let read_filter = input
                    .get("read_filter")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unread");
                let limit = input
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(50)
                    .min(1000) as u32;

                match client.get_notifications(read_filter, limit).await {
                    Ok(notifs) => {
                        if notifs.is_empty() {
                            return Ok(ToolResult::success(format!(
                                "No {} notifications.",
                                read_filter
                            )));
                        }
                        let lines: Vec<String> = notifs
                            .iter()
                            .map(|n| {
                                let who = n
                                    .member_creator
                                    .as_ref()
                                    .map(|m| m.username.as_str())
                                    .unwrap_or("?");
                                let board =
                                    n.data.board.as_ref().map(|b| b.name.as_str()).unwrap_or("");
                                let card =
                                    n.data.card.as_ref().map(|c| c.name.as_str()).unwrap_or("");
                                let text = n.data.text.as_deref().unwrap_or("");
                                let read_mark = if n.unread { "●" } else { "○" };
                                format!(
                                    "{} [{}] {} by @{} | board: {} | card: {} | {}",
                                    read_mark,
                                    &n.date[..10],
                                    n.notification_type,
                                    who,
                                    board,
                                    card,
                                    text.chars().take(80).collect::<String>()
                                )
                            })
                            .collect();
                        Ok(ToolResult::success(format!(
                            "{} notification(s) ({}):\n{}",
                            notifs.len(),
                            read_filter,
                            lines.join("\n")
                        )))
                    }
                    Err(e) => Ok(ToolResult::error(format!(
                        "Failed to fetch notifications: {}",
                        e
                    ))),
                }
            }

            "mark_notifications_read" => match client.mark_all_notifications_read().await {
                Ok(()) => Ok(ToolResult::success(
                    "All Trello notifications marked as read.".to_string(),
                )),
                Err(e) => Ok(ToolResult::error(format!(
                    "Failed to mark notifications read: {}",
                    e
                ))),
            },

            other => Ok(ToolResult::error(format!(
                "Unknown action '{}'. Valid actions: add_comment, create_card, move_card, find_cards, list_boards, get_notifications, mark_notifications_read",
                other
            ))),
        }
    }
}
