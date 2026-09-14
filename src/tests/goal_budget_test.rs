//! Tests for goal turn budget config, creation, and exhaustion (#190).

use crate::brain::goal::GoalManager;
use crate::brain::goal::types::{DEFAULT_MAX_TURNS, GoalDecision};
use crate::brain::provider::Provider;
use crate::brain::tools::goal_manage::GoalManageTool;
use crate::brain::tools::r#trait::Tool;
use crate::config::AgentConfig;
use crate::db::Database;
use crate::services::ServiceContext;
use crate::tests::agent_service_mocks::MockProvider;
use std::sync::Arc;
use uuid::Uuid;

#[test]
fn config_goal_max_turns_parses_cleanly() {
    let toml_str = r#"
        approval_policy = "ask"
        goal_max_turns = 35
    "#;
    let cfg: AgentConfig = toml::from_str(toml_str).expect("should parse AgentConfig");
    assert_eq!(cfg.goal_max_turns, Some(35));

    let default_cfg = AgentConfig::default();
    assert_eq!(default_cfg.goal_max_turns, None);

    let serialized = toml::to_string(&default_cfg).expect("should serialize");
    assert!(!serialized.contains("goal_max_turns"));
}

#[test]
fn goal_manage_schema_exposes_max_turns() {
    let schema = GoalManageTool.input_schema();
    let max_turns_prop = &schema["properties"]["max_turns"];
    assert!(
        max_turns_prop.is_object(),
        "max_turns property missing in schema"
    );
    assert_eq!(max_turns_prop["type"].as_str(), Some("integer"));
}

#[tokio::test]
async fn goal_creation_defaults_to_20_when_unspecified() {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let ctx = ServiceContext::new(db.pool().clone());
    let goal_mgr = GoalManager::new(ctx);
    let sid = Uuid::new_v4();

    let goal = goal_mgr
        .set_goal(
            sid,
            "test goal without max_turns".to_string(),
            None,
            None,
            None,
        )
        .await
        .expect("set_goal should succeed");

    assert_eq!(goal.max_turns, DEFAULT_MAX_TURNS as i32);
    assert_eq!(goal.state, "active");
    assert_eq!(goal.turns_used, 0);

    let loaded = goal_mgr
        .get_goal(sid)
        .await
        .unwrap()
        .expect("goal must exist");
    assert_eq!(loaded.max_turns, DEFAULT_MAX_TURNS as i32);
}

#[tokio::test]
async fn goal_creation_respects_custom_max_turns() {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let ctx = ServiceContext::new(db.pool().clone());
    let goal_mgr = GoalManager::new(ctx);
    let sid = Uuid::new_v4();

    let goal = goal_mgr
        .set_goal(
            sid,
            "test goal with custom max_turns".to_string(),
            None,
            None,
            Some(50),
        )
        .await
        .expect("set_goal should succeed");

    assert_eq!(goal.max_turns, 50);

    let loaded = goal_mgr
        .get_goal(sid)
        .await
        .unwrap()
        .expect("goal must exist");
    assert_eq!(loaded.max_turns, 50);
}

#[tokio::test]
async fn goal_turn_exhaustion_pauses_and_updates_db_state() {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let ctx = ServiceContext::new(db.pool().clone());
    let goal_mgr = GoalManager::new(ctx.clone());
    let sid = Uuid::new_v4();

    // Set goal with a low turn budget of 2
    let goal = goal_mgr
        .set_goal(sid, "exhaust budget test".to_string(), None, None, Some(2))
        .await
        .expect("set_goal should succeed");
    assert_eq!(goal.max_turns, 2);

    // Increment turns to reach the budget (2/2)
    goal_mgr.increment_turns(sid, false).await.unwrap();
    goal_mgr.increment_turns(sid, false).await.unwrap();

    let before_eval = goal_mgr.get_goal(sid).await.unwrap().unwrap();
    assert_eq!(before_eval.turns_used, 2);
    assert_eq!(before_eval.state, "active");

    let provider: Arc<dyn Provider> = Arc::new(MockProvider);
    let decision = goal_mgr
        .evaluate_after_turn(provider.as_ref(), "mock-model", sid, "done with work")
        .await;

    match decision {
        GoalDecision::Paused { reason } => {
            assert!(
                reason.contains("exhausted"),
                "reason should mention exhausted, got: {reason}"
            );
            assert!(
                reason.contains("2/2"),
                "reason should mention turn count 2/2, got: {reason}"
            );
            assert!(
                reason.contains("paused"),
                "reason should mention paused, got: {reason}"
            );
        }
        other => panic!("expected GoalDecision::Paused, got: {:?}", other),
    }

    // Crucial check: DB state must be reconciled to "paused", NOT "failed"
    let after_eval = goal_mgr.get_goal(sid).await.unwrap().unwrap();
    assert_eq!(
        after_eval.state, "paused",
        "turn budget exhaustion state must be 'paused', not 'failed'"
    );
}
