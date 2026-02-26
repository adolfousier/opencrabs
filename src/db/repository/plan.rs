//! Plan Repository
//!
//! Database operations for plans and plan tasks.

use crate::db::models::{Plan, PlanTask};
use crate::tui::plan::{PlanDocument, PlanStatus, TaskStatus, TaskType};
use anyhow::{Context, Result};
use sqlx::SqlitePool;
use uuid::Uuid;

/// Repository for plan operations
#[derive(Clone)]
pub struct PlanRepository {
    pool: SqlitePool,
}

impl PlanRepository {
    /// Create a new plan repository
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Find plan by ID
    pub async fn find_by_id(&self, id: Uuid) -> Result<Option<PlanDocument>> {
        let plan = sqlx::query_as::<_, Plan>("SELECT * FROM plans WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await
            .context("Failed to find plan")?;

        let Some(plan) = plan else {
            return Ok(None);
        };

        // Fetch associated tasks
        let tasks = self.find_tasks_by_plan_id(id).await?;

        // Convert database models to domain models
        Ok(Some(self.plan_from_db(plan, tasks)?))
    }

    /// Find all plans for a session
    pub async fn find_by_session_id(&self, session_id: Uuid) -> Result<Vec<PlanDocument>> {
        let plans = sqlx::query_as::<_, Plan>(
            "SELECT * FROM plans WHERE session_id = ? ORDER BY updated_at DESC",
        )
        .bind(session_id.to_string())
        .fetch_all(&self.pool)
        .await
        .context("Failed to find plans by session")?;

        let mut result = Vec::new();
        for plan in plans {
            let tasks = self.find_tasks_by_plan_id(plan.id).await?;
            result.push(self.plan_from_db(plan, tasks)?);
        }

        Ok(result)
    }

    /// Find tasks for a plan
    async fn find_tasks_by_plan_id(&self, plan_id: Uuid) -> Result<Vec<PlanTask>> {
        let tasks = sqlx::query_as::<_, PlanTask>(
            "SELECT * FROM plan_tasks WHERE plan_id = ? ORDER BY task_order ASC",
        )
        .bind(plan_id.to_string())
        .fetch_all(&self.pool)
        .await
        .context("Failed to find plan tasks")?;

        Ok(tasks)
    }

    /// Create a new plan with tasks
    pub async fn create(&self, plan: &PlanDocument) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        // Insert plan
        let (db_plan, db_tasks) = self.plan_to_db(plan)?;

        sqlx::query(
            r#"
            INSERT INTO plans (id, session_id, title, description, context, risks,
                             test_strategy, technical_stack, status, created_at, updated_at, approved_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(db_plan.id.to_string())
        .bind(db_plan.session_id.to_string())
        .bind(&db_plan.title)
        .bind(&db_plan.description)
        .bind(&db_plan.context)
        .bind(&db_plan.risks)
        .bind(&db_plan.test_strategy)
        .bind(&db_plan.technical_stack)
        .bind(&db_plan.status)
        .bind(db_plan.created_at.timestamp())
        .bind(db_plan.updated_at.timestamp())
        .bind(db_plan.approved_at.map(|dt| dt.timestamp()))
        .execute(&mut *tx)
        .await
        .context("Failed to create plan")?;

        // Insert tasks
        for task in db_tasks {
            sqlx::query(
                r#"
                INSERT INTO plan_tasks (id, plan_id, task_order, title, description,
                                       task_type, dependencies, complexity, acceptance_criteria,
                                       status, notes, completed_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(task.id.to_string())
            .bind(task.plan_id.to_string())
            .bind(task.task_order)
            .bind(&task.title)
            .bind(&task.description)
            .bind(&task.task_type)
            .bind(&task.dependencies)
            .bind(task.complexity)
            .bind(&task.acceptance_criteria)
            .bind(&task.status)
            .bind(&task.notes)
            .bind(task.completed_at.map(|dt| dt.timestamp()))
            .execute(&mut *tx)
            .await
            .context("Failed to create plan task")?;
        }

        tx.commit().await?;

        tracing::debug!("Created plan: {}", plan.id);
        Ok(())
    }

    /// Update an existing plan
    pub async fn update(&self, plan: &PlanDocument) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        let (db_plan, db_tasks) = self.plan_to_db(plan)?;

        // Update plan
        sqlx::query(
            r#"
            UPDATE plans
            SET title = ?, description = ?, context = ?, risks = ?,
                test_strategy = ?, technical_stack = ?,
                status = ?, updated_at = ?, approved_at = ?
            WHERE id = ?
            "#,
        )
        .bind(&db_plan.title)
        .bind(&db_plan.description)
        .bind(&db_plan.context)
        .bind(&db_plan.risks)
        .bind(&db_plan.test_strategy)
        .bind(&db_plan.technical_stack)
        .bind(&db_plan.status)
        .bind(db_plan.updated_at.timestamp())
        .bind(db_plan.approved_at.map(|dt| dt.timestamp()))
        .bind(db_plan.id.to_string())
        .execute(&mut *tx)
        .await
        .context("Failed to update plan")?;

        // Delete existing tasks and re-insert
        sqlx::query("DELETE FROM plan_tasks WHERE plan_id = ?")
            .bind(db_plan.id.to_string())
            .execute(&mut *tx)
            .await
            .context("Failed to delete old plan tasks")?;

        // Insert updated tasks
        for task in db_tasks {
            sqlx::query(
                r#"
                INSERT INTO plan_tasks (id, plan_id, task_order, title, description,
                                       task_type, dependencies, complexity, acceptance_criteria,
                                       status, notes, completed_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(task.id.to_string())
            .bind(task.plan_id.to_string())
            .bind(task.task_order)
            .bind(&task.title)
            .bind(&task.description)
            .bind(&task.task_type)
            .bind(&task.dependencies)
            .bind(task.complexity)
            .bind(&task.acceptance_criteria)
            .bind(&task.status)
            .bind(&task.notes)
            .bind(task.completed_at.map(|dt| dt.timestamp()))
            .execute(&mut *tx)
            .await
            .context("Failed to update plan task")?;
        }

        tx.commit().await?;

        tracing::debug!("Updated plan: {}", plan.id);
        Ok(())
    }

    /// Delete a plan and all its tasks
    pub async fn delete(&self, id: Uuid) -> Result<()> {
        // Tasks will be deleted automatically via CASCADE
        sqlx::query("DELETE FROM plans WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .context("Failed to delete plan")?;

        tracing::debug!("Deleted plan: {}", id);
        Ok(())
    }

    /// Convert database models to domain model
    fn plan_from_db(&self, db_plan: Plan, db_tasks: Vec<PlanTask>) -> Result<PlanDocument> {
        let risks: Vec<String> =
            serde_json::from_str(&db_plan.risks).context("Failed to parse risks JSON")?;
        let technical_stack: Vec<String> = serde_json::from_str(&db_plan.technical_stack)
            .context("Failed to parse technical_stack JSON")?;

        let status = self.parse_plan_status(&db_plan.status)?;

        let mut tasks = Vec::new();
        for db_task in db_tasks {
            tasks.push(self.task_from_db(db_task)?);
        }

        Ok(PlanDocument {
            id: db_plan.id,
            session_id: db_plan.session_id,
            title: db_plan.title,
            description: db_plan.description,
            tasks,
            context: db_plan.context,
            risks,
            test_strategy: db_plan.test_strategy,
            technical_stack,
            status,
            created_at: db_plan.created_at,
            updated_at: db_plan.updated_at,
            approved_at: db_plan.approved_at,
        })
    }

    /// Convert database task to domain task
    fn task_from_db(&self, db_task: PlanTask) -> Result<crate::tui::plan::PlanTask> {
        let dependencies: Vec<Uuid> = serde_json::from_str(&db_task.dependencies)
            .context("Failed to parse dependencies JSON")?;
        let acceptance_criteria: Vec<String> =
            serde_json::from_str(&db_task.acceptance_criteria)
                .context("Failed to parse acceptance_criteria JSON")?;

        let task_type = self.parse_task_type(&db_task.task_type)?;
        let status = self.parse_task_status(&db_task.status)?;

        Ok(crate::tui::plan::PlanTask {
            id: db_task.id,
            order: db_task.task_order as usize,
            title: db_task.title,
            description: db_task.description,
            task_type,
            dependencies,
            complexity: db_task.complexity as u8,
            acceptance_criteria,
            status,
            notes: db_task.notes,
            completed_at: db_task.completed_at,
            execution_history: Vec::new(),
            retry_count: 0,
            max_retries: 3,
            artifacts: Vec::new(),
            reflection: None,
        })
    }

    /// Convert domain model to database models
    fn plan_to_db(&self, plan: &PlanDocument) -> Result<(Plan, Vec<PlanTask>)> {
        let risks = serde_json::to_string(&plan.risks).context("Failed to serialize risks")?;
        let technical_stack = serde_json::to_string(&plan.technical_stack)
            .context("Failed to serialize technical_stack")?;

        let db_plan = Plan {
            id: plan.id,
            session_id: plan.session_id,
            title: plan.title.clone(),
            description: plan.description.clone(),
            context: plan.context.clone(),
            risks,
            test_strategy: plan.test_strategy.clone(),
            technical_stack,
            status: self.format_plan_status(&plan.status),
            created_at: plan.created_at,
            updated_at: plan.updated_at,
            approved_at: plan.approved_at,
        };

        let mut db_tasks = Vec::new();
        for task in &plan.tasks {
            db_tasks.push(self.task_to_db(task, plan.id)?);
        }

        Ok((db_plan, db_tasks))
    }

    /// Convert domain task to database task
    fn task_to_db(&self, task: &crate::tui::plan::PlanTask, plan_id: Uuid) -> Result<PlanTask> {
        let dependencies = serde_json::to_string(&task.dependencies)
            .context("Failed to serialize dependencies")?;
        let acceptance_criteria = serde_json::to_string(&task.acceptance_criteria)
            .context("Failed to serialize acceptance_criteria")?;

        Ok(PlanTask {
            id: task.id,
            plan_id,
            task_order: task.order as i32,
            title: task.title.clone(),
            description: task.description.clone(),
            task_type: self.format_task_type(&task.task_type),
            dependencies,
            complexity: task.complexity as i32,
            acceptance_criteria,
            status: self.format_task_status(&task.status),
            notes: task.notes.clone(),
            completed_at: task.completed_at,
        })
    }

    /// Parse plan status from string
    fn parse_plan_status(&self, status: &str) -> Result<PlanStatus> {
        Ok(match status {
            "Draft" => PlanStatus::Draft,
            "PendingApproval" => PlanStatus::PendingApproval,
            "Approved" => PlanStatus::Approved,
            "Rejected" => PlanStatus::Rejected,
            "InProgress" => PlanStatus::InProgress,
            "Completed" => PlanStatus::Completed,
            "Cancelled" => PlanStatus::Cancelled,
            _ => anyhow::bail!("Invalid plan status: {}", status),
        })
    }

    /// Format plan status to string
    fn format_plan_status(&self, status: &PlanStatus) -> String {
        match status {
            PlanStatus::Draft => "Draft",
            PlanStatus::PendingApproval => "PendingApproval",
            PlanStatus::Approved => "Approved",
            PlanStatus::Rejected => "Rejected",
            PlanStatus::InProgress => "InProgress",
            PlanStatus::Completed => "Completed",
            PlanStatus::Cancelled => "Cancelled",
        }
        .to_string()
    }

    /// Parse task type from string
    fn parse_task_type(&self, task_type: &str) -> Result<TaskType> {
        Ok(match task_type {
            "Research" => TaskType::Research,
            "Edit" => TaskType::Edit,
            "Create" => TaskType::Create,
            "Delete" => TaskType::Delete,
            "Test" => TaskType::Test,
            "Refactor" => TaskType::Refactor,
            "Documentation" => TaskType::Documentation,
            "Configuration" => TaskType::Configuration,
            "Build" => TaskType::Build,
            other => TaskType::Other(other.to_string()),
        })
    }

    /// Format task type to string
    fn format_task_type(&self, task_type: &TaskType) -> String {
        match task_type {
            TaskType::Research => "Research",
            TaskType::Edit => "Edit",
            TaskType::Create => "Create",
            TaskType::Delete => "Delete",
            TaskType::Test => "Test",
            TaskType::Refactor => "Refactor",
            TaskType::Documentation => "Documentation",
            TaskType::Configuration => "Configuration",
            TaskType::Build => "Build",
            TaskType::Other(s) => s,
        }
        .to_string()
    }

    /// Parse task status from string
    fn parse_task_status(&self, status: &str) -> Result<TaskStatus> {
        if let Some(reason) = status.strip_prefix("Blocked:") {
            return Ok(TaskStatus::Blocked(reason.trim().to_string()));
        }

        Ok(match status {
            "Pending" => TaskStatus::Pending,
            "InProgress" => TaskStatus::InProgress,
            "Completed" => TaskStatus::Completed,
            "Skipped" => TaskStatus::Skipped,
            "Failed" => TaskStatus::Failed,
            _ => anyhow::bail!("Invalid task status: {}", status),
        })
    }

    /// Format task status to string
    fn format_task_status(&self, status: &TaskStatus) -> String {
        match status {
            TaskStatus::Pending => "Pending".to_string(),
            TaskStatus::InProgress => "InProgress".to_string(),
            TaskStatus::Completed => "Completed".to_string(),
            TaskStatus::Skipped => "Skipped".to_string(),
            TaskStatus::Failed => "Failed".to_string(),
            TaskStatus::Blocked(reason) => format!("Blocked:{}", reason),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::db::models::Session;
    use crate::db::repository::session::SessionRepository;
    use crate::tui::plan::{PlanTask, TaskType};
    use chrono::Utc;

    /// Helper to create a test database and session
    async fn setup_test_db() -> (Database, SessionRepository, PlanRepository, Session) {
        let db = Database::connect_in_memory()
            .await
            .expect("Failed to create database");
        db.run_migrations().await.expect("Failed to run migrations");

        let session_repo = SessionRepository::new(db.pool().clone());
        let plan_repo = PlanRepository::new(db.pool().clone());

        // Create a test session (required for foreign key)
        let session = Session::new(
            Some("Test Session".to_string()),
            Some("claude-sonnet-4-5".to_string()),
            None,
        );
        session_repo
            .create(&session)
            .await
            .expect("Failed to create test session");

        (db, session_repo, plan_repo, session)
    }

    /// Helper to create a test plan
    fn create_test_plan(session_id: Uuid) -> PlanDocument {
        let mut plan = PlanDocument::new(
            session_id,
            "Test Plan".to_string(),
            "A test plan for unit testing".to_string(),
        );

        plan.context = "Test context".to_string();
        plan.risks = vec!["Risk 1".to_string(), "Risk 2".to_string()];

        // Add some tasks
        let task1 = PlanTask {
            id: Uuid::new_v4(),
            order: 0,
            title: "Task 1".to_string(),
            description: "First task".to_string(),
            task_type: TaskType::Research,
            dependencies: vec![],
            complexity: 3,
            acceptance_criteria: vec![],
            status: TaskStatus::Pending,
            notes: None,
            completed_at: None,
            execution_history: Vec::new(),
            retry_count: 0,
            max_retries: 3,
            artifacts: Vec::new(),
            reflection: None,
        };

        let task2 = PlanTask {
            id: Uuid::new_v4(),
            order: 1,
            title: "Task 2".to_string(),
            description: "Second task".to_string(),
            task_type: TaskType::Edit,
            dependencies: vec![task1.id],
            complexity: 5,
            acceptance_criteria: vec![],
            status: TaskStatus::Pending,
            notes: Some("Some notes".to_string()),
            completed_at: None,
            execution_history: Vec::new(),
            retry_count: 0,
            max_retries: 3,
            artifacts: Vec::new(),
            reflection: None,
        };

        plan.add_task(task1);
        plan.add_task(task2);

        plan
    }

    #[tokio::test]
    async fn test_plan_create() {
        let (_db, _session_repo, plan_repo, session) = setup_test_db().await;

        let plan = create_test_plan(session.id);
        let plan_id = plan.id;

        plan_repo
            .create(&plan)
            .await
            .expect("Failed to create plan");

        // Verify plan was created
        let found = plan_repo
            .find_by_id(plan_id)
            .await
            .expect("Failed to find plan");
        assert!(found.is_some());
        let found_plan = found.unwrap();
        assert_eq!(found_plan.title, "Test Plan");
        assert_eq!(found_plan.tasks.len(), 2);
    }

    #[tokio::test]
    async fn test_plan_find_by_id() {
        let (_db, _session_repo, plan_repo, session) = setup_test_db().await;

        let plan = create_test_plan(session.id);
        plan_repo
            .create(&plan)
            .await
            .expect("Failed to create plan");

        // Find existing plan
        let found = plan_repo
            .find_by_id(plan.id)
            .await
            .expect("Failed to find plan");
        assert!(found.is_some());

        // Find non-existent plan
        let not_found = plan_repo
            .find_by_id(Uuid::new_v4())
            .await
            .expect("Failed to query plan");
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_plan_find_by_session_id() {
        let (_db, _session_repo, plan_repo, session) = setup_test_db().await;

        // Create multiple plans for the same session
        let plan1 = create_test_plan(session.id);
        let plan2 = create_test_plan(session.id);

        plan_repo
            .create(&plan1)
            .await
            .expect("Failed to create plan1");
        plan_repo
            .create(&plan2)
            .await
            .expect("Failed to create plan2");

        // Find all plans for session
        let plans = plan_repo
            .find_by_session_id(session.id)
            .await
            .expect("Failed to find plans");
        assert_eq!(plans.len(), 2);

        // Verify plans are ordered by updated_at DESC (most recent first)
        // plan2 should be first since it was created later
        assert!(plans[0].updated_at >= plans[1].updated_at);
    }

    #[tokio::test]
    async fn test_plan_update() {
        let (_db, _session_repo, plan_repo, session) = setup_test_db().await;

        let mut plan = create_test_plan(session.id);
        plan_repo
            .create(&plan)
            .await
            .expect("Failed to create plan");

        // Update plan
        plan.title = "Updated Plan Title".to_string();
        plan.status = PlanStatus::Approved;
        plan.approved_at = Some(Utc::now());

        // Add a new task
        let task3 = PlanTask {
            id: Uuid::new_v4(),
            order: 2,
            title: "Task 3".to_string(),
            description: "Third task".to_string(),
            task_type: TaskType::Create,
            dependencies: vec![],
            complexity: 2,
            acceptance_criteria: vec![],
            status: TaskStatus::Pending,
            notes: None,
            completed_at: None,
            execution_history: Vec::new(),
            retry_count: 0,
            max_retries: 3,
            artifacts: Vec::new(),
            reflection: None,
        };
        plan.add_task(task3);

        plan_repo
            .update(&plan)
            .await
            .expect("Failed to update plan");

        // Verify updates
        let found = plan_repo
            .find_by_id(plan.id)
            .await
            .expect("Failed to find plan")
            .unwrap();
        assert_eq!(found.title, "Updated Plan Title");
        assert_eq!(found.status, PlanStatus::Approved);
        assert!(found.approved_at.is_some());
        assert_eq!(found.tasks.len(), 3);
    }

    #[tokio::test]
    async fn test_plan_delete() {
        let (_db, _session_repo, plan_repo, session) = setup_test_db().await;

        let plan = create_test_plan(session.id);
        plan_repo
            .create(&plan)
            .await
            .expect("Failed to create plan");

        // Verify plan exists
        let found = plan_repo.find_by_id(plan.id).await.expect("Failed to find");
        assert!(found.is_some());

        // Delete plan
        plan_repo
            .delete(plan.id)
            .await
            .expect("Failed to delete plan");

        // Verify plan is deleted
        let not_found = plan_repo
            .find_by_id(plan.id)
            .await
            .expect("Failed to query");
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_plan_tasks_cascade_delete() {
        let (_db, _session_repo, plan_repo, session) = setup_test_db().await;

        let plan = create_test_plan(session.id);
        let plan_id = plan.id;

        plan_repo
            .create(&plan)
            .await
            .expect("Failed to create plan");

        // Verify tasks exist
        let found = plan_repo
            .find_by_id(plan_id)
            .await
            .expect("Failed to find")
            .unwrap();
        assert_eq!(found.tasks.len(), 2);

        // Delete plan
        plan_repo.delete(plan_id).await.expect("Failed to delete");

        // Verify plan and tasks are deleted
        let not_found = plan_repo
            .find_by_id(plan_id)
            .await
            .expect("Failed to query");
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_plan_status_conversion() {
        let (_db, _session_repo, plan_repo, session) = setup_test_db().await;

        let statuses = vec![
            PlanStatus::Draft,
            PlanStatus::PendingApproval,
            PlanStatus::Approved,
            PlanStatus::Rejected,
            PlanStatus::InProgress,
            PlanStatus::Completed,
            PlanStatus::Cancelled,
        ];

        for status in statuses {
            let mut plan = create_test_plan(session.id);
            plan.status = status.clone();

            plan_repo
                .create(&plan)
                .await
                .expect("Failed to create plan");

            let found = plan_repo
                .find_by_id(plan.id)
                .await
                .expect("Failed to find")
                .unwrap();
            assert_eq!(found.status, status);

            // Clean up for next iteration
            plan_repo.delete(plan.id).await.expect("Failed to delete");
        }
    }

    #[tokio::test]
    async fn test_task_type_conversion() {
        let (_db, _session_repo, plan_repo, session) = setup_test_db().await;

        let task_types = vec![
            TaskType::Research,
            TaskType::Edit,
            TaskType::Create,
            TaskType::Delete,
            TaskType::Test,
            TaskType::Refactor,
            TaskType::Documentation,
            TaskType::Configuration,
            TaskType::Build,
            TaskType::Other("CustomType".to_string()),
        ];

        for task_type in task_types {
            let mut plan = create_test_plan(session.id);
            plan.tasks[0].task_type = task_type.clone();

            plan_repo
                .create(&plan)
                .await
                .expect("Failed to create plan");

            let found = plan_repo
                .find_by_id(plan.id)
                .await
                .expect("Failed to find")
                .unwrap();
            assert_eq!(found.tasks[0].task_type, task_type);

            // Clean up for next iteration
            plan_repo.delete(plan.id).await.expect("Failed to delete");
        }
    }

    #[tokio::test]
    async fn test_task_status_conversion() {
        let (_db, _session_repo, plan_repo, session) = setup_test_db().await;

        let task_statuses = vec![
            TaskStatus::Pending,
            TaskStatus::InProgress,
            TaskStatus::Completed,
            TaskStatus::Skipped,
            TaskStatus::Failed,
            TaskStatus::Blocked("Waiting for review".to_string()),
        ];

        for task_status in task_statuses {
            let mut plan = create_test_plan(session.id);
            plan.tasks[0].status = task_status.clone();

            plan_repo
                .create(&plan)
                .await
                .expect("Failed to create plan");

            let found = plan_repo
                .find_by_id(plan.id)
                .await
                .expect("Failed to find")
                .unwrap();
            assert_eq!(found.tasks[0].status, task_status);

            // Clean up for next iteration
            plan_repo.delete(plan.id).await.expect("Failed to delete");
        }
    }

    #[tokio::test]
    async fn test_task_dependencies_serialization() {
        let (_db, _session_repo, plan_repo, session) = setup_test_db().await;

        let plan = create_test_plan(session.id);
        let task1_id = plan.tasks[0].id;

        plan_repo
            .create(&plan)
            .await
            .expect("Failed to create plan");

        let found = plan_repo
            .find_by_id(plan.id)
            .await
            .expect("Failed to find")
            .unwrap();

        // Task 1 should have no dependencies
        assert_eq!(found.tasks[0].dependencies.len(), 0);

        // Task 2 should depend on Task 1
        assert_eq!(found.tasks[1].dependencies.len(), 1);
        assert_eq!(found.tasks[1].dependencies[0], task1_id);
    }

    #[tokio::test]
    async fn test_plan_risks_serialization() {
        let (_db, _session_repo, plan_repo, session) = setup_test_db().await;

        let plan = create_test_plan(session.id);
        plan_repo
            .create(&plan)
            .await
            .expect("Failed to create plan");

        let found = plan_repo
            .find_by_id(plan.id)
            .await
            .expect("Failed to find")
            .unwrap();

        assert_eq!(found.risks.len(), 2);
        assert_eq!(found.risks[0], "Risk 1");
        assert_eq!(found.risks[1], "Risk 2");
    }

    #[tokio::test]
    async fn test_plan_with_no_tasks() {
        let (_db, _session_repo, plan_repo, session) = setup_test_db().await;

        let mut plan = PlanDocument::new(
            session.id,
            "Empty Plan".to_string(),
            "A plan with no tasks".to_string(),
        );
        plan.risks = vec![];

        plan_repo
            .create(&plan)
            .await
            .expect("Failed to create plan");

        let found = plan_repo
            .find_by_id(plan.id)
            .await
            .expect("Failed to find")
            .unwrap();

        assert_eq!(found.tasks.len(), 0);
        assert_eq!(found.risks.len(), 0);
    }

    #[tokio::test]
    async fn test_plan_update_task_status() {
        let (_db, _session_repo, plan_repo, session) = setup_test_db().await;

        let mut plan = create_test_plan(session.id);
        plan_repo
            .create(&plan)
            .await
            .expect("Failed to create plan");

        // Update task statuses
        let task0_id = plan.tasks[0].id;
        let task1_id = plan.tasks[1].id;

        if let Some(task) = plan.get_task_mut(&task0_id) {
            task.status = TaskStatus::Completed;
            task.completed_at = Some(Utc::now());
        }

        if let Some(task) = plan.get_task_mut(&task1_id) {
            task.status = TaskStatus::InProgress;
        }

        plan_repo
            .update(&plan)
            .await
            .expect("Failed to update plan");

        let found = plan_repo
            .find_by_id(plan.id)
            .await
            .expect("Failed to find")
            .unwrap();

        assert_eq!(found.tasks[0].status, TaskStatus::Completed);
        assert!(found.tasks[0].completed_at.is_some());
        assert_eq!(found.tasks[1].status, TaskStatus::InProgress);
    }

    #[tokio::test]
    async fn test_plan_with_complex_task_graph() {
        let (_db, _session_repo, plan_repo, session) = setup_test_db().await;

        let mut plan = PlanDocument::new(
            session.id,
            "Complex Plan".to_string(),
            "A plan with complex dependencies".to_string(),
        );

        // Create 5 tasks with various dependencies
        let task_ids: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();

        for (i, &task_id) in task_ids.iter().enumerate() {
            let dependencies = match i {
                0 => vec![],                         // No dependencies
                1 => vec![task_ids[0]],              // Depends on task 0
                2 => vec![task_ids[0]],              // Depends on task 0
                3 => vec![task_ids[1], task_ids[2]], // Depends on tasks 1 and 2
                4 => vec![task_ids[3]],              // Depends on task 3
                _ => vec![],
            };

            let task = PlanTask {
                id: task_id,
                order: i,
                title: format!("Task {}", i),
                description: format!("Description for task {}", i),
                task_type: TaskType::Research,
                dependencies,
                complexity: ((i % 5) + 1) as u8,
                acceptance_criteria: vec![],
                status: TaskStatus::Pending,
                notes: None,
                completed_at: None,
                execution_history: Vec::new(),
                retry_count: 0,
                max_retries: 3,
                artifacts: Vec::new(),
                reflection: None,
            };
            plan.add_task(task);
        }

        plan_repo
            .create(&plan)
            .await
            .expect("Failed to create plan");

        let found = plan_repo
            .find_by_id(plan.id)
            .await
            .expect("Failed to find")
            .unwrap();

        assert_eq!(found.tasks.len(), 5);

        // Verify dependencies are preserved
        assert_eq!(found.tasks[0].dependencies.len(), 0);
        assert_eq!(found.tasks[1].dependencies.len(), 1);
        assert_eq!(found.tasks[2].dependencies.len(), 1);
        assert_eq!(found.tasks[3].dependencies.len(), 2);
        assert_eq!(found.tasks[4].dependencies.len(), 1);
    }

    #[tokio::test]
    async fn test_multiple_sessions_multiple_plans() {
        let (_db, session_repo, plan_repo, session1) = setup_test_db().await;

        // Create a second session
        let session2 = Session::new(
            Some("Test Session 2".to_string()),
            Some("claude-sonnet-4-5".to_string()),
            None,
        );
        session_repo
            .create(&session2)
            .await
            .expect("Failed to create session2");

        // Create plans for both sessions
        let plan1_s1 = create_test_plan(session1.id);
        let plan2_s1 = create_test_plan(session1.id);
        let plan1_s2 = create_test_plan(session2.id);

        plan_repo
            .create(&plan1_s1)
            .await
            .expect("Failed to create plan1_s1");
        plan_repo
            .create(&plan2_s1)
            .await
            .expect("Failed to create plan2_s1");
        plan_repo
            .create(&plan1_s2)
            .await
            .expect("Failed to create plan1_s2");

        // Verify session 1 has 2 plans
        let session1_plans = plan_repo
            .find_by_session_id(session1.id)
            .await
            .expect("Failed to find session1 plans");
        assert_eq!(session1_plans.len(), 2);

        // Verify session 2 has 1 plan
        let session2_plans = plan_repo
            .find_by_session_id(session2.id)
            .await
            .expect("Failed to find session2 plans");
        assert_eq!(session2_plans.len(), 1);
    }
}
