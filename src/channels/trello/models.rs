//! Trello API Data Models
//!
//! Adapted from RKeelan/trello-cli (Apache 2.0).

use serde::{Deserialize, Serialize};

/// Trello Board
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Board {
    pub id: String,
    pub name: String,
}

/// Trello Card
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub desc: String,
    #[serde(rename = "idList")]
    pub id_list: String,
    #[serde(rename = "idBoard")]
    pub id_board: String,
}

/// Trello List
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct List {
    pub id: String,
    pub name: String,
}

/// Trello Label
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
}

/// Trello Action (comment, move, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Action {
    pub id: String,
    #[serde(rename = "type")]
    pub action_type: String,
    pub date: String,
    #[serde(rename = "idMemberCreator")]
    pub id_member_creator: String,
    #[serde(rename = "memberCreator")]
    pub member_creator: ActionMember,
    pub data: ActionData,
}

/// Data payload for an Action
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionData {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub card: Option<ActionCard>,
    #[serde(default)]
    pub board: Option<ActionBoard>,
}

/// Card reference inside ActionData
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionCard {
    pub id: String,
    pub name: String,
}

/// Board reference inside ActionData
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionBoard {
    pub id: String,
    pub name: String,
}

/// Member who created the action
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionMember {
    pub id: String,
    pub username: String,
    #[serde(rename = "fullName", default)]
    pub full_name: String,
}

/// Full card details (returned by get_card with checklists/labels/members)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardDetail {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub desc: String,
    #[serde(rename = "idList")]
    pub id_list: String,
    #[serde(rename = "idBoard")]
    pub id_board: String,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(rename = "dueComplete", default)]
    pub due_complete: bool,
    #[serde(default)]
    pub closed: bool,
    #[serde(default)]
    pub labels: Vec<Label>,
    #[serde(rename = "idMembers", default)]
    pub id_members: Vec<String>,
    #[serde(default)]
    pub checklists: Vec<Checklist>,
}

/// Trello Checklist
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checklist {
    pub id: String,
    pub name: String,
    #[serde(rename = "checkItems", default)]
    pub check_items: Vec<ChecklistItem>,
}

/// Item inside a Checklist
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub id: String,
    pub name: String,
    /// "complete" or "incomplete"
    pub state: String,
}

/// Search results returned by /search
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchResult {
    #[serde(default)]
    pub cards: Vec<Card>,
    #[serde(default)]
    pub boards: Vec<Board>,
}

/// Trello Notification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    #[serde(rename = "type")]
    pub notification_type: String,
    pub date: String,
    #[serde(default)]
    pub unread: bool,
    pub data: NotificationData,
    #[serde(rename = "memberCreator", default)]
    pub member_creator: Option<ActionMember>,
}

/// Data payload for a Notification
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NotificationData {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub card: Option<ActionCard>,
    #[serde(default)]
    pub board: Option<ActionBoard>,
    #[serde(default)]
    pub list: Option<ActionCard>, // list has same id/name shape
}
