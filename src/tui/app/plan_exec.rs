//! Plan lifecycle — load, save, export, execute.

use super::*;
use anyhow::Result;
use uuid::Uuid;

impl App {
    /// Load plan for manual viewing (Ctrl+P)
    /// Loads ANY plan (Draft, PendingApproval, etc.) for viewing
    pub(crate) async fn load_plan_for_viewing(&mut self) -> Result<()> {
        // Get session ID for session-scoped operations
        let session_id = match &self.current_session {
            Some(session) => session.id,
            None => {
                tracing::debug!("No current session, skipping plan load");
                return Ok(());
            }
        };

        tracing::debug!("Loading plan for viewing (session: {})", session_id);

        // Try loading from database first
        match self.plan_service.get_most_recent_plan(session_id).await {
            Ok(Some(plan)) => {
                tracing::info!(
                    "✅ Loaded plan from database: '{}' ({:?}, {} tasks)",
                    plan.title,
                    plan.status,
                    plan.tasks.len()
                );
                self.current_plan = Some(plan);
                return Ok(());
            }
            Ok(None) => {
                tracing::debug!("No plan found in database, checking JSON file");
            }
            Err(e) => {
                tracing::warn!("Failed to load plan from database: {}", e);
            }
        }

        // Fallback to JSON file for backward compatibility / migration
        let plan_filename = format!(".opencrabs_plan_{}.json", session_id);
        let plan_file = self.working_directory.join(&plan_filename);

        tracing::debug!("Looking for plan file at: {}", plan_file.display());

        match tokio::fs::read_to_string(&plan_file).await {
            Ok(content) => {
                tracing::debug!("Found plan JSON file, parsing...");
                match serde_json::from_str::<crate::tui::plan::PlanDocument>(&content) {
                    Ok(plan) => {
                        tracing::info!(
                            "✅ Loaded plan from JSON: '{}' ({:?}, {} tasks)",
                            plan.title,
                            plan.status,
                            plan.tasks.len()
                        );

                        // Migrate to database
                        if let Err(e) = self.plan_service.create(&plan).await {
                            tracing::warn!("Failed to migrate plan to database: {}", e);
                        }

                        self.current_plan = Some(plan);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse plan JSON: {}", e);
                    }
                }
            }
            Err(_) => {
                tracing::debug!("No plan file found");
            }
        }

        Ok(())
    }

    /// Check for and load a plan if one was created
    /// Loads from database first, with JSON fallback for migration
    /// Only loads plans with status PendingApproval (for automatic notification)
    pub(crate) async fn check_and_load_plan(&mut self) -> Result<()> {
        // Get session ID for session-scoped operations
        let session_id = match &self.current_session {
            Some(session) => session.id,
            None => {
                tracing::debug!("No current session, skipping plan load");
                return Ok(());
            }
        };

        tracing::debug!("Checking for pending plan (session: {})", session_id);

        // Try loading from database first
        match self.plan_service.get_most_recent_plan(session_id).await {
            Ok(Some(plan)) => {
                tracing::debug!(
                    "Found plan in database: id={}, status={:?}",
                    plan.id,
                    plan.status
                );
                // Only load if plan is pending approval
                if plan.status == crate::tui::plan::PlanStatus::PendingApproval {
                    tracing::info!("✅ Plan ready for review!");

                    // Only load if not already loaded (avoid duplicate messages)
                    if self.current_plan.is_none() {
                        let plan_title = plan.title.clone();
                        let task_count = plan.tasks.len();
                        let task_summaries: Vec<String> = plan
                            .tasks
                            .iter()
                            .map(|t| format!("{} ({})", t.title, t.task_type))
                            .collect();
                        self.current_plan = Some(plan);

                        // Add inline plan approval selector to chat
                        let notification = DisplayMessage {
                            id: Uuid::new_v4(),
                            role: "plan_approval".to_string(),
                            content: String::new(),
                            timestamp: chrono::Utc::now(),
                            token_count: None,
                            cost: None,
                            approval: None,
                            approve_menu: None,
                            details: None,
                            expanded: false,
                            tool_group: None,
                            plan_approval: Some(PlanApprovalData {
                                plan_title,
                                task_count,
                                task_summaries,
                                state: PlanApprovalState::Pending,
                                selected_option: 0,
                                show_details: false,
                            }),
                        };

                        self.messages.push(notification);
                        self.scroll_offset = 0;
                    }
                }
                return Ok(());
            }
            Ok(None) => {
                tracing::debug!("No pending plan found in database, checking JSON file");
            }
            Err(e) => {
                tracing::warn!("Failed to load plan from database: {}", e);
            }
        }

        // Fallback to JSON file for backward compatibility / migration
        let plan_filename = format!(".opencrabs_plan_{}.json", session_id);
        let plan_file = self.working_directory.join(&plan_filename);

        tracing::debug!("Looking for plan file at: {}", plan_file.display());

        // Check if file exists before trying to read
        let file_exists = plan_file.exists();
        tracing::debug!("Plan file exists: {}", file_exists);

        match tokio::fs::read_to_string(&plan_file).await {
            Ok(content) => {
                tracing::debug!("Found plan JSON file, parsing...");
                match serde_json::from_str::<crate::tui::plan::PlanDocument>(&content) {
                    Ok(plan) => {
                        tracing::debug!(
                            "Parsed plan: id={}, status={:?}, tasks={}",
                            plan.id,
                            plan.status,
                            plan.tasks.len()
                        );
                        // Only load if plan is pending approval
                        if plan.status == crate::tui::plan::PlanStatus::PendingApproval {
                            tracing::info!("✅ Plan ready for review!");

                            // Migrate to database
                            if let Err(e) = self.plan_service.create(&plan).await {
                                tracing::warn!("Failed to migrate plan to database: {}", e);
                            }

                            // Only load if not already loaded (avoid duplicate messages)
                            if self.current_plan.is_none() {
                                let plan_title = plan.title.clone();
                                let task_count = plan.tasks.len();
                                let task_summaries: Vec<String> = plan
                                    .tasks
                                    .iter()
                                    .map(|t| format!("{} ({})", t.title, t.task_type))
                                    .collect();
                                self.current_plan = Some(plan);

                                // Add inline plan approval selector to chat
                                let notification = DisplayMessage {
                                    id: Uuid::new_v4(),
                                    role: "plan_approval".to_string(),
                                    content: String::new(),
                                    timestamp: chrono::Utc::now(),
                                    token_count: None,
                                    cost: None,
                                    approval: None,
                                    approve_menu: None,
                                    details: None,
                                    expanded: false,
                                    tool_group: None,
                                    plan_approval: Some(PlanApprovalData {
                                        plan_title,
                                        task_count,
                                        task_summaries,
                                        state: PlanApprovalState::Pending,
                                        selected_option: 0,
                                        show_details: false,
                                    }),
                                };

                                self.messages.push(notification);
                                self.scroll_offset = 0;
                            }
                        } else {
                            tracing::debug!(
                                "Plan status is {:?}, not PendingApproval - skipping",
                                plan.status
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse plan JSON: {}", e);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!("Plan file not found (this is normal if no plan was created)");
            }
            Err(e) => {
                tracing::warn!("Failed to read plan JSON file: {}", e);
            }
        }

        Ok(())
    }

    /// Save the current plan
    /// Dual-write: database as primary, JSON as backup
    /// Export plan to markdown file
    pub(crate) async fn export_plan_to_markdown(&self, filename: &str) -> Result<()> {
        if let Some(plan) = &self.current_plan {
            // Generate markdown content
            let mut markdown = String::new();
            markdown.push_str(&format!("# {}\n\n", plan.title));
            markdown.push_str(&format!("{}\n\n", plan.description));

            if !plan.context.is_empty() {
                markdown.push_str("## Context\n\n");
                markdown.push_str(&format!("{}\n\n", plan.context));
            }

            if !plan.risks.is_empty() {
                markdown.push_str("## Risks & Considerations\n\n");
                for risk in &plan.risks {
                    markdown.push_str(&format!("- {}\n", risk));
                }
                markdown.push('\n');
            }

            markdown.push_str("## Tasks\n\n");

            for task in &plan.tasks {
                markdown.push_str(&format!("### Task {}: {}\n\n", task.order, task.title));
                markdown.push_str(&format!(
                    "**Type:** {:?} | **Complexity:** {}★\n\n",
                    task.task_type, task.complexity
                ));

                if !task.dependencies.is_empty() {
                    let dep_orders: Vec<String> = task
                        .dependencies
                        .iter()
                        .filter_map(|dep_id| {
                            plan.tasks
                                .iter()
                                .find(|t| &t.id == dep_id)
                                .map(|t| t.order.to_string())
                        })
                        .collect();
                    markdown.push_str(&format!(
                        "**Dependencies:** Task(s) {}\n\n",
                        dep_orders.join(", ")
                    ));
                }

                markdown.push_str("**Implementation Steps:**\n\n");
                markdown.push_str(&format!("{}\n\n", task.description));
                markdown.push_str("---\n\n");
            }

            markdown.push_str(&format!(
                "\n*Plan created: {}*\n",
                plan.created_at.format("%Y-%m-%d %H:%M:%S")
            ));
            markdown.push_str(&format!(
                "*Last updated: {}*\n",
                plan.updated_at.format("%Y-%m-%d %H:%M:%S")
            ));

            // Write markdown file to working directory
            let output_path = self.working_directory.join(filename);

            // Write markdown file (overwrite if exists)
            tokio::fs::write(&output_path, markdown)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to write markdown file: {}", e))?;

            tracing::info!("Exported plan to {}", output_path.display());
        }

        Ok(())
    }

    pub(crate) async fn save_plan(&self) -> Result<()> {
        if let Some(plan) = &self.current_plan {
            // Get session ID for session-scoped operations
            let session_id = match &self.current_session {
                Some(session) => session.id,
                None => {
                    tracing::warn!("Cannot save plan: no current session");
                    return Ok(());
                }
            };

            // Primary: Save to database
            // Try to update first (plan may already exist)
            match self.plan_service.update(plan).await {
                Ok(_) => {
                    tracing::debug!("Updated plan in database: {}", plan.id);
                }
                Err(_) => {
                    // If update fails, try creating (plan doesn't exist yet)
                    if let Err(e) = self.plan_service.create(plan).await {
                        tracing::error!("Failed to save plan to database: {}", e);
                        // Continue to JSON backup even if database fails
                    } else {
                        tracing::debug!("Created plan in database: {}", plan.id);
                    }
                }
            }

            // Backup: Save to JSON file (for backward compatibility and backup)
            let plan_filename = format!(".opencrabs_plan_{}.json", session_id);
            let plan_file = self.working_directory.join(&plan_filename);

            if let Err(e) = self.plan_service.export_to_json(plan, &plan_file).await {
                tracing::warn!("Failed to save plan JSON backup: {}", e);
            }
        }
        Ok(())
    }

    /// Execute plan tasks sequentially
    pub(crate) async fn execute_plan_tasks(&mut self) -> Result<()> {
        self.executing_plan = true;
        self.execute_next_plan_task().await
    }

    /// Execute the next pending task in the plan
    pub(crate) async fn execute_next_plan_task(&mut self) -> Result<()> {
        // Collect necessary data from plan first to avoid borrow issues
        let (task_message, completion_data) = {
            let Some(plan) = &mut self.current_plan else {
                self.executing_plan = false;
                return Ok(());
            };

            // Get tasks in dependency order
            let Some(ordered_tasks) = plan.tasks_in_order() else {
                self.executing_plan = false;
                self.show_error(
                    "❌ Cannot Execute Plan\n\n\
                     Circular dependency detected in task graph. Tasks cannot be ordered \
                     because they form a dependency cycle.\n\n\
                     💡 Fix: Review task dependencies and remove circular references.\n\
                     You can reject this plan (Ctrl+R) and ask the AI to revise it."
                        .to_string(),
                );
                return Ok(());
            };

            // Find the next pending task and extract its data
            let next_task_data = ordered_tasks
                .iter()
                .find(|task| matches!(task.status, crate::tui::plan::TaskStatus::Pending))
                .map(|task| {
                    (
                        task.id,
                        task.order,
                        task.title.clone(),
                        task.description.clone(),
                    )
                });

            let total_tasks = plan.tasks.len();

            // Drop the immutable borrow of ordered_tasks
            drop(ordered_tasks);

            match next_task_data {
                Some((task_id, order, title, description)) => {
                    // Mark task as in progress
                    if let Some(task_mut) = plan.tasks.iter_mut().find(|t| t.id == task_id) {
                        task_mut.status = crate::tui::plan::TaskStatus::InProgress;
                    }

                    // Prepare task message
                    let message = format!(
                        "📋 Executing Plan Task #{}/{}\n\n\
                         **{}**\n\n\
                         {}\n\n\
                         Please complete this task.",
                        order, total_tasks, title, description
                    );

                    (Some(message), None)
                }
                None => {
                    // No more pending tasks - plan is complete
                    let title = plan.title.clone();
                    let task_count = plan.tasks.len();
                    plan.complete();
                    self.executing_plan = false;

                    (None, Some((title, task_count)))
                }
            }
        };

        // Save plan after releasing borrow
        self.save_plan().await?;

        // Handle results
        if let Some((title, task_count)) = completion_data {
            // Add completion message
            let completion_msg = DisplayMessage {
                id: uuid::Uuid::new_v4(),
                role: "system".to_string(),
                content: format!(
                    "Plan '{}' completed successfully!\n\
                     All {} tasks have been executed.",
                    title, task_count
                ),
                timestamp: chrono::Utc::now(),
                token_count: None,
                cost: None,
                approval: None,
                approve_menu: None,
                details: None,
                expanded: false,
                tool_group: None,
                plan_approval: None,
            };
            self.messages.push(completion_msg);
        } else if let Some(message) = task_message {
            // Send task message to agent
            tracing::info!(
                "Sending plan task to agent (is_processing={})",
                self.is_processing
            );
            self.send_message(message).await?;
            tracing::info!("Plan task sent (is_processing={})", self.is_processing);
        }

        Ok(())
    }
}
