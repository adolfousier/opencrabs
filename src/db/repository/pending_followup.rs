//! Durable pending follow-up suggestion tracking (#1226 item 3).
//!
//! Telegram suggestion keyboards (`suggest_options`) are backed by a
//! token-keyed in-memory stash: the callback data carries the token, the tap
//! handler resolves it to the option list. That map is process-local, so a
//! restart between render and tap orphaned every live keyboard — buttons
//! stayed rendered but taps could only produce the unknown-token warn.
//!
//! One row per armed keyboard. Mirrors the plan-card store pattern (#809):
//! written on arm (and on merge-host attach), deleted on tap/drop/clear,
//! hydrated into the map at boot. Storage failures are logged and swallowed —
//! the in-memory behaviour stays authoritative and a missing row can only
//! degrade back to today's stale-tap strip, never break a turn.

use anyhow::{Context, Result};
use deadpool_sqlite::Pool;
use rusqlite::params;

use crate::db::database::interact_err;

/// The merged-host bubble a keyboard can ride on, as stored.
/// Mirrors `crate::channels::telegram::state::MergedHost` over the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowupHost {
    pub message_id: i64,
    pub html: String,
    pub rich: bool,
    /// Merged markdown payload for markdown-plane hosts (#79 piece 4):
    /// when set, every edit of this bubble rides `edit_rich_markdown`
    /// (server-side render keeps tables intact); the `html` copy is then
    /// a fallback/strip source only. `None` = html-plane host.
    pub markdown: Option<String>,
}

/// One armed suggestion keyboard, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingFollowup {
    pub token: String,
    pub session_id: String,
    pub options: Vec<String>,
    pub host: Option<FollowupHost>,
}

#[derive(Clone)]
pub struct PendingFollowupRepository {
    pool: Pool,
}

impl PendingFollowupRepository {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Every armed keyboard, for boot hydration.
    pub async fn load_all(&self) -> Result<Vec<PendingFollowup>> {
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| -> Result<Vec<PendingFollowup>> {
                let mut stmt = conn.prepare(
                    "SELECT token, session_id, options_json, host_message_id, host_html, \
                     host_rich, host_markdown FROM pending_followups",
                )?;
                let rows = stmt
                    .query_map([], |row| {
                        let options_json: String = row.get(2)?;
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            options_json,
                            row.get::<_, Option<i64>>(3)?,
                            row.get::<_, Option<String>>(4)?,
                            row.get::<_, i64>(5)? != 0,
                            row.get::<_, Option<String>>(6)?,
                        ))
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                rows.into_iter()
                    .map(
                        |(token, session_id, options_json, mid, html, rich, markdown)| {
                            let options = serde_json::from_str(&options_json)
                                .with_context(|| format!("decode options for token {token}"))?;
                            Ok(PendingFollowup {
                                token,
                                session_id,
                                options,
                                host: mid.map(|message_id| FollowupHost {
                                    message_id,
                                    html: html.unwrap_or_default(),
                                    rich,
                                    markdown,
                                }),
                            })
                        },
                    )
                    .collect()
            })
            .await
            .map_err(interact_err)?
            .context("Failed to read pending followups")
    }

    /// Track (or re-track) one armed keyboard. Upsert: the merge-host attach
    /// re-saves the same token with the host columns filled.
    pub async fn save(&self, entry: &PendingFollowup) -> Result<()> {
        let options_json =
            serde_json::to_string(&entry.options).context("encode followup options")?;
        let (mid, html, rich, markdown) = match &entry.host {
            Some(h) => (
                Some(h.message_id),
                Some(h.html.clone()),
                h.rich,
                h.markdown.clone(),
            ),
            None => (None, None, false, None),
        };
        let token = entry.token.clone();
        let session_id = entry.session_id.clone();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO pending_followups \
                       (token, session_id, options_json, host_message_id, host_html, \
                        host_rich, host_markdown, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, strftime('%s','now')) \
                     ON CONFLICT(token) DO UPDATE SET \
                       session_id = excluded.session_id, \
                       options_json = excluded.options_json, \
                       host_message_id = excluded.host_message_id, \
                       host_html = excluded.host_html, \
                       host_rich = excluded.host_rich, \
                       host_markdown = excluded.host_markdown, \
                       updated_at = excluded.updated_at",
                    params![token, session_id, options_json, mid, html, rich, markdown],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to write pending followup")?;
        Ok(())
    }

    /// Forget one keyboard (tapped or dropped).
    pub async fn delete(&self, token: &str) -> Result<()> {
        let token = token.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "DELETE FROM pending_followups WHERE token = ?1",
                    params![token],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to delete pending followup")?;
        Ok(())
    }

    /// Forget every keyboard belonging to a session (the user sent their own
    /// message; all buttons for that session are stale).
    pub async fn delete_session(&self, session_id: &str) -> Result<()> {
        let sid = session_id.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "DELETE FROM pending_followups WHERE session_id = ?1",
                    params![sid],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to delete session pending followups")?;
        Ok(())
    }
}
