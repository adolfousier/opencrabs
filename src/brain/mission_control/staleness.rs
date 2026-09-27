//! RSI staleness line for Mission Control (#469): a dark RSI looked
//! identical to a healthy one — it went silent for three days while the
//! ledger kept recording, and nothing surfaced it. Pure formatting so the
//! threshold rules are unit tested without a database.

use crate::utils::string::humanize_age;

/// RSI counts as stale when its last activity is older than this while
/// tool events kept recording (activity happened, RSI didn't process it).
pub const RSI_STALE_AFTER_SECS: i64 = 12 * 3600;

/// The Mission Control line for RSI liveness. `now_ts` / `last_ts` are unix
/// seconds; `events_since` counts tool events recorded after the last RSI
/// activity.
pub fn rsi_staleness_line(now_ts: i64, last_ts: Option<i64>, events_since: i64) -> String {
    match last_ts {
        None if events_since > 0 => {
            format!("⚠️ RSI has never run — {events_since} tool event(s) recorded and unprocessed")
        }
        None => "RSI has never run".to_string(),
        Some(ts) => {
            let age = (now_ts - ts).max(0);
            let human = humanize_age(age);
            if age > RSI_STALE_AFTER_SECS && events_since > 0 {
                format!("⚠️ RSI stale: last run {human} ago ({events_since} tool event(s) since)")
            } else {
                format!("RSI last run {human} ago")
            }
        }
    }
}
