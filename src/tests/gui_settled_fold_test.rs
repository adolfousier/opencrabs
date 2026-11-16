//! A turn keeps its ledger folded when it settles, and the count still floors.
//!
//! #2951: `attach_pending_metrics` opened the turn it had just settled, so
//! every completion moved the reader down the transcript and buried the answer
//! under the tool rows. The fix dropped that insert and left the #2632 floor in
//! place, which is what stops a turn that lost its ledger from claiming it did
//! no work.
//!
//! The window has no headless harness, so the shape is read from source, the
//! way the other gui tests do it. Both settle paths are pinned, because a
//! reload runs the second one and could bring the same jump back.

const DASHBOARD_MESSAGES: &str = include_str!("../gui/dashboard_messages.rs");

/// The body of one impl method, from its anchor to the doc comment of the next
/// method or the closing brace of the impl. Both cuts are needed: a method's
/// own closing brace is indented, so a bare `\n}` would run into whatever
/// follows it.
fn method_after<'a>(source: &'a str, anchor: &str) -> &'a str {
    let rest = source
        .split_once(anchor)
        .unwrap_or_else(|| panic!("{anchor} is gone from the source"))
        .1;
    let mut end = rest.len();
    for stop in ["\n    ///", "\n}\n"] {
        if let Some(at) = rest.find(stop) {
            end = end.min(at);
        }
    }
    rest.split_at(end).0
}

/// The settle path never opens the card it just finished. Opening it moved the
/// reader on every completion, which is exactly #2951.
#[test]
fn a_settled_turn_is_not_opened_by_its_settle() {
    let body = method_after(
        DASHBOARD_MESSAGES,
        "pub(crate) fn attach_pending_metrics(&mut self, sid: Uuid) {",
    );
    assert!(
        !body.contains("expanded.insert"),
        "the settle must not open the turn it just finished, got {body:?}"
    );
    assert!(
        !body.contains("transcript.expanded"),
        "the settle must not touch the expanded set at all, got {body:?}"
    );
}

/// The count keeps the #2632 floor: a row that lost its ledger still reports
/// the calls the reader watched, so the card never claims the turn was idle.
#[test]
fn the_settle_keeps_the_tool_count_floor() {
    let body = method_after(
        DASHBOARD_MESSAGES,
        "pub(crate) fn attach_pending_metrics(&mut self, sid: Uuid) {",
    );
    assert!(
        body.contains("tool_calls.max(metrics.tool_calls)"),
        "the settle must floor the count with the calls it watched, got {body:?}"
    );
}

/// A reload runs `claim_pane_metrics`, so the same rule has to hold there or
/// the jump comes back the next time the conversation is opened.
#[test]
fn a_reloaded_pane_does_not_open_the_last_turn() {
    let body = method_after(DASHBOARD_MESSAGES, "pub(crate) fn claim_pane_metrics(");
    assert!(
        !body.contains("expanded.insert"),
        "a reload must not open the last turn, got {body:?}"
    );
    assert!(
        !body.contains("transcript.expanded"),
        "a reload must not touch the expanded set at all, got {body:?}"
    );
    assert!(
        body.contains("tool_calls.max(metrics.tool_calls)"),
        "a reload keeps the count floor too, got {body:?}"
    );
}
