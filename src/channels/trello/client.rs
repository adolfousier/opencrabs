//! Async Trello REST API Client
//!
//! Adapted from RKeelan/trello-cli (Apache 2.0) — converted from blocking to async reqwest.

use super::models::*;
use anyhow::{Context, Result, bail};
use reqwest::Client;

const TRELLO_API_BASE: &str = "https://api.trello.com/1";

/// Async Trello REST API client.
pub struct TrelloClient {
    api_key: String,
    api_token: String,
    http: Client,
}

impl TrelloClient {
    pub fn new(api_key: impl Into<String>, api_token: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            api_token: api_token.into(),
            http: Client::new(),
        }
    }

    /// Build a full URL with key/token auth query params appended.
    fn url(&self, path: &str) -> String {
        if path.contains('?') {
            format!(
                "{}{}&key={}&token={}",
                TRELLO_API_BASE, path, self.api_key, self.api_token
            )
        } else {
            format!(
                "{}{}?key={}&token={}",
                TRELLO_API_BASE, path, self.api_key, self.api_token
            )
        }
    }

    /// Verify credentials and return the bot's member info.
    pub async fn get_member_me(&self) -> Result<ActionMember> {
        let url = self.url("/members/me");
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .context("Failed to reach Trello API")?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Trello auth error {}: {}", status, body);
        }
        resp.json::<ActionMember>()
            .await
            .context("Failed to parse member response")
    }

    /// Get `commentCard` actions on a board since a given ISO-8601 datetime.
    pub async fn get_board_actions_since(
        &self,
        board_id: &str,
        since: &str,
    ) -> Result<Vec<Action>> {
        let encoded_since = urlencoding::encode(since);
        let path = format!(
            "/boards/{}/actions?filter=commentCard&since={}&limit=50",
            board_id, encoded_since
        );
        let url = self.url(&path);
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Trello board actions error {}: {}", status, body);
        }
        resp.json::<Vec<Action>>()
            .await
            .context("Failed to parse board actions")
    }

    /// Get all boards accessible to the authenticated member.
    pub async fn get_member_boards(&self) -> Result<Vec<Board>> {
        let url = self.url("/members/me/boards");
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            bail!("Failed to fetch boards: {}", resp.status());
        }
        resp.json::<Vec<Board>>()
            .await
            .context("Failed to parse boards")
    }

    /// Get all open cards on a board.
    pub async fn get_board_cards(&self, board_id: &str) -> Result<Vec<Card>> {
        let path = format!("/boards/{}/cards?filter=open", board_id);
        let url = self.url(&path);
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            bail!("Failed to fetch cards: {}", resp.status());
        }
        resp.json::<Vec<Card>>()
            .await
            .context("Failed to parse cards")
    }

    /// Get all lists on a board.
    pub async fn get_board_lists(&self, board_id: &str) -> Result<Vec<List>> {
        let path = format!("/boards/{}/lists", board_id);
        let url = self.url(&path);
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            bail!("Failed to fetch lists: {}", resp.status());
        }
        resp.json::<Vec<List>>()
            .await
            .context("Failed to parse lists")
    }

    /// Get all labels on a board.
    pub async fn get_board_labels(&self, board_id: &str) -> Result<Vec<Label>> {
        let path = format!("/boards/{}/labels", board_id);
        let url = self.url(&path);
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            bail!("Failed to fetch labels: {}", resp.status());
        }
        resp.json::<Vec<Label>>()
            .await
            .context("Failed to parse labels")
    }

    /// Add a comment to a card.
    pub async fn add_comment_to_card(&self, card_id: &str, text: &str) -> Result<()> {
        let path = format!("/cards/{}/actions/comments", card_id);
        let url = self.url(&path);
        let resp = self.http.post(&url).form(&[("text", text)]).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to add comment: {}: {}", status, body);
        }
        Ok(())
    }

    /// Create a card in the given list.
    pub async fn create_card(
        &self,
        list_id: &str,
        name: &str,
        desc: &str,
        pos: Option<&str>,
    ) -> Result<Card> {
        let url = self.url("/cards");
        let pos_val = pos.unwrap_or("bottom");
        let resp = self
            .http
            .post(&url)
            .form(&[
                ("idList", list_id),
                ("name", name),
                ("desc", desc),
                ("pos", pos_val),
            ])
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to create card: {}: {}", status, body);
        }
        resp.json::<Card>()
            .await
            .context("Failed to parse card response")
    }

    /// Move a card to a different list.
    pub async fn move_card(&self, card_id: &str, list_id: &str, pos: Option<&str>) -> Result<()> {
        let path = format!("/cards/{}", card_id);
        let url = self.url(&path);
        let mut form_data = vec![("idList", list_id.to_string())];
        if let Some(p) = pos {
            form_data.push(("pos", p.to_string()));
        }
        let resp = self.http.put(&url).form(&form_data).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to move card: {}: {}", status, body);
        }
        Ok(())
    }

    /// Resolve a board name (or ID) to a board ID.
    /// If `query` looks like an existing Trello ID (8+ hex chars), returns as-is.
    pub async fn resolve_board(&self, query: &str) -> Result<String> {
        // If it already looks like a Trello ID, pass through
        if query.len() >= 8 && query.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(query.to_string());
        }
        let boards = self.get_member_boards().await?;
        let q = query.to_lowercase();
        boards
            .iter()
            .find(|b| b.name.to_lowercase().contains(&q))
            .map(|b| b.id.clone())
            .ok_or_else(|| anyhow::anyhow!("No board found matching '{}'", query))
    }

    /// Get full card details including checklists, labels, and member IDs.
    pub async fn get_card(&self, card_id: &str) -> Result<CardDetail> {
        let path = format!(
            "/cards/{}?checklists=all&fields=name,desc,idList,idBoard,due,dueComplete,closed,labels,idMembers",
            card_id
        );
        let url = self.url(&path);
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to fetch card: {}: {}", status, body);
        }
        resp.json::<CardDetail>()
            .await
            .context("Failed to parse card detail")
    }

    /// Update card fields. Pass only the fields to change.
    pub async fn update_card(
        &self,
        card_id: &str,
        name: Option<&str>,
        desc: Option<&str>,
        due: Option<&str>, // ISO-8601 or "null" to clear
        due_complete: Option<bool>,
        closed: Option<bool>,
    ) -> Result<Card> {
        let path = format!("/cards/{}", card_id);
        let url = self.url(&path);
        let mut form: Vec<(&str, String)> = Vec::new();
        if let Some(n) = name {
            form.push(("name", n.to_string()));
        }
        if let Some(d) = desc {
            form.push(("desc", d.to_string()));
        }
        if let Some(d) = due {
            form.push(("due", d.to_string()));
        }
        if let Some(dc) = due_complete {
            form.push(("dueComplete", dc.to_string()));
        }
        if let Some(c) = closed {
            form.push(("closed", c.to_string()));
        }
        let resp = self.http.put(&url).form(&form).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to update card: {}: {}", status, body);
        }
        resp.json::<Card>()
            .await
            .context("Failed to parse updated card")
    }

    /// Get comments on a specific card.
    pub async fn get_card_comments(&self, card_id: &str, limit: u32) -> Result<Vec<Action>> {
        let path = format!(
            "/cards/{}/actions?filter=commentCard&limit={}",
            card_id, limit
        );
        let url = self.url(&path);
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to fetch card comments: {}: {}", status, body);
        }
        resp.json::<Vec<Action>>()
            .await
            .context("Failed to parse card comments")
    }

    /// Get members of a board.
    pub async fn get_board_members(&self, board_id: &str) -> Result<Vec<ActionMember>> {
        let path = format!("/boards/{}/members", board_id);
        let url = self.url(&path);
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to fetch board members: {}: {}", status, body);
        }
        resp.json::<Vec<ActionMember>>()
            .await
            .context("Failed to parse board members")
    }

    /// Add a member to a card by member ID.
    pub async fn add_member_to_card(&self, card_id: &str, member_id: &str) -> Result<()> {
        let path = format!("/cards/{}/idMembers", card_id);
        let url = self.url(&path);
        let resp = self
            .http
            .post(&url)
            .form(&[("value", member_id)])
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to add member to card: {}: {}", status, body);
        }
        Ok(())
    }

    /// Remove a member from a card.
    pub async fn remove_member_from_card(&self, card_id: &str, member_id: &str) -> Result<()> {
        let path = format!("/cards/{}/idMembers/{}", card_id, member_id);
        let url = self.url(&path);
        let resp = self.http.delete(&url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to remove member from card: {}: {}", status, body);
        }
        Ok(())
    }

    /// Add a label to a card by label ID.
    pub async fn add_label_to_card(&self, card_id: &str, label_id: &str) -> Result<()> {
        let path = format!("/cards/{}/idLabels", card_id);
        let url = self.url(&path);
        let resp = self
            .http
            .post(&url)
            .form(&[("value", label_id)])
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to add label to card: {}: {}", status, body);
        }
        Ok(())
    }

    /// Remove a label from a card.
    pub async fn remove_label_from_card(&self, card_id: &str, label_id: &str) -> Result<()> {
        let path = format!("/cards/{}/idLabels/{}", card_id, label_id);
        let url = self.url(&path);
        let resp = self.http.delete(&url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to remove label from card: {}: {}", status, body);
        }
        Ok(())
    }

    /// Create a checklist on a card.
    pub async fn create_checklist(&self, card_id: &str, name: &str) -> Result<Checklist> {
        let url = self.url("/checklists");
        let resp = self
            .http
            .post(&url)
            .form(&[("idCard", card_id), ("name", name)])
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to create checklist: {}: {}", status, body);
        }
        resp.json::<Checklist>()
            .await
            .context("Failed to parse checklist")
    }

    /// Add an item to a checklist.
    pub async fn add_checklist_item(
        &self,
        checklist_id: &str,
        name: &str,
    ) -> Result<ChecklistItem> {
        let path = format!("/checklists/{}/checkItems", checklist_id);
        let url = self.url(&path);
        let resp = self.http.post(&url).form(&[("name", name)]).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to add checklist item: {}: {}", status, body);
        }
        resp.json::<ChecklistItem>()
            .await
            .context("Failed to parse checklist item")
    }

    /// Set a checklist item state: "complete" or "incomplete".
    pub async fn set_checklist_item_state(
        &self,
        card_id: &str,
        item_id: &str,
        complete: bool,
    ) -> Result<()> {
        let path = format!("/cards/{}/checkItem/{}", card_id, item_id);
        let url = self.url(&path);
        let state = if complete { "complete" } else { "incomplete" };
        let resp = self.http.put(&url).form(&[("state", state)]).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to update checklist item: {}: {}", status, body);
        }
        Ok(())
    }

    /// Search across boards and cards.
    pub async fn search(&self, query: &str, limit: u32) -> Result<SearchResult> {
        let encoded = urlencoding::encode(query);
        let path = format!(
            "/search?query={}&modelTypes=cards,boards&cards_limit={}&boards_limit=10&card_fields=name,idBoard,idList,due,closed",
            encoded, limit
        );
        let url = self.url(&path);
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Search failed: {}: {}", status, body);
        }
        resp.json::<SearchResult>()
            .await
            .context("Failed to parse search results")
    }

    /// Get member notifications.
    /// `read_filter`: "all", "read", or "unread" (default: "unread")
    /// `limit`: max notifications to return (default: 50, max: 1000)
    pub async fn get_notifications(
        &self,
        read_filter: &str,
        limit: u32,
    ) -> Result<Vec<Notification>> {
        let path = format!(
            "/members/me/notifications?read_filter={}&limit={}&fields=type,date,unread,data,memberCreator",
            read_filter, limit
        );
        let url = self.url(&path);
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to fetch notifications: {}: {}", status, body);
        }
        resp.json::<Vec<Notification>>()
            .await
            .context("Failed to parse notifications")
    }

    /// Mark all notifications as read.
    pub async fn mark_all_notifications_read(&self) -> Result<()> {
        let url = self.url("/notifications/all/read");
        let resp = self.http.post(&url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Failed to mark notifications read: {}: {}", status, body);
        }
        Ok(())
    }

    /// Resolve a list name to a list ID within a board.
    pub async fn resolve_list(&self, board_id: &str, query: &str) -> Result<String> {
        let lists = self.get_board_lists(board_id).await?;
        let q = query.to_lowercase();
        lists
            .iter()
            .find(|l| l.name.to_lowercase().contains(&q))
            .map(|l| l.id.clone())
            .ok_or_else(|| {
                anyhow::anyhow!("No list found matching '{}' on board {}", query, board_id)
            })
    }
}
