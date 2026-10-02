//! Durable plan-card tracking (#809).
//!
//! Which Telegram message carries a session's plan card lived only in an
//! in-memory map, so a restart wiped it. The card could then neither be edited
//! (no tracked id) nor removed (the take returned nothing), leaving a
//! permanently stale checklist in the chat: observed showing the final task
//! unchecked hours after the plan had completed and archived correctly.
//!
//! One row per session. The stored `signature` is the last rendered content
//! hash, which is what suppresses no-op edits; losing it on restart also lost
//! that dedup, so it is persisted alongside the id rather than rebuilt.

use anyhow::{Context, Result};
use deadpool_sqlite::Pool;
use rusqlite::OptionalExtension;
use rusqlite::params;

use crate::db::database::interact_err;

/// A tracked plan card, as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanCard {
    pub session_id: String,
    pub chat_id: i64,
    pub thread_id: Option<i64>,
    pub message_id: i64,
    pub signature: String,
}

pub struct PlanCardRepository {
    pool: Pool,
}

impl PlanCardRepository {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// The card tracked for `session_id`, if any.
    pub async fn get(&self, session_id: &str) -> Result<Option<PlanCard>> {
        let sid = session_id.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.query_row(
                    "SELECT session_id, chat_id, thread_id, message_id, signature \
                     FROM plan_cards WHERE session_id = ?1",
                    params![sid],
                    |row| {
                        Ok(PlanCard {
                            session_id: row.get(0)?,
                            chat_id: row.get(1)?,
                            thread_id: row.get(2)?,
                            message_id: row.get(3)?,
                            signature: row.get(4)?,
                        })
                    },
                )
                .optional()
            })
            .await
            .map_err(interact_err)?
            .context("Failed to read plan card")
    }

    /// Track (or re-track) the card for a session.
    pub async fn set(&self, card: PlanCard) -> Result<()> {
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute(
                    "INSERT INTO plan_cards \
                       (session_id, chat_id, thread_id, message_id, signature, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, strftime('%s','now')) \
                     ON CONFLICT(session_id) DO UPDATE SET \
                       chat_id = excluded.chat_id, \
                       thread_id = excluded.thread_id, \
                       message_id = excluded.message_id, \
                       signature = excluded.signature, \
                       updated_at = excluded.updated_at",
                    params![
                        card.session_id,
                        card.chat_id,
                        card.thread_id,
                        card.message_id,
                        card.signature
                    ],
                )
            })
            .await
            .map_err(interact_err)?
            .context("Failed to write plan card")?;
        Ok(())
    }

    /// Stop tracking a session's card.
    pub async fn delete(&self, session_id: &str) -> Result<()> {
        let sid = session_id.to_string();
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                conn.execute("DELETE FROM plan_cards WHERE session_id = ?1", params![sid])
            })
            .await
            .map_err(interact_err)?
            .context("Failed to delete plan card")?;
        Ok(())
    }

    /// Every card currently tracked in one chat, whichever session owns it.
    ///
    /// The read the incident in #1684 needed. `get` is session-scoped, and the
    /// orphaned card belonged to the chat's no-topic fallback session while the
    /// plan work finished in another session, so a session-scoped read could
    /// not see the zombie that was sitting in front of everyone. Chat-scoped is
    /// the only shape that answers "what cards does this chat have?".
    pub async fn list_by_chat(&self, chat_id: i64) -> Result<Vec<PlanCard>> {
        self.pool
            .get()
            .await
            .context("Failed to get connection")?
            .interact(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT session_id, chat_id, thread_id, message_id, signature \
                     FROM plan_cards WHERE chat_id = ?1 ORDER BY updated_at DESC",
                )?;
                let rows = stmt.query_map(params![chat_id], |row| {
                    Ok(PlanCard {
                        session_id: row.get(0)?,
                        chat_id: row.get(1)?,
                        thread_id: row.get(2)?,
                        message_id: row.get(3)?,
                        signature: row.get(4)?,
                    })
                })?;
                let mut cards = Vec::new();
                for row in rows {
                    cards.push(row?);
                }
                Ok::<Vec<PlanCard>, rusqlite::Error>(cards)
            })
            .await
            .map_err(interact_err)?
            .context("Failed to list plan cards for chat")
    }
}
