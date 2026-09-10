//! Shared session-id resolution for every CLI surface that accepts a session
//! id from the user (#1340 follow-up: one resolver, all call sites).
//!
//! `session list` prints only the first 8 chars of each id, so every command
//! must accept what the tool itself shows. Full UUIDs pass through untouched;
//! anything else is matched as a case-insensitive prefix. 0 matches is an
//! error, 1 resolves, several list the candidates — never a guess.

use crate::db::models::Session;
use uuid::Uuid;

/// Match a case-insensitive id prefix to exactly one session.
///
/// 0 matches -> Err, 1 -> Ok(id), several -> Err listing candidates. This is
/// the single prefix-matching core that both [`resolve_session_id`] and
/// `resolve_targets`' id branch delegate to, so the ambiguity rules can't
/// drift between commands. Also consumed by the `oc://` target resolver
/// (#148) — the ONE prefix law.
pub fn resolve_one_by_prefix(sessions: &[Session], prefix: &str) -> Result<Uuid, String> {
    let prefix = prefix.to_lowercase();
    let matches: Vec<&Session> = sessions
        .iter()
        .filter(|s| s.id.to_string().to_lowercase().starts_with(&prefix))
        .collect();
    match matches.len() {
        0 => Err(format!("no session id starts with '{prefix}'")),
        1 => Ok(matches[0].id),
        _ => Err(format!(
            "'{prefix}' is ambiguous — candidates:\n{}",
            candidates(&matches)
        )),
    }
}

/// Resolve a user-supplied session id for commands that take exactly one
/// target (#1340).
///
/// Full UUIDs parse as a fast path (existing behavior preserved verbatim,
/// including ids not present in the DB, which the caller reports as
/// not-found). Anything else is matched as a case-insensitive prefix via
/// [`resolve_one_by_prefix`].
pub(crate) fn resolve_session_id(sessions: &[Session], id: &str) -> Result<Uuid, String> {
    if let Ok(uuid) = Uuid::parse_str(id) {
        return Ok(uuid);
    }
    resolve_one_by_prefix(sessions, id)
}

/// Format the candidate list used in ambiguity errors.
pub(crate) fn candidates(matches: &[&Session]) -> String {
    matches
        .iter()
        .map(|s| {
            format!(
                "  {} {}",
                &s.id.to_string()[..8],
                s.title.as_deref().unwrap_or("untitled")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Resolve `--session <arg>` against the DB: an existing session id resumes,
/// `None` creates a fresh one titled `default_title` (#1368).
///
/// This is the async tier of the same concern the pure helpers above cover:
/// every CLI surface that can resume a session funnels through here so the
/// id rules (full UUID passthrough, case-insensitive prefix, ambiguity
/// candidates) cannot drift between commands. Archived sessions are
/// resumable, matching `session get`/`notify`.
pub(crate) async fn resolve_or_create_session(
    session_service: &crate::services::SessionService,
    arg: Option<&str>,
    default_title: &str,
) -> anyhow::Result<crate::db::models::Session> {
    use crate::db::repository::SessionListOptions;

    let Some(id) = arg else {
        return session_service
            .create_session(Some(default_title.to_string()))
            .await;
    };
    let sessions = session_service
        .list_sessions(SessionListOptions {
            include_archived: true,
            ..Default::default()
        })
        .await?;
    let uuid = resolve_session_id(&sessions, id).map_err(anyhow::Error::msg)?;
    session_service
        .get_session(uuid)
        .await?
        .ok_or_else(|| anyhow::anyhow!("session not found: {id}"))
}
