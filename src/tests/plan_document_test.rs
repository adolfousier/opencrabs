//! Tests for `tui::plan` — PlanDocument, PlanTask, PlanStatus, and dependency resolution.

use uuid::Uuid;

use crate::tui::plan::*;

// ── PlanDocument creation ───────────────────────────────────────

#[test]
fn new_plan_has_draft_status() {
    let session_id = Uuid::new_v4();
    let plan = PlanDocument::new(session_id, "Test Plan".to_string(), "Desc".to_string());
    assert_eq!(plan.status, PlanStatus::Draft);
    assert_eq!(plan.title, "Test Plan");
    assert_eq!(plan.description, "Desc");
    assert_eq!(plan.session_id, session_id);
    assert!(plan.tasks.is_empty());
    assert!(plan.approved_at.is_none());
}

#[test]
fn new_plan_has_unique_id() {
    let s = Uuid::new_v4();
    let p1 = PlanDocument::new(s, "A".to_string(), "".to_string());
    let p2 = PlanDocument::new(s, "B".to_string(), "".to_string());
    assert_ne!(p1.id, p2.id);
}

// ── add_task ────────────────────────────────────────────────────

#[test]
fn add_task_appends() {
    let mut plan = PlanDocument::new(Uuid::new_v4(), "P".to_string(), "D".to_string());
    let task = PlanTask::new(1, "Task 1".to_string(), "Do something".to_string(), TaskType::Create);
    plan.add_task(task);
    assert_eq!(plan.tasks.len(), 1);
    assert_eq!(plan.tasks[0].title, "Task 1");
}

#[test]
fn add_task_updates_timestamp() {
    let mut plan = PlanDocument::new(Uuid::new_v4(), "P".to_string(), "D".to_string());
    let before = plan.updated_at;
    std::thread::sleep(std::time::Duration::from_millis(10));
    plan.add_task(PlanTask::new(1, "T".to_string(), "D".to_string(), TaskType::Create));
    assert!(plan.updated_at >= before);
}

#[test]
fn add_multiple_tasks() {
    let mut plan = PlanDocument::new(Uuid::new_v4(), "P".to_string(), "D".to_string());
    for i in 0..5 {
        plan.add_task(PlanTask::new(i, format!("Task {i}"), format!("Desc {i}"), TaskType::Create));
    }
    assert_eq!(plan.tasks.len(), 5);
}

// ── PlanTask ────────────────────────────────────────────────────

#[test]
fn new_task_is_pending() {
    let task = PlanTask::new(1, "T".to_string(), "D".to_string(), TaskType::Create);
    assert_eq!(task.status, TaskStatus::Pending);
    assert_eq!(task.title, "T");
    assert_eq!(task.description, "D");
    assert!(task.dependencies.is_empty());
    assert!(task.completed_at.is_none());
}

#[test]
fn task_has_unique_id() {
    let t1 = PlanTask::new(1, "A".to_string(), "".to_string(), TaskType::Create);
    let t2 = PlanTask::new(2, "B".to_string(), "".to_string(), TaskType::Create);
    assert_ne!(t1.id, t2.id);
}

// ── PlanStatus ──────────────────────────────────────────────────

#[test]
fn plan_status_eq() {
    assert_eq!(PlanStatus::Draft, PlanStatus::Draft);
    assert_eq!(PlanStatus::InProgress, PlanStatus::InProgress);
    assert_eq!(PlanStatus::Completed, PlanStatus::Completed);
    assert_eq!(PlanStatus::Cancelled, PlanStatus::Cancelled);
    assert_ne!(PlanStatus::Draft, PlanStatus::InProgress);
}

// ── TaskStatus ──────────────────────────────────────────────────

#[test]
fn task_status_eq() {
    assert_eq!(TaskStatus::Pending, TaskStatus::Pending);
    assert_eq!(TaskStatus::InProgress, TaskStatus::InProgress);
    assert_eq!(TaskStatus::Completed, TaskStatus::Completed);
    assert_eq!(TaskStatus::Skipped, TaskStatus::Skipped);
    assert_ne!(TaskStatus::Pending, TaskStatus::Completed);
}

// ── topological_sort (tasks_in_order) ───────────────────────

#[test]
fn tasks_in_order_no_deps() {
    let mut plan = PlanDocument::new(Uuid::new_v4(), "P".to_string(), "D".to_string());
    let t1 = PlanTask::new(1, "A".to_string(), "".to_string(), TaskType::Create);
    let t2 = PlanTask::new(2, "B".to_string(), "".to_string(), TaskType::Create);
    plan.add_task(t1);
    plan.add_task(t2);
    let ordered = plan.tasks_in_order();
    assert!(ordered.is_some());
    assert_eq!(ordered.unwrap().len(), 2);
}

#[test]
fn tasks_with_linear_deps() {
    let mut plan = PlanDocument::new(Uuid::new_v4(), "P".to_string(), "D".to_string());
    let t1 = PlanTask::new(1, "First".to_string(), "".to_string(), TaskType::Create);
    let t1_id = t1.id;
    let mut t2 = PlanTask::new(2, "Second".to_string(), "".to_string(), TaskType::Create);
    t2.dependencies.push(t1_id);
    plan.add_task(t1);
    plan.add_task(t2);

    let ordered = plan.tasks_in_order().unwrap();
    assert_eq!(ordered.len(), 2);
    // First task should come before second
    assert_eq!(ordered[0].title, "First");
    assert_eq!(ordered[1].title, "Second");
}

// ── Serialization ───────────────────────────────────────────────

#[test]
fn plan_document_serializes_to_json() {
    let plan = PlanDocument::new(Uuid::new_v4(), "Test".to_string(), "Desc".to_string());
    let json = serde_json::to_string(&plan).unwrap();
    assert!(json.contains("Test"));
    assert!(json.contains("Draft"));
}

#[test]
fn plan_document_round_trip() {
    let mut plan = PlanDocument::new(Uuid::new_v4(), "Round Trip".to_string(), "D".to_string());
    plan.add_task(PlanTask::new(1, "T1".to_string(), "D1".to_string(), TaskType::Create));
    let json = serde_json::to_string(&plan).unwrap();
    let deserialized: PlanDocument = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.title, "Round Trip");
    assert_eq!(deserialized.tasks.len(), 1);
    assert_eq!(deserialized.tasks[0].title, "T1");
}

#[test]
fn plan_status_serializes() {
    let json = serde_json::to_string(&PlanStatus::InProgress).unwrap();
    let deser: PlanStatus = serde_json::from_str(&json).unwrap();
    assert_eq!(deser, PlanStatus::InProgress);
}

#[test]
fn task_status_serializes() {
    let json = serde_json::to_string(&TaskStatus::Completed).unwrap();
    let deser: TaskStatus = serde_json::from_str(&json).unwrap();
    assert_eq!(deser, TaskStatus::Completed);
}
