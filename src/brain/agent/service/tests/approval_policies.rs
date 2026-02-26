use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[tokio::test]
async fn test_auto_approve_skips_callback() {
    // with_auto_approve_tools(true) — callback never called, tool executes
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let pool = db.pool().clone();
    let context = ServiceContext::new(pool);

    let callback_called = Arc::new(AtomicBool::new(false));
    let callback_called_clone = Arc::clone(&callback_called);

    let provider = Arc::new(MockProviderWithTools::new());
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(MockTool));

    let approval_cb: ApprovalCallback = Arc::new(move |_info| {
        callback_called_clone.store(true, Ordering::SeqCst);
        Box::pin(async move { Ok(true) })
    });

    let agent_service = AgentService::new(provider, context.clone())
        .with_tool_registry(Arc::new(registry))
        .with_auto_approve_tools(true) // auto-approve ON
        .with_approval_callback(Some(approval_cb));

    let session_service = SessionService::new(context);
    let session = session_service
        .create_session(Some("Auto Approve Test".to_string()))
        .await
        .unwrap();

    let response = agent_service
        .send_message_with_tools(session.id, "Use the test tool".to_string(), None)
        .await
        .unwrap();

    assert!(!response.content.is_empty(), "tool should execute and produce response");
    assert!(
        !callback_called.load(Ordering::SeqCst),
        "approval callback should NOT be called when auto_approve_tools is true"
    );
}

#[tokio::test]
async fn test_approval_required_calls_callback() {
    // Tool with requires_approval() -> true triggers approval callback
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let pool = db.pool().clone();
    let context = ServiceContext::new(pool);

    let callback_called = Arc::new(AtomicBool::new(false));
    let callback_called_clone = Arc::clone(&callback_called);

    let provider = Arc::new(MockProviderWithNamedTool::new("approval_tool"));
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(MockToolRequiresApproval));

    let approval_cb: ApprovalCallback = Arc::new(move |_info| {
        callback_called_clone.store(true, Ordering::SeqCst);
        Box::pin(async move { Ok(true) }) // approve
    });

    let agent_service = AgentService::new(provider, context.clone())
        .with_tool_registry(Arc::new(registry))
        .with_auto_approve_tools(false) // auto-approve OFF
        .with_approval_callback(Some(approval_cb));

    let session_service = SessionService::new(context);
    let session = session_service
        .create_session(Some("Approval Callback Test".to_string()))
        .await
        .unwrap();

    let response = agent_service
        .send_message_with_tools(session.id, "Use the approval tool".to_string(), None)
        .await
        .unwrap();

    assert!(!response.content.is_empty());
    assert!(
        callback_called.load(Ordering::SeqCst),
        "approval callback MUST be called for tools that require_approval() -> true"
    );
}

#[tokio::test]
async fn test_approval_denied_sends_error_result() {
    // Callback returns Ok(false) → tool result contains denial message
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let pool = db.pool().clone();
    let context = ServiceContext::new(pool);

    let provider = Arc::new(MockProviderWithNamedTool::new("approval_tool"));
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(MockToolRequiresApproval));

    // Always deny
    let approval_cb: ApprovalCallback =
        Arc::new(move |_info| Box::pin(async move { Ok(false) }));

    let agent_service = AgentService::new(provider, context.clone())
        .with_tool_registry(Arc::new(registry))
        .with_auto_approve_tools(false)
        .with_approval_callback(Some(approval_cb));

    let session_service = SessionService::new(context);
    let session = session_service
        .create_session(Some("Approval Denied Test".to_string()))
        .await
        .unwrap();

    // The tool loop should complete (LLM gets denied error as tool result and continues)
    let result = agent_service
        .send_message_with_tools(session.id, "Use the approval tool".to_string(), None)
        .await;

    // The overall request succeeds (LLM handles the denied tool result and responds)
    assert!(
        result.is_ok(),
        "send_message_with_tools should succeed even when tool is denied"
    );
}

#[tokio::test]
async fn test_approval_callback_receives_session_id() {
    // ToolApprovalInfo.session_id matches the session being processed
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let pool = db.pool().clone();
    let context = ServiceContext::new(pool);

    let session_service = SessionService::new(context.clone());
    let session = session_service
        .create_session(Some("Session ID Check".to_string()))
        .await
        .unwrap();
    let expected_session_id = session.id;

    let captured_session_id: Arc<tokio::sync::Mutex<Option<Uuid>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    let captured_clone = Arc::clone(&captured_session_id);

    let provider = Arc::new(MockProviderWithNamedTool::new("approval_tool"));
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(MockToolRequiresApproval));

    let approval_cb: ApprovalCallback = Arc::new(move |info| {
        let captured = Arc::clone(&captured_clone);
        Box::pin(async move {
            *captured.lock().await = Some(info.session_id);
            Ok(true)
        })
    });

    let agent_service = AgentService::new(provider, context)
        .with_tool_registry(Arc::new(registry))
        .with_auto_approve_tools(false)
        .with_approval_callback(Some(approval_cb));

    agent_service
        .send_message_with_tools(session.id, "Use the approval tool".to_string(), None)
        .await
        .unwrap();

    let captured = *captured_session_id.lock().await;
    assert_eq!(
        captured,
        Some(expected_session_id),
        "ToolApprovalInfo.session_id must match the session being processed"
    );
}

#[tokio::test]
async fn test_no_callback_denies_execution() {
    // No approval callback configured → tool requiring approval is denied
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let pool = db.pool().clone();
    let context = ServiceContext::new(pool);

    let provider = Arc::new(MockProviderWithNamedTool::new("approval_tool"));
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(MockToolRequiresApproval));

    // No approval_callback set
    let agent_service = AgentService::new(provider, context.clone())
        .with_tool_registry(Arc::new(registry))
        .with_auto_approve_tools(false);

    let session_service = SessionService::new(context);
    let session = session_service
        .create_session(Some("No Callback Test".to_string()))
        .await
        .unwrap();

    // Should complete (LLM gets "no approval mechanism" error as tool result and finishes)
    let result = agent_service
        .send_message_with_tools(session.id, "Use the approval tool".to_string(), None)
        .await;

    assert!(
        result.is_ok(),
        "should complete even when no approval callback is set; tool is denied gracefully"
    );
}

#[tokio::test]
async fn test_non_approval_tool_executes_directly() {
    // Tool with requires_approval() -> false skips approval entirely
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let pool = db.pool().clone();
    let context = ServiceContext::new(pool);

    let callback_called = Arc::new(AtomicBool::new(false));
    let callback_called_clone = Arc::clone(&callback_called);

    let provider = Arc::new(MockProviderWithTools::new());
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(MockTool)); // requires_approval() -> false

    let approval_cb: ApprovalCallback = Arc::new(move |_info| {
        callback_called_clone.store(true, Ordering::SeqCst);
        Box::pin(async move { Ok(true) })
    });

    let agent_service = AgentService::new(provider, context.clone())
        .with_tool_registry(Arc::new(registry))
        .with_auto_approve_tools(false) // auto-approve OFF, but tool doesn't need it
        .with_approval_callback(Some(approval_cb));

    let session_service = SessionService::new(context);
    let session = session_service
        .create_session(Some("No Approval Needed Test".to_string()))
        .await
        .unwrap();

    let response = agent_service
        .send_message_with_tools(session.id, "Use the test tool".to_string(), None)
        .await
        .unwrap();

    assert!(!response.content.is_empty(), "tool should execute successfully");
    assert!(
        !callback_called.load(Ordering::SeqCst),
        "approval callback should NOT be called for tools where requires_approval() -> false"
    );
}

#[tokio::test]
async fn test_mixed_tools_approval_and_auto() {
    // Response has 2 tool calls: one requiring approval (approved), one auto — both execute
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let pool = db.pool().clone();
    let context = ServiceContext::new(pool);

    let approval_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let approval_count_clone = Arc::clone(&approval_count);

    // Provider that emits two tool calls: "approval_tool" and "test_tool"
    let provider = Arc::new(MockProviderWithTwoToolCalls::new("approval_tool", "test_tool"));

    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(MockToolRequiresApproval)); // requires approval
    registry.register(Arc::new(MockTool)); // no approval needed

    let approval_cb: ApprovalCallback = Arc::new(move |_info| {
        approval_count_clone.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { Ok(true) }) // approve
    });

    let agent_service = AgentService::new(provider, context.clone())
        .with_tool_registry(Arc::new(registry))
        .with_auto_approve_tools(false)
        .with_approval_callback(Some(approval_cb));

    let session_service = SessionService::new(context);
    let session = session_service
        .create_session(Some("Mixed Tools Test".to_string()))
        .await
        .unwrap();

    let response = agent_service
        .send_message_with_tools(session.id, "Use both tools".to_string(), None)
        .await
        .unwrap();

    assert!(!response.content.is_empty(), "both tools should execute and produce final response");

    // Exactly one approval request: only approval_tool needed it
    assert_eq!(
        approval_count.load(Ordering::SeqCst),
        1,
        "exactly one approval request should be made (for approval_tool only)"
    );
}
