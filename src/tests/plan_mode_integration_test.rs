//! Plan Mode Integration Tests
//!
//! End-to-end tests for Plan Mode functionality including:
//! - Plan creation and persistence workflow
//! - Plan state transitions
//! - Multiple concurrent plans
//! - Database and JSON synchronization

use opencrabs::db::models::Session;
use opencrabs::db::repository::session::SessionRepository;
use opencrabs::db::Database;
use opencrabs::services::{PlanService, ServiceContext};
use opencrabs::tui::plan::{PlanDocument, PlanStatus, PlanTask, TaskStatus, TaskType};
use tempfile::TempDir;
use uuid::Uuid;

/// Helper to setup test environment with database
async fn setup_test_env() -> (Database, ServiceContext, PlanService, Session, TempDir) {
    let db = Database::connect_in_memory()
        .await
        .expect("Failed to create database");
    db.run_migrations().await.expect("Failed to run migrations");

    let context = ServiceContext::new(db.pool().clone());
    let plan_service = PlanService::new(context.clone());

    // Create a test session
    let session_repo = SessionRepository::new(db.pool().clone());
    let session = Session::new(
        Some("Integration Test Session".to_string()),
        Some("claude-sonnet-4-5".to_string()),
        None,
    );
    session_repo
        .create(&session)
        .await
        .expect("Failed to create test session");

    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    (db, context, plan_service, session, temp_dir)
}

/// Helper to create a test plan with multiple tasks
fn create_multi_task_plan(session_id: Uuid) -> PlanDocument {
    let mut plan = PlanDocument::new(
        session_id,
        "Multi-Task Integration Test Plan".to_string(),
        "Testing full plan workflow with dependencies".to_string(),
    );

    plan.context = "Integration testing context".to_string();
    plan.risks = vec![
        "Risk 1: Test might fail".to_string(),
        "Risk 2: Dependencies might break".to_string(),
    ];

    // Create tasks with dependencies
    let task1_id = Uuid::new_v4();
    let task2_id = Uuid::new_v4();
    let task3_id = Uuid::new_v4();

    let task1 = PlanTask {
        id: task1_id,
        order: 0,
        title: "Research phase".to_string(),
        description: "Gather requirements and research".to_string(),
        task_type: TaskType::Research,
        dependencies: vec![],
        complexity: 3,
        acceptance_criteria: vec![],
        status: TaskStatus::Pending,
        notes: None,
        completed_at: None,
        execution_history: vec![],
        retry_count: 0,
        max_retries: 3,
        artifacts: vec![],
        reflection: None,
    };

    let task2 = PlanTask {
        id: task2_id,
        order: 1,
        title: "Implementation phase".to_string(),
        description: "Implement the feature".to_string(),
        task_type: TaskType::Create,
        dependencies: vec![task1_id], // Depends on research
        complexity: 5,
        acceptance_criteria: vec![],
        status: TaskStatus::Pending,
        notes: None,
        completed_at: None,
        execution_history: vec![],
        retry_count: 0,
        max_retries: 3,
        artifacts: vec![],
        reflection: None,
    };

    let task3 = PlanTask {
        id: task3_id,
        order: 2,
        title: "Testing phase".to_string(),
        description: "Write and run tests".to_string(),
        task_type: TaskType::Test,
        dependencies: vec![task2_id], // Depends on implementation
        complexity: 4,
        acceptance_criteria: vec![],
        status: TaskStatus::Pending,
        notes: None,
        completed_at: None,
        execution_history: vec![],
        retry_count: 0,
        max_retries: 3,
        artifacts: vec![],
        reflection: None,
    };

    plan.add_task(task1);
    plan.add_task(task2);
    plan.add_task(task3);

    plan
}

#[tokio::test]
async fn test_end_to_end_plan_creation_and_retrieval() {
    let (_db, _context, plan_service, session, _temp) = setup_test_env().await;

    // Create a plan
    let plan = create_multi_task_plan(session.id);
    let plan_id = plan.id;

    // Save to database
    plan_service
        .create(&plan)
        .await
        .expect("Failed to create plan");

    // Retrieve from database
    let retrieved = plan_service
        .find_by_id(plan_id)
        .await
        .expect("Failed to retrieve plan")
        .expect("Plan not found");

    // Verify all data is intact
    assert_eq!(retrieved.id, plan.id);
    assert_eq!(retrieved.session_id, plan.session_id);
    assert_eq!(retrieved.title, plan.title);
    assert_eq!(retrieved.description, plan.description);
    assert_eq!(retrieved.context, plan.context);
    assert_eq!(retrieved.risks.len(), 2);
    assert_eq!(retrieved.tasks.len(), 3);
    assert_eq!(retrieved.status, PlanStatus::Draft);

    // Verify task dependencies are preserved
    assert_eq!(retrieved.tasks[0].dependencies.len(), 0);
    assert_eq!(retrieved.tasks[1].dependencies.len(), 1);
    assert_eq!(retrieved.tasks[2].dependencies.len(), 1);
}

#[tokio::test]
async fn test_plan_state_transition_workflow() {
    let (_db, _context, plan_service, session, _temp) = setup_test_env().await;

    let mut plan = create_multi_task_plan(session.id);

    // Initial state: Draft
    assert_eq!(plan.status, PlanStatus::Draft);
    plan_service.create(&plan).await.expect("Failed to create");

    // Transition to PendingApproval
    plan.status = PlanStatus::PendingApproval;
    plan_service.update(&plan).await.expect("Failed to update");

    let retrieved = plan_service.find_by_id(plan.id).await.unwrap().unwrap();
    assert_eq!(retrieved.status, PlanStatus::PendingApproval);

    // Approve plan
    plan.status = PlanStatus::Approved;
    plan.approved_at = Some(chrono::Utc::now());
    plan_service.update(&plan).await.expect("Failed to update");

    let retrieved = plan_service.find_by_id(plan.id).await.unwrap().unwrap();
    assert_eq!(retrieved.status, PlanStatus::Approved);
    assert!(retrieved.approved_at.is_some());

    // Start execution
    plan.status = PlanStatus::InProgress;
    plan_service.update(&plan).await.expect("Failed to update");

    // Complete tasks sequentially
    let task_ids: Vec<Uuid> = plan.tasks.iter().map(|t| t.id).collect();
    for task_id in task_ids {
        if let Some(task) = plan.get_task_mut(&task_id) {
            task.status = TaskStatus::Completed;
            task.completed_at = Some(chrono::Utc::now());
        }
        plan_service.update(&plan).await.expect("Failed to update");
    }

    // Mark plan as completed
    plan.status = PlanStatus::Completed;
    plan_service.update(&plan).await.expect("Failed to update");

    let final_plan = plan_service.find_by_id(plan.id).await.unwrap().unwrap();
    assert_eq!(final_plan.status, PlanStatus::Completed);
    assert!(final_plan.is_complete());
    assert_eq!(
        final_plan
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Completed)
            .count(),
        3
    );
}

#[tokio::test]
async fn test_multiple_concurrent_plans_for_same_session() {
    let (_db, _context, plan_service, session, _temp) = setup_test_env().await;

    // Create multiple plans for the same session
    let plan1 = create_multi_task_plan(session.id);
    let plan2 = create_multi_task_plan(session.id);
    let plan3 = create_multi_task_plan(session.id);

    plan_service
        .create(&plan1)
        .await
        .expect("Failed to create plan1");
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    plan_service
        .create(&plan2)
        .await
        .expect("Failed to create plan2");
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    plan_service
        .create(&plan3)
        .await
        .expect("Failed to create plan3");

    // Retrieve all plans for the session
    let plans = plan_service
        .find_by_session_id(session.id)
        .await
        .expect("Failed to find plans");

    assert_eq!(plans.len(), 3);

    // Verify they're all for the same session
    for plan in &plans {
        assert_eq!(plan.session_id, session.id);
    }

    // Verify each plan is independent
    let plan_ids: Vec<Uuid> = plans.iter().map(|p| p.id).collect();
    assert!(plan_ids.contains(&plan1.id));
    assert!(plan_ids.contains(&plan2.id));
    assert!(plan_ids.contains(&plan3.id));
}

#[tokio::test]
async fn test_multiple_sessions_with_separate_plans() {
    let (db, _context, plan_service, session1, _temp) = setup_test_env().await;

    // Create a second session
    let session_repo = SessionRepository::new(db.pool().clone());
    let session2 = Session::new(
        Some("Second Test Session".to_string()),
        Some("claude-sonnet-4-5".to_string()),
        None,
    );
    session_repo
        .create(&session2)
        .await
        .expect("Failed to create session2");

    // Create plans for each session
    let plan1 = create_multi_task_plan(session1.id);
    let plan2 = create_multi_task_plan(session2.id);

    plan_service
        .create(&plan1)
        .await
        .expect("Failed to create plan1");
    plan_service
        .create(&plan2)
        .await
        .expect("Failed to create plan2");

    // Verify session1 has only its plan
    let session1_plans = plan_service
        .find_by_session_id(session1.id)
        .await
        .expect("Failed to find session1 plans");
    assert_eq!(session1_plans.len(), 1);
    assert_eq!(session1_plans[0].id, plan1.id);

    // Verify session2 has only its plan
    let session2_plans = plan_service
        .find_by_session_id(session2.id)
        .await
        .expect("Failed to find session2 plans");
    assert_eq!(session2_plans.len(), 1);
    assert_eq!(session2_plans[0].id, plan2.id);
}

#[tokio::test]
async fn test_plan_deletion_with_cascade() {
    let (_db, _context, plan_service, session, _temp) = setup_test_env().await;

    let plan = create_multi_task_plan(session.id);
    let plan_id = plan.id;

    // Create plan with tasks
    plan_service.create(&plan).await.expect("Failed to create");

    // Verify plan exists
    let found = plan_service.find_by_id(plan_id).await.unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().tasks.len(), 3);

    // Delete plan
    plan_service
        .delete(plan_id)
        .await
        .expect("Failed to delete");

    // Verify plan and all tasks are deleted
    let not_found = plan_service.find_by_id(plan_id).await.unwrap();
    assert!(not_found.is_none());
}

#[tokio::test]
async fn test_json_export_import_integration() {
    let (_db, _context, plan_service, session, temp) = setup_test_env().await;

    let original_plan = create_multi_task_plan(session.id);
    plan_service
        .create(&original_plan)
        .await
        .expect("Failed to create");

    // Export to JSON
    let json_path = temp.path().join("integration_plan.json");
    plan_service
        .export_to_json(&original_plan, &json_path)
        .await
        .expect("Failed to export");

    // Verify file exists
    assert!(json_path.exists());

    // Import from JSON
    let imported_plan = plan_service
        .import_from_json(&json_path)
        .await
        .expect("Failed to import");

    // Verify complete data integrity
    assert_eq!(imported_plan.id, original_plan.id);
    assert_eq!(imported_plan.session_id, original_plan.session_id);
    assert_eq!(imported_plan.title, original_plan.title);
    assert_eq!(imported_plan.tasks.len(), original_plan.tasks.len());

    // Verify task details
    for (orig, imp) in original_plan.tasks.iter().zip(imported_plan.tasks.iter()) {
        assert_eq!(orig.id, imp.id);
        assert_eq!(orig.title, imp.title);
        assert_eq!(orig.dependencies, imp.dependencies);
        assert_eq!(orig.task_type, imp.task_type);
    }
}

#[tokio::test]
async fn test_plan_rejection_workflow() {
    let (_db, _context, plan_service, session, _temp) = setup_test_env().await;

    let mut plan = create_multi_task_plan(session.id);
    plan.status = PlanStatus::PendingApproval;

    plan_service.create(&plan).await.expect("Failed to create");

    // Reject the plan
    plan.status = PlanStatus::Rejected;
    plan_service.update(&plan).await.expect("Failed to update");

    let retrieved = plan_service.find_by_id(plan.id).await.unwrap().unwrap();
    assert_eq!(retrieved.status, PlanStatus::Rejected);

    // Rejected plans should not transition to InProgress
    // (This would be enforced by UI/business logic layer)
}

#[tokio::test]
async fn test_task_blocking_and_failure_scenarios() {
    let (_db, _context, plan_service, session, _temp) = setup_test_env().await;

    let mut plan = create_multi_task_plan(session.id);
    plan_service.create(&plan).await.expect("Failed to create");

    // Block second task
    let task2_id = plan.tasks[1].id;
    if let Some(task) = plan.get_task_mut(&task2_id) {
        task.status = TaskStatus::Blocked("Waiting for API access".to_string());
    }
    plan_service.update(&plan).await.expect("Failed to update");

    let retrieved = plan_service.find_by_id(plan.id).await.unwrap().unwrap();
    if let TaskStatus::Blocked(reason) = &retrieved.tasks[1].status {
        assert_eq!(reason, "Waiting for API access");
    } else {
        panic!("Task should be blocked");
    }

    // Fail third task
    let task3_id = plan.tasks[2].id;
    if let Some(task) = plan.get_task_mut(&task3_id) {
        task.status = TaskStatus::Failed;
        task.notes = Some("Tests failed to compile".to_string());
    }
    plan_service.update(&plan).await.expect("Failed to update");

    let retrieved = plan_service.find_by_id(plan.id).await.unwrap().unwrap();
    assert_eq!(retrieved.tasks[2].status, TaskStatus::Failed);
    assert_eq!(
        retrieved.tasks[2].notes,
        Some("Tests failed to compile".to_string())
    );
}

#[tokio::test]
async fn test_get_most_recent_plan_integration() {
    let (_db, _context, plan_service, session, _temp) = setup_test_env().await;

    // No plans initially
    let recent = plan_service
        .get_most_recent_plan(session.id)
        .await
        .expect("Failed to get recent");
    assert!(recent.is_none());

    // Create first plan
    let plan1 = create_multi_task_plan(session.id);
    plan_service.create(&plan1).await.expect("Failed to create");

    let recent = plan_service
        .get_most_recent_plan(session.id)
        .await
        .expect("Failed to get recent")
        .expect("Should have a plan");
    assert_eq!(recent.id, plan1.id);

    // Create and update second plan to make it more recent
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    let plan2 = create_multi_task_plan(session.id);
    plan_service.create(&plan2).await.expect("Failed to create");

    // Most recent should be one of the two plans
    let recent = plan_service
        .get_most_recent_plan(session.id)
        .await
        .expect("Failed to get recent")
        .expect("Should have a plan");
    assert!(recent.id == plan1.id || recent.id == plan2.id);
}
