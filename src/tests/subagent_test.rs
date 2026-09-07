//! Sub-Agent / Swarm System Tests
//!
//! Covers SubAgentManager state machine, all 8 tool operations
//! (spawn, wait, send_input, close, resume, team_create, team_delete, team_broadcast),
//! lifecycle transitions, input channel wiring, cancellation, team orchestration,
//! and concurrent access.

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
            input_tx: Some(tx),
            ..SubAgent::new(
                id.to_string(),
                label.to_string(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            )
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
    fn alive_counts_for_splits_working_and_awaiting_per_parent() {
        // #1183: the settle card counts only THIS session's children, split
        // working vs parked-awaiting-collection. The manager is process-global
        // (one instance wired into every channel agent), so an unfiltered
        // count would report another chat's fan-out as this chat's pending
        // work. Terminal agents never count.
        let mgr = SubAgentManager::new();
        let parent = Uuid::new_v4();
        let other_chat = Uuid::new_v4();

        let mut working = make_agent("w1", "working");
        working.parent_session_id = parent;
        let mut parked = make_agent("p1", "parked");
        parked.parent_session_id = parent;
        let mut done = make_agent("d1", "done");
        done.parent_session_id = parent;
        let mut foreign = make_agent("f1", "other-chat");
        foreign.parent_session_id = other_chat;

        mgr.insert(working);
        mgr.insert(parked);
        mgr.insert(done);
        mgr.insert(foreign);

        mgr.mark_awaiting_input("p1");
        mgr.mark_completed("d1", "finished".to_string());

        assert_eq!(mgr.alive_counts_for(parent), (1, 1));
        assert_eq!(mgr.alive_counts_for(other_chat), (1, 0));
        assert_eq!(mgr.alive_counts_for(Uuid::new_v4()), (0, 0));

        // Follow-up input flips the parked agent back to working: the split
        // must track the state machine, not a static snapshot.
        mgr.mark_running_again("p1");
        assert_eq!(mgr.alive_counts_for(parent), (2, 0));
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
            input_tx: Some(tx),
            ..SubAgent::new(
                "a1".to_string(),
                "test".to_string(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            )
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
    use crate::brain::tools::subagent::{SubAgent, SubAgentManager};
    use crate::brain::tools::{Tool, ToolExecutionContext};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use uuid::Uuid;

    fn test_context() -> ToolExecutionContext {
        ToolExecutionContext {
            session_provider: None,
            session_id: Uuid::new_v4(),
            working_directory: std::path::PathBuf::from("/tmp"),
            env_vars: HashMap::new(),
            auto_approve: true,
            timeout_secs: 30,
            sudo_callback: None,
            ssh_callback: None,
            shared_working_directory: None,
            service_context: None,
            progress_callback: None,
            background_manager: None,
            plan_session_override: None,
            subagent_manager: None,
            parent_tool_registry: None,
            headless: false,
        }
    }

    fn make_running_agent(id: &str) -> (SubAgent, mpsc::UnboundedReceiver<String>) {
        let (tx, rx) = mpsc::unbounded_channel::<String>();
        let agent = SubAgent {
            input_tx: Some(tx),
            ..SubAgent::new(
                id.to_string(),
                "test".to_string(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            )
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
        // #1184 ripple: completed agents are pointed at resume_agent instead of
        // the old generic "not running" rejection.
        assert!(
            result
                .error
                .as_ref()
                .unwrap()
                .contains("use resume_agent to continue")
        );
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
            input_tx: Some(tx),
            ..SubAgent::new(
                "a1".to_string(),
                "test".to_string(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            )
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
    use uuid::Uuid;

    fn test_context() -> ToolExecutionContext {
        ToolExecutionContext {
            session_provider: None,
            session_id: Uuid::new_v4(),
            working_directory: std::path::PathBuf::from("/tmp"),
            env_vars: HashMap::new(),
            auto_approve: true,
            timeout_secs: 30,
            sudo_callback: None,
            ssh_callback: None,
            shared_working_directory: None,
            service_context: None,
            progress_callback: None,
            background_manager: None,
            plan_session_override: None,
            subagent_manager: None,
            parent_tool_registry: None,
            headless: false,
        }
    }

    fn make_running_agent(id: &str) -> SubAgent {
        let (tx, _rx) = mpsc::unbounded_channel::<String>();
        SubAgent {
            input_tx: Some(tx),
            ..SubAgent::new(
                id.to_string(),
                "test".to_string(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            )
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
    use crate::brain::tools::subagent::{SubAgent, SubAgentManager};
    use crate::brain::tools::{Tool, ToolExecutionContext};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use uuid::Uuid;

    fn test_context() -> ToolExecutionContext {
        ToolExecutionContext {
            session_provider: None,
            session_id: Uuid::new_v4(),
            working_directory: std::path::PathBuf::from("/tmp"),
            env_vars: HashMap::new(),
            auto_approve: true,
            timeout_secs: 30,
            sudo_callback: None,
            ssh_callback: None,
            shared_working_directory: None,
            service_context: None,
            progress_callback: None,
            background_manager: None,
            plan_session_override: None,
            subagent_manager: None,
            parent_tool_registry: None,
            headless: false,
        }
    }

    fn make_running_agent(id: &str) -> SubAgent {
        let (tx, _rx) = mpsc::unbounded_channel::<String>();
        SubAgent {
            input_tx: Some(tx),
            ..SubAgent::new(
                id.to_string(),
                "test".to_string(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            )
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
    async fn wait_timeout_returns_progress_not_error() {
        // New semantics: on timeout we return success with a progress
        // preview (or "No output yet.") so the LLM sees work-in-flight
        // and can keep polling. The old "Timed out" error wording drove
        // LLMs to abandon the parent turn after a few retries — replaced
        // by "still running after {secs}s" success messaging.
        let mgr = Arc::new(SubAgentManager::new());
        mgr.insert(make_running_agent("a1"));

        let tool = WaitAgentTool::new(mgr);
        let ctx = test_context();

        let result = tool
            .execute(json!({"agent_id": "a1", "timeout_secs": 1}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("still running"));
    }

    #[tokio::test]
    async fn wait_returns_round_output_on_awaiting_input() {
        // Replaces the old no-handle test. The real fix for the
        // 2026-04-17 deadlock: sub-agents transition to AwaitingInput
        // at a round boundary, and wait_agent returns immediately with
        // the buffered round output instead of blocking on task-join
        // (which never resolves between rounds).
        let mgr = Arc::new(SubAgentManager::new());
        mgr.insert(make_running_agent("a1"));
        mgr.update_output("a1", "round 1 results go here".to_string());
        mgr.mark_awaiting_input("a1");

        let tool = WaitAgentTool::new(mgr);
        let ctx = test_context();

        let result = tool
            .execute(json!({"agent_id": "a1", "timeout_secs": 30}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("paused for input"));
        assert!(result.output.contains("round 1 results"));
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
            input_tx: Some(tx),
            ..SubAgent::new(
                id.to_string(),
                "lifecycle-test".to_string(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            )
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

// ─── Child Registry & Grant Tests (#1173) ─────────────────────────────────

mod agent_type {
    use crate::brain::tools::subagent::map_deprecated_agent_type;

    /// Build a mock parent registry with common tools for filtering tests.
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
        reg.register(Arc::new(
            crate::brain::tools::web_search::WebSearchTool::default(),
        ));
        reg
    }

    #[test]
    fn child_registry_inherits_full_parent() {
        let parent = mock_parent_registry();
        let registry = crate::brain::tools::subagent::build_child_registry(&parent);
        let tools = registry.list_tools();
        assert!(tools.contains(&"read_file".to_string()));
        assert!(tools.contains(&"write_file".to_string()));
        assert!(tools.contains(&"edit_file".to_string()));
        assert!(tools.contains(&"bash".to_string()));
        assert!(tools.contains(&"glob".to_string()));
        assert!(tools.contains(&"grep".to_string()));
    }

    #[test]
    fn child_registry_excludes_recursive_tools() {
        use std::sync::Arc;
        let parent = mock_parent_registry();
        let mgr = Arc::new(crate::brain::tools::subagent::SubAgentManager::new());
        parent.register(Arc::new(
            crate::brain::tools::subagent::SpawnAgentTool::new(
                mgr.clone(),
                Arc::new(crate::brain::tools::ToolRegistry::new()),
            ),
        ));
        let registry = crate::brain::tools::subagent::build_child_registry(&parent);
        let tools = registry.list_tools();
        assert!(
            !tools.contains(&"spawn_agent".to_string()),
            "recursive spawn must never reach a child"
        );
    }

    #[test]
    fn child_registry_excludes_team_tools() {
        use std::sync::Arc;
        let parent = mock_parent_registry();
        let subagent_mgr = Arc::new(crate::brain::tools::subagent::SubAgentManager::new());
        let team_mgr = Arc::new(crate::brain::tools::subagent::TeamManager::new());
        parent.register(Arc::new(
            crate::brain::tools::subagent::TeamCreateTool::new(
                subagent_mgr.clone(),
                team_mgr.clone(),
                Arc::new(crate::brain::tools::ToolRegistry::new()),
            ),
        ));
        parent.register(Arc::new(
            crate::brain::tools::subagent::TeamDeleteTool::new(
                subagent_mgr.clone(),
                team_mgr.clone(),
            ),
        ));
        parent.register(Arc::new(
            crate::brain::tools::subagent::TeamBroadcastTool::new(
                subagent_mgr.clone(),
                team_mgr.clone(),
            ),
        ));

        let registry = crate::brain::tools::subagent::build_child_registry(&parent);
        let tools = registry.list_tools();
        assert!(!tools.contains(&"team_create".to_string()));
        assert!(!tools.contains(&"team_delete".to_string()));
        assert!(!tools.contains(&"team_broadcast".to_string()));
    }

    #[test]
    fn deprecated_read_only_types_map_true() {
        for s in [
            "explore",
            "search",
            "find",
            "research",
            "web",
            "lookup",
            "EXPLORE",
            "  research  ",
        ] {
            assert_eq!(map_deprecated_agent_type(s), Ok(true), "input: {s}");
        }
    }

    #[test]
    fn deprecated_write_capable_types_map_false() {
        // `plan` carried bash historically, so it was NEVER read-only.
        for s in [
            "general",
            "plan",
            "architect",
            "design",
            "code",
            "implement",
            "write",
        ] {
            assert_eq!(map_deprecated_agent_type(s), Ok(false), "input: {s}");
        }
    }

    #[test]
    fn unknown_type_fails_closed() {
        for s in ["", "foobar", "Plan-X", "explor"] {
            let err = map_deprecated_agent_type(s).unwrap_err();
            assert!(err.contains("Unknown agent_type"), "input: {s}: {err}");
        }
    }

    #[test]
    fn restricted_registry_strips_mutating_tools() {
        let parent = mock_parent_registry();
        let reg = crate::brain::tools::subagent::build_child_registry(&parent);
        crate::brain::tools::plan_gate::restrict_registry_to_read_only(&reg);
        let tools = reg.list_tools();
        assert!(tools.contains(&"read_file".to_string()));
        assert!(tools.contains(&"glob".to_string()));
        assert!(!tools.contains(&"write_file".to_string()));
        assert!(!tools.contains(&"edit_file".to_string()));
        assert!(!tools.contains(&"bash".to_string()));
    }

    #[test]
    fn read_only_grant_frozen_in_manager() {
        use crate::brain::tools::subagent::{SubAgent, SubAgentManager};
        use uuid::Uuid;
        let mgr = SubAgentManager::new();
        mgr.insert(SubAgent {
            read_only: true,
            input_tx: None,
            ..SubAgent::new(
                "ro1".to_string(),
                "ro".to_string(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            )
        });
        assert_eq!(mgr.get_read_only("ro1"), Some(true));
        assert_eq!(mgr.get_read_only("missing"), None);
    }
}

// ─── TeamManager Tests ──────────────────────────────────────────────────────

mod team_manager {
    use crate::brain::tools::subagent::TeamManager;

    #[test]
    fn new_manager_is_empty() {
        let mgr = TeamManager::new();
        assert!(mgr.list_teams().is_empty());
    }

    #[test]
    fn default_creates_empty_manager() {
        let mgr = TeamManager::default();
        assert!(mgr.list_teams().is_empty());
    }

    #[test]
    fn create_team_succeeds() {
        let mgr = TeamManager::new();
        assert!(mgr.create_team(
            "alpha".to_string(),
            vec!["a1".to_string(), "a2".to_string()]
        ));
        assert!(mgr.exists("alpha"));
    }

    #[test]
    fn create_duplicate_team_fails() {
        let mgr = TeamManager::new();
        assert!(mgr.create_team("alpha".to_string(), vec!["a1".to_string()]));
        assert!(!mgr.create_team("alpha".to_string(), vec!["a2".to_string()]));
    }

    #[test]
    fn get_agent_ids_returns_correct_ids() {
        let mgr = TeamManager::new();
        mgr.create_team(
            "alpha".to_string(),
            vec!["a1".to_string(), "a2".to_string(), "a3".to_string()],
        );

        let ids = mgr.get_agent_ids("alpha").unwrap();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&"a1".to_string()));
        assert!(ids.contains(&"a2".to_string()));
        assert!(ids.contains(&"a3".to_string()));
    }

    #[test]
    fn get_agent_ids_missing_returns_none() {
        let mgr = TeamManager::new();
        assert!(mgr.get_agent_ids("ghost").is_none());
    }

    #[test]
    fn delete_team_removes_it() {
        let mgr = TeamManager::new();
        mgr.create_team("alpha".to_string(), vec!["a1".to_string()]);

        let team = mgr.delete_team("alpha");
        assert!(team.is_some());
        assert_eq!(team.unwrap().name, "alpha");
        assert!(!mgr.exists("alpha"));
    }

    #[test]
    fn delete_nonexistent_returns_none() {
        let mgr = TeamManager::new();
        assert!(mgr.delete_team("ghost").is_none());
    }

    #[test]
    fn list_teams_returns_names_and_counts() {
        let mgr = TeamManager::new();
        mgr.create_team(
            "alpha".to_string(),
            vec!["a1".to_string(), "a2".to_string()],
        );
        mgr.create_team("beta".to_string(), vec!["b1".to_string()]);

        let list = mgr.list_teams();
        assert_eq!(list.len(), 2);

        let names: Vec<&str> = list.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"beta"));
    }

    #[test]
    fn exists_returns_false_for_missing() {
        let mgr = TeamManager::new();
        assert!(!mgr.exists("ghost"));
    }

    #[test]
    fn concurrent_team_creation() {
        use std::sync::Arc;
        use std::thread;

        let mgr = Arc::new(TeamManager::new());
        let mut handles = vec![];

        for i in 0..10 {
            let mgr = mgr.clone();
            handles.push(thread::spawn(move || {
                mgr.create_team(format!("team-{}", i), vec![format!("agent-{}", i)]);
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(mgr.list_teams().len(), 10);
    }
}

// ─── TeamDeleteTool Tests ───────────────────────────────────────────────────

mod team_delete_tool {
    use crate::brain::tools::subagent::{SubAgent, SubAgentManager, TeamDeleteTool, TeamManager};
    use crate::brain::tools::{Tool, ToolExecutionContext};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use uuid::Uuid;

    fn test_context() -> ToolExecutionContext {
        ToolExecutionContext {
            session_provider: None,
            session_id: Uuid::new_v4(),
            working_directory: std::path::PathBuf::from("/tmp"),
            env_vars: HashMap::new(),
            auto_approve: true,
            timeout_secs: 30,
            sudo_callback: None,
            ssh_callback: None,
            shared_working_directory: None,
            service_context: None,
            progress_callback: None,
            background_manager: None,
            plan_session_override: None,
            subagent_manager: None,
            parent_tool_registry: None,
            headless: false,
        }
    }

    fn make_running_agent(id: &str) -> SubAgent {
        let (tx, _rx) = mpsc::unbounded_channel::<String>();
        SubAgent {
            input_tx: Some(tx),
            ..SubAgent::new(
                id.to_string(),
                "test".to_string(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            )
        }
    }

    #[tokio::test]
    async fn missing_team_name_returns_error() {
        let subagent_mgr = Arc::new(SubAgentManager::new());
        let team_mgr = Arc::new(TeamManager::new());
        let tool = TeamDeleteTool::new(subagent_mgr, team_mgr);
        let ctx = test_context();

        let result = tool.execute(json!({}), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn delete_nonexistent_team_returns_error() {
        let subagent_mgr = Arc::new(SubAgentManager::new());
        let team_mgr = Arc::new(TeamManager::new());
        let tool = TeamDeleteTool::new(subagent_mgr, team_mgr);
        let ctx = test_context();

        let result = tool.execute(json!({"team_name": "ghost"}), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn delete_team_cancels_running_agents() {
        let subagent_mgr = Arc::new(SubAgentManager::new());
        let team_mgr = Arc::new(TeamManager::new());

        // Insert agents
        let a1 = make_running_agent("a1");
        let a1_token = a1.cancel_token.clone();
        let a2 = make_running_agent("a2");
        let a2_token = a2.cancel_token.clone();
        subagent_mgr.insert(a1);
        subagent_mgr.insert(a2);

        // Create team
        team_mgr.create_team(
            "test-team".to_string(),
            vec!["a1".to_string(), "a2".to_string()],
        );

        let tool = TeamDeleteTool::new(subagent_mgr.clone(), team_mgr.clone());
        let ctx = test_context();

        let result = tool
            .execute(json!({"team_name": "test-team"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("2 agents cancelled"));
        assert!(a1_token.is_cancelled());
        assert!(a2_token.is_cancelled());
        assert!(!team_mgr.exists("test-team"));
    }

    #[tokio::test]
    async fn delete_team_with_completed_agents() {
        let subagent_mgr = Arc::new(SubAgentManager::new());
        let team_mgr = Arc::new(TeamManager::new());

        subagent_mgr.insert(make_running_agent("a1"));
        subagent_mgr.insert(make_running_agent("a2"));
        subagent_mgr.mark_completed("a2", "done".to_string());

        team_mgr.create_team(
            "test-team".to_string(),
            vec!["a1".to_string(), "a2".to_string()],
        );

        let tool = TeamDeleteTool::new(subagent_mgr.clone(), team_mgr.clone());
        let ctx = test_context();

        let result = tool
            .execute(json!({"team_name": "test-team"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("1 agents cancelled"));
        assert!(result.output.contains("1 already completed"));
    }
}

// ─── TeamBroadcastTool Tests ────────────────────────────────────────────────

mod team_broadcast_tool {
    use crate::brain::tools::subagent::{
        SubAgent, SubAgentManager, TeamBroadcastTool, TeamManager,
    };
    use crate::brain::tools::{Tool, ToolExecutionContext};
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use uuid::Uuid;

    fn test_context() -> ToolExecutionContext {
        ToolExecutionContext {
            session_provider: None,
            session_id: Uuid::new_v4(),
            working_directory: std::path::PathBuf::from("/tmp"),
            env_vars: HashMap::new(),
            auto_approve: true,
            timeout_secs: 30,
            sudo_callback: None,
            ssh_callback: None,
            shared_working_directory: None,
            service_context: None,
            progress_callback: None,
            background_manager: None,
            plan_session_override: None,
            subagent_manager: None,
            parent_tool_registry: None,
            headless: false,
        }
    }

    fn make_agent_with_channel(id: &str) -> (SubAgent, mpsc::UnboundedReceiver<String>) {
        let (tx, rx) = mpsc::unbounded_channel::<String>();
        let agent = SubAgent {
            input_tx: Some(tx),
            ..SubAgent::new(
                id.to_string(),
                "test".to_string(),
                Uuid::new_v4(),
                Uuid::new_v4(),
            )
        };
        (agent, rx)
    }

    #[tokio::test]
    async fn missing_team_name_returns_error() {
        let subagent_mgr = Arc::new(SubAgentManager::new());
        let team_mgr = Arc::new(TeamManager::new());
        let tool = TeamBroadcastTool::new(subagent_mgr, team_mgr);
        let ctx = test_context();

        let result = tool.execute(json!({"message": "hi"}), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn missing_message_returns_error() {
        let subagent_mgr = Arc::new(SubAgentManager::new());
        let team_mgr = Arc::new(TeamManager::new());
        let tool = TeamBroadcastTool::new(subagent_mgr, team_mgr);
        let ctx = test_context();

        let result = tool.execute(json!({"team_name": "alpha"}), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn broadcast_to_nonexistent_team_returns_error() {
        let subagent_mgr = Arc::new(SubAgentManager::new());
        let team_mgr = Arc::new(TeamManager::new());
        let tool = TeamBroadcastTool::new(subagent_mgr, team_mgr);
        let ctx = test_context();

        let result = tool
            .execute(json!({"team_name": "ghost", "message": "hi"}), &ctx)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn broadcast_delivers_to_all_running_agents() {
        let subagent_mgr = Arc::new(SubAgentManager::new());
        let team_mgr = Arc::new(TeamManager::new());

        let (a1, mut rx1) = make_agent_with_channel("a1");
        let (a2, mut rx2) = make_agent_with_channel("a2");
        subagent_mgr.insert(a1);
        subagent_mgr.insert(a2);

        team_mgr.create_team(
            "alpha".to_string(),
            vec!["a1".to_string(), "a2".to_string()],
        );

        let tool = TeamBroadcastTool::new(subagent_mgr.clone(), team_mgr.clone());
        let ctx = test_context();

        let result = tool
            .execute(json!({"team_name": "alpha", "message": "sync up"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("2 agents received"));

        assert_eq!(rx1.try_recv().unwrap(), "sync up");
        assert_eq!(rx2.try_recv().unwrap(), "sync up");
    }

    #[tokio::test]
    async fn broadcast_skips_completed_agents() {
        let subagent_mgr = Arc::new(SubAgentManager::new());
        let team_mgr = Arc::new(TeamManager::new());

        let (a1, mut rx1) = make_agent_with_channel("a1");
        let (a2, _rx2) = make_agent_with_channel("a2");
        subagent_mgr.insert(a1);
        subagent_mgr.insert(a2);
        subagent_mgr.mark_completed("a2", "done".to_string());

        team_mgr.create_team(
            "alpha".to_string(),
            vec!["a1".to_string(), "a2".to_string()],
        );

        let tool = TeamBroadcastTool::new(subagent_mgr.clone(), team_mgr.clone());
        let ctx = test_context();

        let result = tool
            .execute(json!({"team_name": "alpha", "message": "update"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("1 agents received"));
        assert!(result.output.contains("1 skipped"));

        assert_eq!(rx1.try_recv().unwrap(), "update");
    }
}
