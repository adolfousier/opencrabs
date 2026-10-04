//! #1703 fix 2: every delivered message of a scheduled cron run carries the
//! run that produced it. A reader can anchor a message to the ledger row
//! even when the writer cannot tell fresh work from a stale replay.
//! The ledger row keeps the raw report; only the delivered copy is stamped.
//! Direct-trigger runs are deliberately unstamped: the user asked for them,
//! freshness is not in question.

use crate::cron::scheduler::{run_stamp, stamp_delivery};

const RUN: &str = "01af9c2e-7d31-4c0a-9b2f-1234567890ab";
const AT: &str = "2026-10-04T00:00:00Z";

#[test]
fn stamp_is_short_run_id_plus_start_time() {
    assert_eq!(
        run_stamp(RUN, AT),
        "[run 01af9c2e, started 2026-10-04T00:00:00Z]"
    );
}

#[test]
fn stamp_survives_short_ids() {
    // run ids are uuids in production, but the helper must not panic on a
    // shorter id a test or a future caller might pass.
    assert_eq!(run_stamp("abc", "t"), "[run abc, started t]");
}

#[test]
fn delivery_appends_the_stamp_once() {
    let once = stamp_delivery("report body", RUN, AT);
    assert!(
        once.starts_with("report body\n\n[run 01af9c2e, started 2026-10-04T00:00:00Z]"),
        "{once}"
    );
    let twice = stamp_delivery(&once, RUN, AT);
    assert_eq!(twice, once, "a stamp must never stack on itself");
}

#[test]
fn another_run_gets_its_own_stamp() {
    // The dedupe key is the whole stamp, not a marker phrase: content
    // carrying a different run's stamp still receives this run's stamp.
    let a = stamp_delivery("x", "aaaaaaaa-1111-2222-3333-444455556666", AT);
    let b = stamp_delivery(&a, RUN, AT);
    assert!(b.contains("01af9c2e"), "{b}");
    assert!(b.contains("aaaaaaaa"), "{b}");
}

#[test]
fn scheduled_delivery_paths_call_the_stamper() {
    // Structural sentinel (#1703): both delivery arms of execute_job must
    // stamp what they deliver, and the direct-trigger path must stay
    // unstamped. A refactor that drops or spreads the stamp fails here.
    let src = include_str!("../cron/scheduler.rs");
    let after_ok = src
        .split_once("Ok(response) =>")
        .map(|(_, rest)| rest)
        .expect("success arm of execute_job");
    let (ok_arm, after_err) = after_ok
        .split_once("Err(e) =>")
        .expect("error arm follows the success arm");
    let err_arm = after_err
        .split_once("\n    Ok(())")
        .map(|(body, _)| body)
        .expect("end of execute_job match");
    assert!(
        ok_arm.contains("stamp_delivery(&clean, &run_id, &run_started_at)"),
        "the success arm must deliver stamped content (#1703 fix 2)"
    );
    assert!(
        err_arm.contains("let msg = stamp_delivery("),
        "the error arm must deliver stamped content (#1703 fix 2)"
    );
    let (_, direct) = src
        .split_once("async fn execute_direct_trigger_job")
        .expect("direct trigger path");
    // Bound the direct-trigger body at the next sibling method so a helper
    // defined further down the file cannot fail this assertion by existing.
    let direct = direct
        .split_once("\n    async fn ")
        .map(|(body, _)| body)
        .unwrap_or(direct);
    assert!(
        !direct.contains("stamp_delivery("),
        "direct-trigger runs are intentionally unstamped (#1703 fix 2 scope)"
    );
}
