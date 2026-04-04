//! Sub-Agent / Swarm System Tests
//!
//! Covers SubAgentManager state machine, all 5 tool operations
//! (spawn, wait, send_input, close, resume), lifecycle transitions,
//! input channel wiring, cancellation, and concurrent access.

// ─── SubAgentManager Unit Tests ────────────────────────────────────────────

mod manager {
    use crate::brain::tools::subagent::SubAgentManager;
    use crate::brain::tools::subagent::{SubAgent, SubAgentState};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    fn make_agent(id: &str, label: &str) -> SubAgent {
        let (tx, _rx) = mpsc::unbounded_channel::<String>();
        SubAgent {
            id: id.to_string(),
            label: label.to_string(),
            session_id: Uuid::new_v4(),
            state: SubAgentState::Running,
            cancel_token: CancellationToken::new(),
            join_handle: None,
            input_tx: Some(tx),
            output: None,
            spawned_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn new_manager_is_empty() {
        let mgr = SubAgentManager::new();
        assert!(mgr.list().is_empty());
    }

    #[test]
    fn default_creates_empty_manager() {
        let mgr = SubAgentManager::default();
        assert!(mgr.list().is_empty());
    }

    #[test]
    fn generate_id_is_8_chars() {
        let id = SubAgentManager::generate_id();
        assert_eq!(id.len(), 8);
        // Should be hex chars from UUID
        assert!(id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
    }

    #[test]
    fn insert_and_get_state() {
        let mgr = SubAgentManager::new();
        let agent = make_agent("a1", "test-agent");
        mgr.insert(agent);

        assert_eq!(mgr.get_state("a1"), Some(SubAgentState::Running));
        assert!(mgr.exists("a1"));
    }

    #[test]
    fn get_state_missing_returns_none() {
        let mgr = SubAgentManager::new();
        assert_eq!(mgr.get_state("nonexistent"), None);
    }

    #[test]
    fn get_output_initially_none() {
        let mgr = SubAgentManager::new();
        mgr.insert(make_agent("a1", "test"));
        assert_eq!(mgr.get_output("a1"), None);
    }

    #[test]
    fn update_output_preserves_running_state() {
        let mgr = SubAgentManager::new();
        mgr.insert(make_agent("a1", "test"));

        mgr.update_output("a1", "partial result".to_string());

        assert_eq!(mgr.get_state("a1"), Some(SubAgentState::Running));
        assert_eq!(mgr.get_output("a1"), Some("partial result".to_string()));
    }

    #[test]
    fn update_output_on_nonexistent_is_noop() {
        let mgr = SubAgentManager::new();
        mgr.update_output("ghost", "data".to_string());
        // No panic, no entry created
        assert!(!mgr.exists("ghost"));
    }

    #[test]
    fn mark_completed_sets_state_and_output() {
        let mgr = SubAgentManager::new();
        mgr.insert(make_agent("a1", "test"));

        mgr.mark_completed("a1", "final output".to_string());

        assert_eq!(mgr.get_state("a1"), Some(SubAgentState::Completed));
        assert_eq!(mgr.get_output("a1"), Some("final output".to_string()));
    }

    #[test]
    fn mark_completed_clears_input_tx() {
        let mgr = SubAgentManager::new();
        mgr.insert(make_agent("a1", "test"));
        assert!(mgr.get_input_tx("a1").is_some());

        mgr.mark_completed("a1", "done".to_string());
        assert!(mgr.get_input_tx("a1").is_none());
    }

    #[test]
    fn mark_failed_sets_state_and_clears_input() {
        let mgr = SubAgentManager::new();
        mgr.insert(make_agent("a1", "test"));

        mgr.mark_failed("a1", "something broke".to_string());

        assert_eq!(
            mgr.get_state("a1"),
            Some(SubAgentState::Failed("something broke".to_string()))
        );
        assert!(mgr.get_input_tx("a1").is_none());
    }

    #[test]
    fn cancel_running_agent_succeeds() {
        let mgr = SubAgentManager::new();
        let agent = make_agent("a1", "test");
        let token = agent.cancel_token.clone();
        mgr.insert(agent);

        assert!(mgr.cancel("a1"));
        assert_eq!(mgr.get_state("a1"), Some(SubAgentState::Cancelled));
        assert!(token.is_cancelled());
        assert!(mgr.get_input_tx("a1").is_none());
    }

    #[test]
    fn cancel_non_running_agent_returns_false() {
        let mgr = SubAgentManager::new();
        mgr.insert(make_agent("a1", "test"));
        mgr.mark_completed("a1", "done".to_string());

        assert!(!mgr.cancel("a1"));
        assert_eq!(mgr.get_state("a1"), Some(SubAgentState::Completed));
    }

    #[test]
    fn cancel_nonexistent_returns_false() {
        let mgr = SubAgentManager::new();
        assert!(!mgr.cancel("ghost"));
    }

    #[test]
    fn get_input_tx_returns_sender() {
        let mgr = SubAgentManager::new();
        mgr.insert(make_agent("a1", "test"));

        let tx = mgr.get_input_tx("a1");
        assert!(tx.is_some());
    }

    #[test]
    fn get_input_tx_missing_returns_none() {
        let mgr = SubAgentManager::new();
        assert!(mgr.get_input_tx("ghost").is_none());
    }

    #[test]
    fn take_join_handle_returns_none_when_not_set() {
        let mgr = SubAgentManager::new();
        mgr.insert(make_agent("a1", "test")); // make_agent sets handle to None
        assert!(mgr.take_join_handle("a1").is_none());
    }

    #[test]
    fn set_and_take_join_handle() {
        let mgr = SubAgentManager::new();
        mgr.insert(make_agent("a1", "test"));

        let handle = tokio::runtime::Runtime::new().unwrap().spawn(async {});
        mgr.set_join_handle("a1", handle);

        assert!(mgr.take_join_handle("a1").is_some());
        // Second take returns None
        assert!(mgr.take_join_handle("a1").is_none());
    }

    #[test]
    fn prepare_resume_from_completed() {
        let mgr = SubAgentManager::new();
        mgr.insert(make_agent("a1", "test"));
        mgr.mark_completed("a1", "done".to_string());

        let new_token = CancellationToken::new();
        let (new_tx, _rx) = mpsc::unbounded_channel::<String>();

        assert!(mgr.prepare_resume("a1", new_token, new_tx));
        assert_eq!(mgr.get_state("a1"), Some(SubAgentState::Running));
        assert_eq!(mgr.get_output("a1"), None); // output cleared
        assert!(mgr.get_input_tx("a1").is_some()); // new channel set
    }

    #[test]
    fn prepare_resume_from_failed() {
        let mgr = SubAgentManager::new();
        mgr.insert(make_agent("a1", "test"));
        mgr.mark_failed("a1", "error".to_string());

        let new_token = CancellationToken::new();
        let (new_tx, _rx) = mpsc::unbounded_channel::<String>();

        assert!(mgr.prepare_resume("a1", new_token, new_tx));
        assert_eq!(mgr.get_state("a1"), Some(SubAgentState::Running));
    }

    #[test]
    fn prepare_resume_from_running_fails() {
        let mgr = SubAgentManager::new();
        mgr.insert(make_agent("a1", "test")); // Running

        let new_token = CancellationToken::new();
        let (new_tx, _rx) = mpsc::unbounded_channel::<String>();

        assert!(!mgr.prepare_resume("a1", new_token, new_tx));
        assert_eq!(mgr.get_state("a1"), Some(SubAgentState::Running));
    }

    #[test]
    fn prepare_resume_from_cancelled_fails() {
        let mgr = SubAgentManager::new();
        mgr.insert(make_agent("a1", "test"));
        mgr.cancel("a1");

        let new_token = CancellationToken::new();
        let (new_tx, _rx) = mpsc::unbounded_channel::<String>();

        assert!(!mgr.prepare_resume("a1", new_token, new_tx));
    }

    #[test]
    fn list_returns_all_agents() {
        let mgr = SubAgentManager::new();
        mgr.insert(make_agent("a1", "alpha"));
        mgr.insert(make_agent("a2", "beta"));
        mgr.insert(make_agent("a3", "gamma"));

        let list = mgr.list();
        assert_eq!(list.len(), 3);

        let ids: Vec<&str> = list.iter().map(|(id, _, _)| id.as_str()).collect();
        assert!(ids.contains(&"a1"));
        assert!(ids.contains(&"a2"));
        assert!(ids.contains(&"a3"));
    }

    #[test]
    fn exists_returns_false_after_remove() {
        let mgr = SubAgentManager::new();
        mgr.insert(make_agent("a1", "test"));
        assert!(mgr.exists("a1"));

        let removed = mgr.remove("a1");
        assert!(removed.is_some());
        assert!(!mgr.exists("a1"));
    }

    #[test]
    fn remove_nonexistent_returns_none() {
        let mgr = SubAgentManager::new();
        assert!(mgr.remove("ghost").is_none());
    }

    #[test]
    fn get_session_id() {
        let mgr = SubAgentManager::new();
        let agent = make_agent("a1", "test");
        let expected_sid = agent.session_id;
        mgr.insert(agent);

        assert_eq!(mgr.get_session_id("a1"), Some(expected_sid));
    }

    #[test]
    fn get_session_id_missing_returns_none() {
        let mgr = SubAgentManager::new();
        assert_eq!(mgr.get_session_id("ghost"), None);
    }

    #[test]
    fn concurrent_access_is_safe() {
        use std::sync::Arc;
        use std::thread;

        let mgr = Arc::new(SubAgentManager::new());
        let mut handles = vec![];

        // Spawn 10 threads inserting concurrently
        for i in 0..10 {
            let mgr = mgr.clone();
            handles.push(thread::spawn(move || {
                let id = format!("agent-{}", i);
                mgr.insert(make_agent(&id, &format!("worker-{}", i)));
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(mgr.list().len(), 10);
    }

    #[test]
    fn input_channel_delivers_messages() {
        let mgr = SubAgentManager::new();
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let agent = SubAgent {
            id: "a1".to_string(),
            label: "test".to_string(),
            session_id: Uuid::new_v4(),
            state: SubAgentState::Running,
            cancel_token: CancellationToken::new(),
            join_handle: None,
            input_tx: Some(tx),
            output: None,
            spawned_at: chrono::Utc::now(),
        };
        mgr.insert(agent);

        // Send via manager's tx
        let sender = mgr.get_input_tx("a1").unwrap();
        sender.send("hello".to_string()).unwrap();
        sender.send("world".to_string()).unwrap();

        // Receive on the other end
        assert_eq!(rx.try_recv().unwrap(), "hello");
        assert_eq!(rx.try_recv().unwrap(), "world");
    }
}

// ─── SendInputTool Tests ───────────────────────────────────────────────────

mod send_input_tool {
    use crate::brain::tools::subagent::SendInputTool;
    use crate::brain::tools::subagent::{SubAgent, SubAgentManager, SubAgentState};
    use crate::brain::tools::{Tool, ToolExecutionContext};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    fn test_context() -> ToolExecutionContext {
        ToolExecutionContext {
            session_id: Uuid::new_v4(),
            working_directory: std::path::PathBuf::from("/tmp"),
            env_vars: HashMap::new(),
            auto_approve: true,
            timeout_secs: 30,
            sudo_callback: None,
            shared_working_directory: None,
            service_context: None,
        }
    }

    fn make_running_agent(id: &str) -> (SubAgent, mpsc::UnboundedReceiver<String>) {
        let (tx, rx) = mpsc::unbounded_channel::<String>();
        let agent = SubAgent {
            id: id.to_string(),
            label: "test".to_string(),
            session_id: Uuid::new_v4(),
            state: SubAgentState::Running,
            cancel_token: CancellationToken::new(),
            join_handle: None,
            input_tx: Some(tx),
            output: None,
            spawned_at: chrono::Utc::now(),
        };
        (agent, rx)
    }

    #[tokio::test]
    async fn missing_agent_id_returns_error() {
        let mgr = Arc::new(SubAgentManager::new());
        let tool = SendInputTool::new(mgr);
        let ctx = test_context();

        let result = tool.execute(json!({"text": "hi"}), &ctx).await;
        assert!(result.is_err()); // InvalidInput error
    }

    #[tokio::test]
    async fn missing_text_returns_error() {
        let mgr = Arc::new(SubAgentManager::new());
        let tool = SendInputTool::new(mgr);
        let ctx = test_context();

        let result = tool.execute(json!({"agent_id": "a1"}), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn nonexistent_agent_returns_tool_error() {
        let mgr = Arc::new(SubAgentManager::new());
        let tool = SendInputTool::new(mgr);
        let ctx = test_context();

        let result = tool
            .execute(json!({"agent_id": "ghost", "text": "hi"}), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
        assert!(
            result
                .error
                .as_ref()
                .unwrap()
                .contains("No sub-agent found")
        );
    }

    #[tokio::test]
    async fn send_to_completed_agent_returns_error() {
        let mgr = Arc::new(SubAgentManager::new());
        let (agent, _rx) = make_running_agent("a1");
        mgr.insert(agent);
        mgr.mark_completed("a1", "done".to_string());

        let tool = SendInputTool::new(mgr);
        let ctx = test_context();

        let result = tool
            .execute(json!({"agent_id": "a1", "text": "hi"}), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("not running"));
    }

    #[tokio::test]
    async fn send_to_running_agent_succeeds() {
        let mgr = Arc::new(SubAgentManager::new());
        let (agent, mut rx) = make_running_agent("a1");
        mgr.insert(agent);

        let tool = SendInputTool::new(mgr);
        let ctx = test_context();

        let result = tool
            .execute(json!({"agent_id": "a1", "text": "do something"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("Input sent"));

        // Message actually arrived
        assert_eq!(rx.try_recv().unwrap(), "do something");
    }

    #[tokio::test]
    async fn send_after_channel_closed_returns_error() {
        let mgr = Arc::new(SubAgentManager::new());
        let (tx, rx) = mpsc::unbounded_channel::<String>();
        let agent = SubAgent {
            id: "a1".to_string(),
            label: "test".to_string(),
            session_id: Uuid::new_v4(),
            state: SubAgentState::Running,
            cancel_token: CancellationToken::new(),
            join_handle: None,
            input_tx: Some(tx),
            output: None,
            spawned_at: chrono::Utc::now(),
        };
        mgr.insert(agent);

        // Drop receiver to close channel
        drop(rx);

        let tool = SendInputTool::new(mgr);
        let ctx = test_context();

        let result = tool
            .execute(json!({"agent_id": "a1", "text": "hi"}), &ctx)
            .await;
        // Should be Err (ToolError::Execution) since send fails
        assert!(result.is_err());
    }
}

// ─── CloseAgentTool Tests ──────────────────────────────────────────────────

mod close_agent_tool {
    use crate::brain::tools::subagent::CloseAgentTool;
    use crate::brain::tools::subagent::{SubAgent, SubAgentManager, SubAgentState};
    use crate::brain::tools::{Tool, ToolExecutionContext};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    fn test_context() -> ToolExecutionContext {
        ToolExecutionContext {
            session_id: Uuid::new_v4(),
            working_directory: std::path::PathBuf::from("/tmp"),
            env_vars: HashMap::new(),
            auto_approve: true,
            timeout_secs: 30,
            sudo_callback: None,
            shared_working_directory: None,
            service_context: None,
        }
    }

    fn make_running_agent(id: &str) -> SubAgent {
        let (tx, _rx) = mpsc::unbounded_channel::<String>();
        SubAgent {
            id: id.to_string(),
            label: "test".to_string(),
            session_id: Uuid::new_v4(),
            state: SubAgentState::Running,
            cancel_token: CancellationToken::new(),
            join_handle: None,
            input_tx: Some(tx),
            output: None,
            spawned_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn missing_agent_id_returns_error() {
        let mgr = Arc::new(SubAgentManager::new());
        let tool = CloseAgentTool::new(mgr);
        let ctx = test_context();

        let result = tool.execute(json!({}), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn close_nonexistent_returns_error() {
        let mgr = Arc::new(SubAgentManager::new());
        let tool = CloseAgentTool::new(mgr);
        let ctx = test_context();

        let result = tool
            .execute(json!({"agent_id": "ghost"}), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
        assert!(
            result
                .error
                .as_ref()
                .unwrap()
                .contains("No sub-agent found")
        );
    }

    #[tokio::test]
    async fn close_running_agent_cancels() {
        let mgr = Arc::new(SubAgentManager::new());
        let agent = make_running_agent("a1");
        let token = agent.cancel_token.clone();
        mgr.insert(agent);

        let tool = CloseAgentTool::new(mgr.clone());
        let ctx = test_context();

        let result = tool.execute(json!({"agent_id": "a1"}), &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("cancelled"));
        assert!(token.is_cancelled());
        assert_eq!(mgr.get_state("a1"), Some(SubAgentState::Cancelled));
        // Still tracked
        assert!(mgr.exists("a1"));
    }

    #[tokio::test]
    async fn close_with_remove_deletes_from_tracking() {
        let mgr = Arc::new(SubAgentManager::new());
        mgr.insert(make_running_agent("a1"));

        let tool = CloseAgentTool::new(mgr.clone());
        let ctx = test_context();

        let result = tool
            .execute(json!({"agent_id": "a1", "remove": true}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("removed"));
        assert!(!mgr.exists("a1"));
    }

    #[tokio::test]
    async fn close_already_completed_agent() {
        let mgr = Arc::new(SubAgentManager::new());
        mgr.insert(make_running_agent("a1"));
        mgr.mark_completed("a1", "done".to_string());

        let tool = CloseAgentTool::new(mgr.clone());
        let ctx = test_context();

        // Close on a completed agent should still succeed (just doesn't cancel)
        let result = tool.execute(json!({"agent_id": "a1"}), &ctx).await.unwrap();
        assert!(result.success);
        // State stays Completed since cancel() returns false for non-running
        assert_eq!(mgr.get_state("a1"), Some(SubAgentState::Completed));
    }
}

// ─── WaitAgentTool Tests ───────────────────────────────────────────────────

mod wait_agent_tool {
    use crate::brain::tools::subagent::WaitAgentTool;
    use crate::brain::tools::subagent::{SubAgent, SubAgentManager, SubAgentState};
    use crate::brain::tools::{Tool, ToolExecutionContext};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    fn test_context() -> ToolExecutionContext {
        ToolExecutionContext {
            session_id: Uuid::new_v4(),
            working_directory: std::path::PathBuf::from("/tmp"),
            env_vars: HashMap::new(),
            auto_approve: true,
            timeout_secs: 30,
            sudo_callback: None,
            shared_working_directory: None,
            service_context: None,
        }
    }

    fn make_running_agent(id: &str) -> SubAgent {
        let (tx, _rx) = mpsc::unbounded_channel::<String>();
        SubAgent {
            id: id.to_string(),
            label: "test".to_string(),
            session_id: Uuid::new_v4(),
            state: SubAgentState::Running,
            cancel_token: CancellationToken::new(),
            join_handle: None,
            input_tx: Some(tx),
            output: None,
            spawned_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn missing_agent_id_returns_error() {
        let mgr = Arc::new(SubAgentManager::new());
        let tool = WaitAgentTool::new(mgr);
        let ctx = test_context();

        let result = tool.execute(json!({}), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn wait_nonexistent_returns_error() {
        let mgr = Arc::new(SubAgentManager::new());
        let tool = WaitAgentTool::new(mgr);
        let ctx = test_context();

        let result = tool
            .execute(json!({"agent_id": "ghost"}), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
        assert!(
            result
                .error
                .as_ref()
                .unwrap()
                .contains("No sub-agent found")
        );
    }

    #[tokio::test]
    async fn wait_already_completed_returns_immediately() {
        let mgr = Arc::new(SubAgentManager::new());
        mgr.insert(make_running_agent("a1"));
        mgr.mark_completed("a1", "result data".to_string());

        let tool = WaitAgentTool::new(mgr);
        let ctx = test_context();

        let result = tool.execute(json!({"agent_id": "a1"}), &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("completed"));
        assert!(result.output.contains("result data"));
    }

    #[tokio::test]
    async fn wait_already_failed_returns_immediately() {
        let mgr = Arc::new(SubAgentManager::new());
        mgr.insert(make_running_agent("a1"));
        mgr.mark_failed("a1", "something broke".to_string());

        let tool = WaitAgentTool::new(mgr);
        let ctx = test_context();

        let result = tool.execute(json!({"agent_id": "a1"}), &ctx).await.unwrap();
        assert!(!result.success);
        let err = result.error.as_ref().unwrap();
        assert!(err.contains("failed"));
        assert!(err.contains("something broke"));
    }

    #[tokio::test]
    async fn wait_cancelled_returns_immediately() {
        let mgr = Arc::new(SubAgentManager::new());
        mgr.insert(make_running_agent("a1"));
        mgr.cancel("a1");

        let tool = WaitAgentTool::new(mgr);
        let ctx = test_context();

        let result = tool.execute(json!({"agent_id": "a1"}), &ctx).await.unwrap();
        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("cancelled"));
    }

    #[tokio::test]
    async fn wait_with_join_handle_completes() {
        let mgr = Arc::new(SubAgentManager::new());
        let mgr_clone = mgr.clone();
        mgr.insert(make_running_agent("a1"));

        // Create a task that completes quickly and marks agent done
        let handle = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            mgr_clone.mark_completed("a1", "async result".to_string());
        });
        mgr.set_join_handle("a1", handle);

        let tool = WaitAgentTool::new(mgr);
        let ctx = test_context();

        let result = tool
            .execute(json!({"agent_id": "a1", "timeout_secs": 5}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("async result"));
    }

    #[tokio::test]
    async fn wait_timeout_returns_error() {
        let mgr = Arc::new(SubAgentManager::new());
        mgr.insert(make_running_agent("a1"));

        // Create a task that takes forever
        let handle = tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_secs(600)).await;
        });
        mgr.set_join_handle("a1", handle);

        let tool = WaitAgentTool::new(mgr);
        let ctx = test_context();

        let result = tool
            .execute(json!({"agent_id": "a1", "timeout_secs": 1}), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("Timed out"));
    }

    #[tokio::test]
    async fn wait_no_handle_returns_state() {
        let mgr = Arc::new(SubAgentManager::new());
        mgr.insert(make_running_agent("a1")); // No join handle

        let tool = WaitAgentTool::new(mgr);
        let ctx = test_context();

        let result = tool.execute(json!({"agent_id": "a1"}), &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("state"));
    }
}

// ─── Lifecycle Integration Tests ───────────────────────────────────────────

mod lifecycle {
    use crate::brain::tools::subagent::{SubAgent, SubAgentManager, SubAgentState};
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    fn make_agent(id: &str) -> (SubAgent, mpsc::UnboundedReceiver<String>) {
        let (tx, rx) = mpsc::unbounded_channel::<String>();
        let agent = SubAgent {
            id: id.to_string(),
            label: "lifecycle-test".to_string(),
            session_id: Uuid::new_v4(),
            state: SubAgentState::Running,
            cancel_token: CancellationToken::new(),
            join_handle: None,
            input_tx: Some(tx),
            output: None,
            spawned_at: chrono::Utc::now(),
        };
        (agent, rx)
    }

    #[test]
    fn full_lifecycle_spawn_complete_resume() {
        let mgr = Arc::new(SubAgentManager::new());

        // 1. Spawn
        let (agent, _rx) = make_agent("a1");
        mgr.insert(agent);
        assert_eq!(mgr.get_state("a1"), Some(SubAgentState::Running));

        // 2. Update output mid-run
        mgr.update_output("a1", "progress...".to_string());
        assert_eq!(mgr.get_output("a1"), Some("progress...".to_string()));
        assert_eq!(mgr.get_state("a1"), Some(SubAgentState::Running));

        // 3. Complete
        mgr.mark_completed("a1", "final result".to_string());
        assert_eq!(mgr.get_state("a1"), Some(SubAgentState::Completed));
        assert_eq!(mgr.get_output("a1"), Some("final result".to_string()));

        // 4. Resume
        let new_token = CancellationToken::new();
        let (new_tx, _new_rx) = mpsc::unbounded_channel::<String>();
        assert!(mgr.prepare_resume("a1", new_token, new_tx));
        assert_eq!(mgr.get_state("a1"), Some(SubAgentState::Running));
        assert_eq!(mgr.get_output("a1"), None); // cleared

        // 5. Fail this time
        mgr.mark_failed("a1", "crashed".to_string());
        assert_eq!(
            mgr.get_state("a1"),
            Some(SubAgentState::Failed("crashed".to_string()))
        );

        // 6. Resume again from failed
        let new_token2 = CancellationToken::new();
        let (new_tx2, _rx2) = mpsc::unbounded_channel::<String>();
        assert!(mgr.prepare_resume("a1", new_token2, new_tx2));
        assert_eq!(mgr.get_state("a1"), Some(SubAgentState::Running));
    }

    #[test]
    fn cancelled_cannot_resume() {
        let mgr = Arc::new(SubAgentManager::new());
        let (agent, _rx) = make_agent("a1");
        mgr.insert(agent);
        mgr.cancel("a1");

        let new_token = CancellationToken::new();
        let (new_tx, _rx2) = mpsc::unbounded_channel::<String>();
        assert!(!mgr.prepare_resume("a1", new_token, new_tx));
        assert_eq!(mgr.get_state("a1"), Some(SubAgentState::Cancelled));
    }

    #[tokio::test]
    async fn input_channel_survives_multiple_sends() {
        let mgr = Arc::new(SubAgentManager::new());
        let (agent, mut rx) = make_agent("a1");
        mgr.insert(agent);

        let tx = mgr.get_input_tx("a1").unwrap();
        for i in 0..100 {
            tx.send(format!("msg-{}", i)).unwrap();
        }

        for i in 0..100 {
            assert_eq!(rx.try_recv().unwrap(), format!("msg-{}", i));
        }
    }

    #[test]
    fn multiple_agents_independent_state() {
        let mgr = Arc::new(SubAgentManager::new());

        let (a1, _) = make_agent("a1");
        let (a2, _) = make_agent("a2");
        let (a3, _) = make_agent("a3");
        mgr.insert(a1);
        mgr.insert(a2);
        mgr.insert(a3);

        mgr.mark_completed("a1", "done-1".to_string());
        mgr.mark_failed("a2", "error-2".to_string());
        // a3 still running

        assert_eq!(mgr.get_state("a1"), Some(SubAgentState::Completed));
        assert_eq!(
            mgr.get_state("a2"),
            Some(SubAgentState::Failed("error-2".to_string()))
        );
        assert_eq!(mgr.get_state("a3"), Some(SubAgentState::Running));
    }

    #[test]
    fn remove_cleans_up_completely() {
        let mgr = Arc::new(SubAgentManager::new());
        let (agent, _rx) = make_agent("a1");
        let sid = agent.session_id;
        mgr.insert(agent);

        assert!(mgr.exists("a1"));
        assert_eq!(mgr.get_session_id("a1"), Some(sid));

        let removed = mgr.remove("a1").unwrap();
        assert_eq!(removed.id, "a1");
        assert_eq!(removed.session_id, sid);

        assert!(!mgr.exists("a1"));
        assert_eq!(mgr.get_state("a1"), None);
        assert_eq!(mgr.get_output("a1"), None);
        assert_eq!(mgr.get_session_id("a1"), None);
        assert!(mgr.get_input_tx("a1").is_none());
    }
}

// ─── AgentType Tests ───────────────────────────────────────────────────────

mod agent_type {
    use crate::brain::tools::subagent::AgentType;

    #[test]
    fn parse_known_types() {
        assert_eq!(AgentType::parse("explore"), AgentType::Explore);
        assert_eq!(AgentType::parse("search"), AgentType::Explore);
        assert_eq!(AgentType::parse("find"), AgentType::Explore);
        assert_eq!(AgentType::parse("plan"), AgentType::Plan);
        assert_eq!(AgentType::parse("architect"), AgentType::Plan);
        assert_eq!(AgentType::parse("code"), AgentType::Code);
        assert_eq!(AgentType::parse("implement"), AgentType::Code);
        assert_eq!(AgentType::parse("write"), AgentType::Code);
        assert_eq!(AgentType::parse("research"), AgentType::Research);
        assert_eq!(AgentType::parse("web"), AgentType::Research);
    }

    #[test]
    fn parse_unknown_defaults_to_general() {
        assert_eq!(AgentType::parse(""), AgentType::General);
        assert_eq!(AgentType::parse("foobar"), AgentType::General);
        assert_eq!(AgentType::parse("random"), AgentType::General);
    }

    #[test]
    fn parse_case_insensitive() {
        assert_eq!(AgentType::parse("EXPLORE"), AgentType::Explore);
        assert_eq!(AgentType::parse("Plan"), AgentType::Plan);
        assert_eq!(AgentType::parse("CODE"), AgentType::Code);
    }

    #[test]
    fn labels_are_lowercase() {
        assert_eq!(AgentType::General.label(), "general");
        assert_eq!(AgentType::Explore.label(), "explore");
        assert_eq!(AgentType::Plan.label(), "plan");
        assert_eq!(AgentType::Code.label(), "code");
        assert_eq!(AgentType::Research.label(), "research");
    }

    #[test]
    fn system_prompts_are_nonempty() {
        for agent_type in &[
            AgentType::General,
            AgentType::Explore,
            AgentType::Plan,
            AgentType::Code,
            AgentType::Research,
        ] {
            assert!(!agent_type.system_prompt().is_empty());
        }
    }

    /// Build a mock parent registry with all common tools for testing filtering.
    fn mock_parent_registry() -> crate::brain::tools::ToolRegistry {
        use std::sync::Arc;
        let reg = crate::brain::tools::ToolRegistry::new();
        reg.register(Arc::new(crate::brain::tools::read::ReadTool));
        reg.register(Arc::new(crate::brain::tools::write::WriteTool));
        reg.register(Arc::new(crate::brain::tools::edit::EditTool));
        reg.register(Arc::new(crate::brain::tools::bash::BashTool));
        reg.register(Arc::new(crate::brain::tools::glob::GlobTool));
        reg.register(Arc::new(crate::brain::tools::grep::GrepTool));
        reg.register(Arc::new(crate::brain::tools::ls::LsTool));
        reg.register(Arc::new(crate::brain::tools::web_search::WebSearchTool));
        reg
    }

    #[test]
    fn explore_registry_is_read_only() {
        let parent = mock_parent_registry();
        let registry = AgentType::Explore.build_registry(&parent);
        let tools = registry.list_tools();
        assert!(tools.contains(&"read_file".to_string()));
        assert!(tools.contains(&"glob".to_string()));
        assert!(tools.contains(&"grep".to_string()));
        assert!(tools.contains(&"ls".to_string()));
        assert!(!tools.contains(&"write_file".to_string()));
        assert!(!tools.contains(&"edit_file".to_string()));
        assert!(!tools.contains(&"bash".to_string()));
    }

    #[test]
    fn general_registry_inherits_full_parent() {
        let parent = mock_parent_registry();
        let registry = AgentType::General.build_registry(&parent);
        let tools = registry.list_tools();
        assert!(tools.contains(&"read_file".to_string()));
        assert!(tools.contains(&"write_file".to_string()));
        assert!(tools.contains(&"edit_file".to_string()));
        assert!(tools.contains(&"bash".to_string()));
        assert!(tools.contains(&"glob".to_string()));
        assert!(tools.contains(&"grep".to_string()));
    }

    #[test]
    fn general_registry_excludes_recursive_tools() {
        let parent = mock_parent_registry();
        // Add a "spawn_agent" to parent — it should be filtered out
        use std::sync::Arc;
        let mgr = Arc::new(crate::brain::tools::subagent::SubAgentManager::new());
        parent.register(Arc::new(
            crate::brain::tools::subagent::SpawnAgentTool::new(
                mgr.clone(),
                Arc::new(crate::brain::tools::ToolRegistry::new()),
            ),
        ));
        let registry = AgentType::General.build_registry(&parent);
        let tools = registry.list_tools();
        assert!(!tools.contains(&"spawn_agent".to_string()));
    }

    #[test]
    fn research_registry_has_web_no_write() {
        let parent = mock_parent_registry();
        let registry = AgentType::Research.build_registry(&parent);
        let tools = registry.list_tools();
        assert!(tools.contains(&"web_search".to_string()));
        assert!(tools.contains(&"read_file".to_string()));
        assert!(!tools.contains(&"write_file".to_string()));
        assert!(!tools.contains(&"edit_file".to_string()));
    }

    #[test]
    fn plan_registry_has_bash_for_analysis() {
        let parent = mock_parent_registry();
        let registry = AgentType::Plan.build_registry(&parent);
        let tools = registry.list_tools();
        assert!(tools.contains(&"bash".to_string()));
        assert!(tools.contains(&"read_file".to_string()));
        assert!(!tools.contains(&"write_file".to_string()));
    }
}
