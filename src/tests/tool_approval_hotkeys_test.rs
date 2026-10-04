//! Input-level tests for the inline tool-approval direct verbs (#1775).
//!
//! The shared footer advertises `a: approve` and `D/r/Esc: deny`; these
//! tests drive a live App through `handle_event` with a pending approval
//! and assert each advertised key does exactly what the footer promises:
//! response on the channel, message removed. Enter keeps confirming the
//! highlighted option (Yes/Always/No) and Esc keeps denying.

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::brain::agent::service::AgentService;
use crate::db::Database;
use crate::services::ServiceContext;
use crate::tests::agent_service_mocks::MockProvider;
use crate::tui::app::events::TuiEvent;
use crate::tui::app::state::{ApprovalData, ApprovalState};
use crate::tui::app::{App, DisplayMessage};

async fn app_with_pending_approval(
    selected: usize,
) -> (
    App,
    mpsc::UnboundedReceiver<crate::tui::app::events::ToolApprovalResponse>,
) {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let context = ServiceContext::new(db.pool().clone());
    let provider: Arc<dyn crate::brain::provider::Provider> = Arc::new(MockProvider);
    let service = Arc::new(AgentService::new_for_test(provider, context.clone()).await);
    #[cfg(feature = "whatsapp")]
    let mut app = App::new(
        service,
        context,
        Arc::new(crate::channels::whatsapp::WhatsAppState::new()),
    );
    #[cfg(not(feature = "whatsapp"))]
    let mut app = App::new(service, context);

    let (tx, rx) = mpsc::unbounded_channel();
    app.messages.push(DisplayMessage {
        id: Uuid::new_v4(),
        role: "assistant".to_string(),
        content: "approval request".to_string(),
        timestamp: chrono::Utc::now(),
        token_count: None,
        cost: None,
        approval: Some(ApprovalData {
            tool_name: "bash".to_string(),
            tool_description: "run a command".to_string(),
            tool_input: serde_json::json!({"command": "ls"}),
            capabilities: vec![],
            request_id: Uuid::new_v4(),
            response_tx: tx,
            requested_at: std::time::Instant::now(),
            state: ApprovalState::Pending,
            selected_option: selected,
            show_details: false,
        }),
        approve_menu: None,
        details: None,
        expanded: false,
        expanded_full: false,
        tool_group: None,
        duration_secs: None,
    });
    (app, rx)
}

fn key(code: KeyCode) -> TuiEvent {
    TuiEvent::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

#[tokio::test]
async fn a_hotkey_approves_pending_approval() {
    let (mut app, mut rx) = app_with_pending_approval(2).await; // "No" highlighted
    app.handle_event(key(KeyCode::Char('a'))).await.unwrap();

    let resp = rx.try_recv().expect("`a` must send a response");
    assert!(
        resp.approved,
        "`a` approves regardless of highlighted option"
    );
    assert!(
        app.messages.iter().all(|m| m.approval.is_none()),
        "resolved approval message must be removed"
    );
    assert!(
        !app.approval_auto_session,
        "`a` must never enable the session auto policy"
    );
}

#[tokio::test]
async fn r_hotkey_denies_pending_approval() {
    let (mut app, mut rx) = app_with_pending_approval(0).await; // "Yes" highlighted
    app.handle_event(key(KeyCode::Char('r'))).await.unwrap();

    let resp = rx.try_recv().expect("`r` must send a response");
    assert!(
        !resp.approved,
        "`r` denies regardless of highlighted option"
    );
    assert!(app.messages.iter().all(|m| m.approval.is_none()));
}

#[tokio::test]
async fn enter_still_confirms_highlighted_no_option() {
    // Enter on "No" (index 2) must keep denying — the a/r hotkeys did not
    // repurpose Enter.
    let (mut app, mut rx) = app_with_pending_approval(2).await;
    app.handle_event(key(KeyCode::Enter)).await.unwrap();

    let resp = rx.try_recv().expect("Enter must send a response");
    assert!(!resp.approved, "Enter on 'No' denies");
    assert!(app.messages.iter().all(|m| m.approval.is_none()));
}

#[tokio::test]
async fn enter_still_confirms_highlighted_yes_option() {
    let (mut app, mut rx) = app_with_pending_approval(0).await;
    app.handle_event(key(KeyCode::Enter)).await.unwrap();

    let resp = rx.try_recv().expect("Enter must send a response");
    assert!(resp.approved, "Enter on 'Yes' approves");
    assert!(app.messages.iter().all(|m| m.approval.is_none()));
}

#[tokio::test]
async fn d_and_esc_still_deny_directly() {
    for code in [KeyCode::Char('d'), KeyCode::Esc] {
        let (mut app, mut rx) = app_with_pending_approval(0).await;
        app.handle_event(key(code)).await.unwrap();
        let resp = rx.try_recv().expect("D/Esc must send a response");
        assert!(!resp.approved, "{code:?} denies");
        assert!(app.messages.iter().all(|m| m.approval.is_none()));
    }
}

#[tokio::test]
async fn modified_a_does_not_trigger_hotkey() {
    // Ctrl+A / Alt+A must fall through to the normal input path, not
    // approve a tool call.
    let (mut app, mut rx) = app_with_pending_approval(0).await;
    app.handle_event(TuiEvent::Key(KeyEvent::new(
        KeyCode::Char('a'),
        KeyModifiers::CONTROL,
    )))
    .await
    .unwrap();
    assert!(rx.try_recv().is_err(), "Ctrl+A must not approve");
    assert!(
        app.messages.iter().any(|m| m.approval.is_some()),
        "approval must still be pending"
    );
}

/// The footer vocabulary and the handler stay one source: the catalog's
/// deny entry includes the `r` synonym.
#[test]
fn tool_approval_catalog_deny_entry_carries_r() {
    let keys = crate::tui::app::dialog_keys::dialog_keys(
        crate::tui::app::dialog_keys::DialogScope::ToolApproval,
    );
    let deny = keys.iter().find(|k| k.verb == "deny").expect("deny entry");
    assert!(deny.label.contains('r'), "deny label: {}", deny.label);
}
