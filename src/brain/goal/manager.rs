//! Goal manager — CRUD and post-turn evaluation for session goals.

use crate::brain::goal::judge::judge_goal;
use crate::brain::goal::types::{GoalDecision, GoalVerdict, MAX_PARSE_FAILURES};
use crate::brain::provider::Provider;
use crate::db::models::GoalState;
use crate::services::ServiceContext;
use chrono::Utc;
use uuid::Uuid;

/// Goal manager: handles goal lifecycle and post-turn evaluation.
pub struct GoalManager {
    ctx: ServiceContext,
}

impl GoalManager {
    pub fn new(ctx: ServiceContext) -> Self {
        Self { ctx }
    }

    /// Set a new goal for a session. Replaces any existing active goal.
    /// If `max_turns` is None, falls back to `agent.goal_max_turns` from config,
    /// or `DEFAULT_MAX_TURNS` (20).
    pub async fn set_goal(
        &self,
        session_id: Uuid,
        goal_text: String,
        channel: Option<String>,
        channel_chat_id: Option<String>,
        max_turns: Option<u32>,
    ) -> Result<GoalState, String> {
        // Clear any existing goal for this session first
        if let Err(e) = self.clear_goal(session_id).await {
            tracing::warn!(error = %e, session_id = %session_id, "failed to clear goal");
        }

        let effective_max_turns =
            max_turns.or_else(|| crate::config::Config::current().agent.goal_max_turns);
        let goal = GoalState::new(
            session_id,
            goal_text,
            channel,
            channel_chat_id,
            effective_max_turns,
        );
        let pool = self.ctx.pool();
        let conn = pool
            .get()
            .await
            .map_err(|e| format!("DB connection failed: {}", e))?;

        let id = goal.id.to_string();
        let sid = goal.session_id.to_string();
        let goal_text_clone = goal.goal_text.clone();
        let max_turns = goal.max_turns;
        let now = Utc::now().to_rfc3339();

        conn.interact(move |conn| {
            conn.execute(
                "INSERT INTO goal_state \
                 (id, session_id, goal_text, state, turns_used, max_turns, \
                  consecutive_parse_failures, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, 'active', 0, ?4, 0, ?5, ?5)",
                rusqlite::params![id, sid, goal_text_clone, max_turns, now],
            )
        })
        .await
        .map_err(|e| format!("DB interact failed: {}", e))?
        .map_err(|e| format!("DB insert failed: {}", e))?;

        tracing::info!("Goal set for session {}: {}", session_id, goal.goal_text);
        Ok(goal)
    }

    /// Clear (delete) the goal for a session.
    pub async fn clear_goal(&self, session_id: Uuid) -> Result<(), String> {
        let pool = self.ctx.pool();
        let conn = pool
            .get()
            .await
            .map_err(|e| format!("DB connection failed: {}", e))?;
        let sid = session_id.to_string();

        conn.interact(move |conn| {
            conn.execute(
                "DELETE FROM goal_state WHERE session_id = ?1",
                rusqlite::params![sid],
            )
        })
        .await
        .map_err(|e| format!("DB interact failed: {}", e))?
        .map_err(|e| format!("DB delete failed: {}", e))?;

        Ok(())
    }

    /// Get the active goal for a session, if any.
    pub async fn get_goal(&self, session_id: Uuid) -> Result<Option<GoalState>, String> {
        let pool = self.ctx.pool();
        let conn = pool
            .get()
            .await
            .map_err(|e| format!("DB connection failed: {}", e))?;
        let sid = session_id.to_string();

        let row: Option<GoalState> = conn
            .interact(move |conn| -> rusqlite::Result<Option<GoalState>> {
                let mut stmt = conn.prepare(
                    "SELECT id, session_id, goal_text, state, turns_used, max_turns, \
                     consecutive_parse_failures, judge_verdict, judge_reason, \
                     channel, channel_chat_id, created_at, updated_at \
                     FROM goal_state WHERE session_id = ?1 LIMIT 1",
                )?;
                let result = stmt
                    .query_row(rusqlite::params![sid], GoalState::from_row)
                    .optional()?;
                Ok(result)
            })
            .await
            .map_err(|e| format!("DB interact failed: {}", e))?
            .map_err(|e| format!("DB query failed: {}", e))?;

        Ok(row)
    }

    /// Pause a goal.
    pub async fn pause_goal(&self, session_id: Uuid) -> Result<(), String> {
        self.set_state(session_id, "paused").await
    }

    /// Resume a paused goal.
    pub async fn resume_goal(&self, session_id: Uuid) -> Result<(), String> {
        self.set_state(session_id, "active").await
    }

    /// Update the goal state string.
    async fn set_state(&self, session_id: Uuid, state: &str) -> Result<(), String> {
        let pool = self.ctx.pool();
        let conn = pool
            .get()
            .await
            .map_err(|e| format!("DB connection failed: {}", e))?;
        let sid = session_id.to_string();
        let st = state.to_string();
        let now = Utc::now().to_rfc3339();

        conn.interact(move |conn| {
            conn.execute(
                "UPDATE goal_state SET state = ?1, updated_at = ?2 \
                 WHERE session_id = ?3",
                rusqlite::params![st, now, sid],
            )
        })
        .await
        .map_err(|e| format!("DB interact failed: {}", e))?
        .map_err(|e| format!("DB update failed: {}", e))?;

        Ok(())
    }

    /// Record a judge verdict in the DB.
    async fn record_verdict(
        &self,
        session_id: Uuid,
        verdict: &GoalVerdict,
        reason: &str,
    ) -> Result<(), String> {
        let pool = self.ctx.pool();
        let conn = pool
            .get()
            .await
            .map_err(|e| format!("DB connection failed: {}", e))?;
        let sid = session_id.to_string();
        let v = match verdict {
            GoalVerdict::Done => "DONE".to_string(),
            GoalVerdict::Continue => "CONTINUE".to_string(),
        };
        let r = reason.to_string();
        let now = Utc::now().to_rfc3339();

        conn.interact(move |conn| {
            conn.execute(
                "UPDATE goal_state SET judge_verdict = ?1, judge_reason = ?2, updated_at = ?3 \
                 WHERE session_id = ?4",
                rusqlite::params![v, r, now, sid],
            )
        })
        .await
        .map_err(|e| format!("DB interact failed: {}", e))?
        .map_err(|e| format!("DB update failed: {}", e))?;

        Ok(())
    }

    /// Increment the turn counter and parse failure count.
    pub async fn increment_turns(
        &self,
        session_id: Uuid,
        parse_failed: bool,
    ) -> Result<(), String> {
        let pool = self.ctx.pool();
        let conn = pool
            .get()
            .await
            .map_err(|e| format!("DB connection failed: {}", e))?;
        let sid = session_id.to_string();
        let now = Utc::now().to_rfc3339();

        conn.interact(move |conn| {
            if parse_failed {
                conn.execute(
                    "UPDATE goal_state SET turns_used = turns_used + 1, \
                     consecutive_parse_failures = consecutive_parse_failures + 1, \
                     updated_at = ?1 WHERE session_id = ?2",
                    rusqlite::params![now, sid],
                )
            } else {
                conn.execute(
                    "UPDATE goal_state SET turns_used = turns_used + 1, \
                     consecutive_parse_failures = 0, updated_at = ?1 \
                     WHERE session_id = ?2",
                    rusqlite::params![now, sid],
                )
            }
        })
        .await
        .map_err(|e| format!("DB interact failed: {}", e))?
        .map_err(|e| format!("DB update failed: {}", e))?;

        Ok(())
    }

    /// Evaluate the goal after a turn completes. This is the core hook.
    ///
    /// Calls the judge, updates state, returns a `GoalDecision` that tells
    /// the tool loop whether to continue or stop.
    pub async fn evaluate_after_turn(
        &self,
        provider: &dyn Provider,
        model: &str,
        session_id: Uuid,
        last_response: &str,
    ) -> GoalDecision {
        // Load current goal
        let goal = match self.get_goal(session_id).await {
            Ok(Some(g)) => g,
            Ok(None) => {
                return GoalDecision::Done {
                    reason: "no active goal".to_string(),
                };
            }
            Err(e) => {
                tracing::error!("Failed to load goal state: {}", e);
                return GoalDecision::Done {
                    reason: format!("goal load error: {}", e),
                };
            }
        };

        // Check if goal is already completed/failed
        if goal.state != "active" {
            return GoalDecision::Done {
                reason: format!("goal state is '{}'", goal.state),
            };
        }

        // Check turn budget
        if goal.turns_used >= goal.max_turns {
            tracing::info!(
                "Goal turn budget exhausted ({}/{}) — pausing",
                goal.turns_used,
                goal.max_turns
            );
            if let Err(e) = self.set_state(session_id, "paused").await {
                tracing::warn!(error = %e, session_id = %session_id, "failed to set goal state to paused");
            }
            return GoalDecision::Paused {
                reason: format!(
                    "Goal turn budget exhausted ({}/{} turns used). The goal has been paused. Use `/goal resume` or increase the turn budget to continue.",
                    goal.turns_used, goal.max_turns
                ),
            };
        }

        // Check consecutive parse failures
        if goal.consecutive_parse_failures >= MAX_PARSE_FAILURES as i32 {
            tracing::warn!(
                "Goal auto-paused after {} consecutive parse failures",
                goal.consecutive_parse_failures
            );
            if let Err(e) = self.set_state(session_id, "paused").await {
                tracing::warn!(error = %e, session_id = %session_id, "failed to set goal state to paused");
            }
            return GoalDecision::Paused {
                reason: format!(
                    "auto-paused after {} consecutive judge parse failures",
                    goal.consecutive_parse_failures
                ),
            };
        }

        // Call the judge
        let decision = judge_goal(provider, model, &goal.goal_text, last_response).await;

        // Record verdict
        let _ = self
            .record_verdict(session_id, &decision.verdict, &decision.reason)
            .await;

        // Increment turns and track parse failures
        let is_parse_issue = decision.reason.starts_with("judge parse error")
            || decision.reason.starts_with("judge returned empty");
        if let Err(e) = self.increment_turns(session_id, is_parse_issue).await {
            tracing::warn!(error = %e, session_id = %session_id, "failed to increment goal turns");
        }

        match decision.verdict {
            GoalVerdict::Done => {
                if let Err(e) = self.set_state(session_id, "completed").await {
                    tracing::warn!(error = %e, session_id = %session_id, "failed to set goal state to completed");
                }
                GoalDecision::Done {
                    reason: decision.reason,
                }
            }
            GoalVerdict::Continue => {
                let mut prompt = format!(
                    "[GOAL CONTINUATION — The goal is not yet satisfied.]\n\n\
                     Goal: {}\n\n\
                     Judge's assessment: {}",
                    goal.goal_text, decision.reason
                );
                if let Some(ref corrections) = decision.corrections {
                    prompt.push_str(&format!("\n\nCorrections: {}", corrections));
                }
                prompt.push_str(
                    "\n\nContinue working toward the goal. \
                     Use tools as needed. The goal will be re-evaluated after your response.",
                );
                GoalDecision::Continue {
                    continuation_prompt: prompt,
                    corrections: decision.corrections,
                }
            }
        }
    }
}

use rusqlite::OptionalExtension;
