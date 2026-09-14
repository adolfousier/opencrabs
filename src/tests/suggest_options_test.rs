//! Tests for the `suggest_options` tool: option validation via
//! `sanitize_options`, and the non-blocking execute path (fires a progress
//! event through the callback and always succeeds; no callback is a no-op, not
//! an error, because the suggestions are optional).

use crate::brain::agent::ProgressEvent;
use crate::brain::tools::suggest_options::{
    MAX_OPTIONS, RawSuggestionItem, SuggestOptionsTool, SuggestionItem, SuggestionStyle,
    sanitize_options,
};
use crate::brain::tools::{Tool, ToolExecutionContext};
use serde_json::json;
use std::sync::Arc;
use std::sync::Mutex;

fn raw_strs(v: &[&str]) -> Vec<RawSuggestionItem> {
    v.iter()
        .map(|s| RawSuggestionItem::Bare(s.to_string()))
        .collect()
}

#[test]
fn accepts_one_to_max_distinct() {
    assert_eq!(
        sanitize_options(raw_strs(&["do the thing"])).unwrap().len(),
        1
    );
    // Contract-relative fixture: exactly MAX_OPTIONS distinct options are accepted.
    let max: Vec<RawSuggestionItem> = (1..=MAX_OPTIONS)
        .map(|i| RawSuggestionItem::Bare(format!("option {i}")))
        .collect();
    assert_eq!(sanitize_options(max).unwrap().len(), MAX_OPTIONS);
}

#[test]
fn trims_and_drops_empties() {
    let out = sanitize_options(raw_strs(&["  keep  ", "   ", "also"])).unwrap();
    assert_eq!(
        out,
        vec![
            SuggestionItem::default_styled("keep"),
            SuggestionItem::default_styled("also")
        ]
    );
}

#[test]
fn rejects_empty_after_trim() {
    assert!(sanitize_options(raw_strs(&["   ", ""])).is_err());
    assert!(sanitize_options(vec![]).is_err());
}

#[test]
fn rejects_over_cap() {
    // Contract-relative fixture: MAX_OPTIONS + 1 must be rejected.
    let over: Vec<RawSuggestionItem> = (0..=MAX_OPTIONS)
        .map(|i| RawSuggestionItem::Bare(format!("option {i}")))
        .collect();
    assert_eq!(over.len(), MAX_OPTIONS + 1);
    let err = sanitize_options(over).unwrap_err();
    assert!(err.contains("Too many"), "got: {err}");
}

#[test]
fn rejects_duplicates() {
    let err = sanitize_options(raw_strs(&["same", "same"])).unwrap_err();
    assert!(err.contains("Duplicate"), "got: {err}");
}

#[test]
fn single_unmarked_option_promoted_to_primary() {
    let out = sanitize_options(raw_strs(&["single option"])).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].label, "single option");
    assert_eq!(out[0].style, SuggestionStyle::Primary);
}

#[test]
fn single_danger_option_stays_danger() {
    let items = vec![RawSuggestionItem::Styled {
        label: "Abort operation".to_string(),
        style: Some(SuggestionStyle::Danger),
    }];
    let out = sanitize_options(items).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].label, "Abort operation");
    assert_eq!(out[0].style, SuggestionStyle::Danger);
}

#[test]
fn multiple_unmarked_options_stay_default() {
    let out = sanitize_options(raw_strs(&["option A", "option B"])).unwrap();
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].style, SuggestionStyle::Default);
    assert_eq!(out[1].style, SuggestionStyle::Default);
}

#[test]
fn polymorphic_mixed_styles() {
    let items = vec![
        RawSuggestionItem::Bare("Normal".to_string()),
        RawSuggestionItem::Styled {
            label: "Recommended".to_string(),
            style: Some(SuggestionStyle::Primary),
        },
        RawSuggestionItem::Styled {
            label: "Delete".to_string(),
            style: Some(SuggestionStyle::Danger),
        },
    ];
    let out = sanitize_options(items).unwrap();
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].style, SuggestionStyle::Default);
    assert_eq!(out[1].style, SuggestionStyle::Primary);
    assert_eq!(out[2].style, SuggestionStyle::Danger);
}

#[tokio::test]
async fn execute_fires_progress_event() {
    let captured: Arc<Mutex<Vec<crate::brain::tools::suggest_options::SuggestionItem>>> =
        Arc::new(Mutex::new(Vec::new()));
    let sink = captured.clone();
    let cb: crate::brain::agent::ProgressCallback = Arc::new(move |_sid, event| {
        if let ProgressEvent::SuggestedOptions(opts) = event {
            *sink.lock().unwrap() = opts;
        }
    });
    let mut ctx = ToolExecutionContext::new(uuid::Uuid::new_v4());
    ctx.progress_callback = Some(cb);

    let result = SuggestOptionsTool
        .execute(
            json!({ "options": ["run the tests", "show the diff"] }),
            &ctx,
        )
        .await
        .expect("execute");

    assert!(result.success, "error: {:?}", result.error);
    assert_eq!(
        *captured.lock().unwrap(),
        vec![
            crate::brain::tools::suggest_options::SuggestionItem::default_styled("run the tests"),
            crate::brain::tools::suggest_options::SuggestionItem::default_styled("show the diff")
        ]
    );
}

#[tokio::test]
async fn execute_without_callback_is_ok_noop() {
    // Surfaces with no progress bridge just don't render — never an error.
    let ctx = ToolExecutionContext::new(uuid::Uuid::new_v4());
    let result = SuggestOptionsTool
        .execute(json!({ "options": ["one thing"] }), &ctx)
        .await
        .expect("execute");
    assert!(result.success);
}

#[tokio::test]
async fn execute_rejects_bad_options() {
    let ctx = ToolExecutionContext::new(uuid::Uuid::new_v4());
    let result = SuggestOptionsTool
        .execute(json!({ "options": [] }), &ctx)
        .await
        .expect("execute");
    assert!(!result.success);
}

#[test]
fn tool_is_non_blocking_metadata() {
    let tool = SuggestOptionsTool;
    assert_eq!(tool.name(), "suggest_options");
    assert!(!tool.requires_approval());
}

#[test]
fn description_cap_tracks_max_options_const() {
    use crate::brain::tools::Tool;
    use crate::brain::tools::suggest_options::{MAX_OPTIONS, SuggestOptionsTool};

    let d = Tool::description(&SuggestOptionsTool);
    assert!(
        d.contains(&format!("up to {MAX_OPTIONS}")),
        "description cap must track MAX_OPTIONS, not a restated literal (#1176)"
    );
    assert!(
        !d.contains("1-4"),
        "stale pre-merge cap range must not return to the description"
    );
}
