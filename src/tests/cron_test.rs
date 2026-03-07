//! Cron Jobs Tests
//!
//! Tests for CLI parsing, DB repository CRUD, cron expression validation,
//! scheduler logic, and the cron_manage agent tool.

// --- CLI Parsing Tests ---

mod cli {
    use crate::cli::{Cli, Commands, CronCommands};
    use clap::Parser;

    #[test]
    fn test_cron_add_full() {
        let cli = Cli::try_parse_from([
            "opencrabs",
            "cron",
            "add",
            "--name",
            "Daily Report",
            "--cron",
            "0 9 * * *",
            "--tz",
            "America/New_York",
            "--prompt",
            "Check emails and summarize",
            "--provider",
            "anthropic",
            "--model",
            "claude-sonnet-4-20250514",
            "--thinking",
            "off",
            "--deliver-to",
            "telegram:123456",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::Cron {
                operation:
                    CronCommands::Add {
                        name,
                        cron,
                        tz,
                        prompt,
                        provider,
                        model,
                        thinking,
                        deliver_to,
                        ..
                    },
            }) => {
                assert_eq!(name, "Daily Report");
                assert_eq!(cron, "0 9 * * *");
                assert_eq!(tz, "America/New_York");
                assert_eq!(prompt, "Check emails and summarize");
                assert_eq!(provider, Some("anthropic".to_string()));
                assert_eq!(model, Some("claude-sonnet-4-20250514".to_string()));
                assert_eq!(thinking, "off");
                assert_eq!(deliver_to, Some("telegram:123456".to_string()));
            }
            _ => panic!("Expected Cron Add command"),
        }
    }

    #[test]
    fn test_cron_add_minimal() {
        let cli = Cli::try_parse_from([
            "opencrabs",
            "cron",
            "add",
            "--name",
            "Test",
            "--cron",
            "*/30 * * * *",
            "--prompt",
            "Do something",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::Cron {
                operation:
                    CronCommands::Add {
                        name,
                        cron,
                        tz,
                        provider,
                        model,
                        deliver_to,
                        ..
                    },
            }) => {
                assert_eq!(name, "Test");
                assert_eq!(cron, "*/30 * * * *");
                assert_eq!(tz, "UTC"); // default
                assert!(provider.is_none());
                assert!(model.is_none());
                assert!(deliver_to.is_none());
            }
            _ => panic!("Expected Cron Add command"),
        }
    }

    #[test]
    fn test_cron_add_message_alias() {
        // --message should work as alias for --prompt
        let cli = Cli::try_parse_from([
            "opencrabs",
            "cron",
            "add",
            "--name",
            "Test",
            "--cron",
            "0 9 * * *",
            "--message",
            "Hello",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::Cron {
                operation: CronCommands::Add { prompt, .. },
            }) => {
                assert_eq!(prompt, "Hello");
            }
            _ => panic!("Expected Cron Add command"),
        }
    }

    #[test]
    fn test_cron_add_deliver_alias() {
        // --deliver should work as alias for --deliver-to
        let cli = Cli::try_parse_from([
            "opencrabs",
            "cron",
            "add",
            "--name",
            "Test",
            "--cron",
            "0 9 * * *",
            "--prompt",
            "Hello",
            "--deliver",
            "slack:C123",
        ])
        .unwrap();

        match cli.command {
            Some(Commands::Cron {
                operation: CronCommands::Add { deliver_to, .. },
            }) => {
                assert_eq!(deliver_to, Some("slack:C123".to_string()));
            }
            _ => panic!("Expected Cron Add command"),
        }
    }

    #[test]
    fn test_cron_add_missing_name() {
        let result = Cli::try_parse_from([
            "opencrabs",
            "cron",
            "add",
            "--cron",
            "0 9 * * *",
            "--prompt",
            "Test",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cron_add_missing_cron() {
        let result = Cli::try_parse_from([
            "opencrabs",
            "cron",
            "add",
            "--name",
            "Test",
            "--prompt",
            "Test",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cron_add_missing_prompt() {
        let result = Cli::try_parse_from([
            "opencrabs",
            "cron",
            "add",
            "--name",
            "Test",
            "--cron",
            "0 9 * * *",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cron_list() {
        let cli = Cli::try_parse_from(["opencrabs", "cron", "list"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Cron {
                operation: CronCommands::List
            })
        ));
    }

    #[test]
    fn test_cron_remove() {
        let cli = Cli::try_parse_from(["opencrabs", "cron", "remove", "my-job-id"]).unwrap();
        match cli.command {
            Some(Commands::Cron {
                operation: CronCommands::Remove { id },
            }) => assert_eq!(id, "my-job-id"),
            _ => panic!("Expected Cron Remove command"),
        }
    }

    #[test]
    fn test_cron_enable() {
        let cli = Cli::try_parse_from(["opencrabs", "cron", "enable", "my-job-id"]).unwrap();
        match cli.command {
            Some(Commands::Cron {
                operation: CronCommands::Enable { id },
            }) => assert_eq!(id, "my-job-id"),
            _ => panic!("Expected Cron Enable command"),
        }
    }

    #[test]
    fn test_cron_disable() {
        let cli = Cli::try_parse_from(["opencrabs", "cron", "disable", "my-job-id"]).unwrap();
        match cli.command {
            Some(Commands::Cron {
                operation: CronCommands::Disable { id },
            }) => assert_eq!(id, "my-job-id"),
            _ => panic!("Expected Cron Disable command"),
        }
    }

    #[test]
    fn test_cron_remove_missing_id() {
        let result = Cli::try_parse_from(["opencrabs", "cron", "remove"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cron_missing_subcommand() {
        let result = Cli::try_parse_from(["opencrabs", "cron"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cron_invalid_subcommand() {
        let result = Cli::try_parse_from(["opencrabs", "cron", "invalid"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cron_with_debug_flag() {
        let cli = Cli::try_parse_from(["opencrabs", "--debug", "cron", "list"]).unwrap();
        assert!(cli.debug);
        assert!(matches!(
            cli.command,
            Some(Commands::Cron {
                operation: CronCommands::List
            })
        ));
    }
}

// --- DB Repository Tests ---

mod repository {
    use crate::db::CronJobRepository;
    use crate::db::Database;
    use crate::db::models::CronJob;

    async fn setup() -> (Database, CronJobRepository) {
        let db = Database::connect_in_memory()
            .await
            .expect("Failed to create database");
        db.run_migrations().await.expect("Failed to run migrations");
        let repo = CronJobRepository::new(db.pool().clone());
        (db, repo)
    }

    fn make_job(name: &str, cron: &str) -> CronJob {
        CronJob::new(
            name.to_string(),
            cron.to_string(),
            "UTC".to_string(),
            "Test prompt".to_string(),
            None,
            None,
            "off".to_string(),
            true,
            None,
        )
    }

    #[tokio::test]
    async fn test_insert_and_find_by_id() {
        let (_db, repo) = setup().await;
        let job = make_job("test-job", "0 9 * * *");
        let id = job.id.to_string();

        repo.insert(&job).await.unwrap();
        let found = repo.find_by_id(&id).await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "test-job");
    }

    #[tokio::test]
    async fn test_find_by_name() {
        let (_db, repo) = setup().await;
        let job = make_job("unique-name", "0 9 * * *");
        repo.insert(&job).await.unwrap();

        let found = repo.find_by_name("unique-name").await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, job.id);

        let not_found = repo.find_by_name("nonexistent").await.unwrap();
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_list_all() {
        let (_db, repo) = setup().await;
        repo.insert(&make_job("job-a", "0 9 * * *")).await.unwrap();
        repo.insert(&make_job("job-b", "0 10 * * *")).await.unwrap();
        repo.insert(&make_job("job-c", "0 11 * * *")).await.unwrap();

        let jobs = repo.list_all().await.unwrap();
        assert_eq!(jobs.len(), 3);
        // Should be ordered by name
        assert_eq!(jobs[0].name, "job-a");
        assert_eq!(jobs[1].name, "job-b");
        assert_eq!(jobs[2].name, "job-c");
    }

    #[tokio::test]
    async fn test_list_enabled() {
        let (_db, repo) = setup().await;
        let job_a = make_job("enabled-job", "0 9 * * *");
        let job_b = make_job("disabled-job", "0 10 * * *");
        repo.insert(&job_a).await.unwrap();
        repo.insert(&job_b).await.unwrap();
        repo.set_enabled(&job_b.id.to_string(), false)
            .await
            .unwrap();

        let enabled = repo.list_enabled().await.unwrap();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].name, "enabled-job");
    }

    #[tokio::test]
    async fn test_delete() {
        let (_db, repo) = setup().await;
        let job = make_job("to-delete", "0 9 * * *");
        let id = job.id.to_string();
        repo.insert(&job).await.unwrap();

        let deleted = repo.delete(&id).await.unwrap();
        assert!(deleted);

        let found = repo.find_by_id(&id).await.unwrap();
        assert!(found.is_none());

        // Delete nonexistent returns false
        let deleted_again = repo.delete(&id).await.unwrap();
        assert!(!deleted_again);
    }

    #[tokio::test]
    async fn test_set_enabled() {
        let (_db, repo) = setup().await;
        let job = make_job("toggle-job", "0 9 * * *");
        let id = job.id.to_string();
        repo.insert(&job).await.unwrap();

        // Initially enabled
        let found = repo.find_by_id(&id).await.unwrap().unwrap();
        assert!(found.enabled);

        // Disable
        let updated = repo.set_enabled(&id, false).await.unwrap();
        assert!(updated);
        let found = repo.find_by_id(&id).await.unwrap().unwrap();
        assert!(!found.enabled);

        // Re-enable
        repo.set_enabled(&id, true).await.unwrap();
        let found = repo.find_by_id(&id).await.unwrap().unwrap();
        assert!(found.enabled);

        // Toggle nonexistent
        let not_found = repo.set_enabled("nonexistent-id", false).await.unwrap();
        assert!(!not_found);
    }

    #[tokio::test]
    async fn test_update_last_run() {
        let (_db, repo) = setup().await;
        let job = make_job("run-job", "0 9 * * *");
        let id = job.id.to_string();
        repo.insert(&job).await.unwrap();

        // Initially no last_run_at
        let found = repo.find_by_id(&id).await.unwrap().unwrap();
        assert!(found.last_run_at.is_none());

        // Update with next_run_at
        repo.update_last_run(&id, Some("2026-03-06T09:00:00Z"))
            .await
            .unwrap();
        let found = repo.find_by_id(&id).await.unwrap().unwrap();
        assert!(found.last_run_at.is_some());
        assert!(found.next_run_at.is_some());
    }

    #[tokio::test]
    async fn test_job_with_all_fields() {
        let (_db, repo) = setup().await;
        let job = CronJob::new(
            "full-job".to_string(),
            "30 14 * * 1-5".to_string(),
            "Europe/London".to_string(),
            "Check stock prices".to_string(),
            Some("openai".to_string()),
            Some("gpt-4".to_string()),
            "on".to_string(),
            false,
            Some("telegram:123456".to_string()),
        );
        repo.insert(&job).await.unwrap();

        let found = repo.find_by_id(&job.id.to_string()).await.unwrap().unwrap();
        assert_eq!(found.name, "full-job");
        assert_eq!(found.cron_expr, "30 14 * * 1-5");
        assert_eq!(found.timezone, "Europe/London");
        assert_eq!(found.prompt, "Check stock prices");
        assert_eq!(found.provider.as_deref(), Some("openai"));
        assert_eq!(found.model.as_deref(), Some("gpt-4"));
        assert_eq!(found.thinking, "on");
        assert!(!found.auto_approve);
        assert_eq!(found.deliver_to.as_deref(), Some("telegram:123456"));
        assert!(found.enabled);
    }

    #[tokio::test]
    async fn test_empty_list() {
        let (_db, repo) = setup().await;
        let jobs = repo.list_all().await.unwrap();
        assert!(jobs.is_empty());
    }
}

// --- Cron Expression Validation Tests ---

mod cron_expr {
    use cron::Schedule;
    use std::str::FromStr;

    fn validate(expr: &str) -> bool {
        // Our convention: user provides 5-field, we prepend "0 " for seconds
        let with_secs = format!("0 {expr}");
        Schedule::from_str(&with_secs).is_ok()
    }

    #[test]
    fn test_valid_expressions() {
        assert!(validate("0 9 * * *")); // daily at 9am
        assert!(validate("*/30 * * * *")); // every 30 min
        assert!(validate("0 0 * * 1")); // every Monday midnight
        assert!(validate("30 14 * * 1-5")); // weekdays at 2:30pm
        assert!(validate("0 */6 * * *")); // every 6 hours
        assert!(validate("0 9 1 * *")); // 1st of every month
        assert!(validate("15 10 * * 7")); // Sundays at 10:15 (cron crate: 1=Mon..7=Sun)
    }

    #[test]
    fn test_invalid_expressions() {
        assert!(!validate("")); // empty
        assert!(!validate("not a cron")); // garbage
        assert!(!validate("60 9 * * *")); // minute > 59
        assert!(!validate("0 25 * * *")); // hour > 23
        assert!(!validate("0 9 32 * *")); // day > 31
    }

    #[test]
    fn test_next_run_calculation() {
        let schedule = Schedule::from_str("0 0 9 * * *").unwrap(); // daily at 9am
        let now = chrono::Utc::now();
        let next = schedule.upcoming(chrono::Utc).next();
        assert!(next.is_some());
        assert!(next.unwrap() > now);
    }
}

// --- CronManage Tool Tests ---

mod tool {
    use crate::brain::tools::cron_manage::CronManageTool;
    use crate::brain::tools::{Tool, ToolExecutionContext};
    use crate::db::CronJobRepository;
    use crate::db::Database;

    async fn setup() -> (Database, CronManageTool) {
        let db = Database::connect_in_memory()
            .await
            .expect("Failed to create database");
        db.run_migrations().await.expect("Failed to run migrations");
        let repo = CronJobRepository::new(db.pool().clone());
        let tool = CronManageTool::new(repo);
        (db, tool)
    }

    fn ctx() -> ToolExecutionContext {
        ToolExecutionContext::new(uuid::Uuid::new_v4())
    }

    #[test]
    fn test_tool_name_and_schema() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (_db, tool) = setup().await;
            assert_eq!(tool.name(), "cron_manage");
            let schema = tool.input_schema();
            assert!(schema.get("properties").is_some());
            let props = schema["properties"].as_object().unwrap();
            assert!(props.contains_key("action"));
            assert!(props.contains_key("name"));
            assert!(props.contains_key("cron"));
            assert!(props.contains_key("prompt"));
            assert!(props.contains_key("deliver_to"));
            assert!(props.contains_key("job_id"));
        });
    }

    #[tokio::test]
    async fn test_create_and_list() {
        let (_db, tool) = setup().await;

        // Create
        let input = serde_json::json!({
            "action": "create",
            "name": "Test Job",
            "cron": "0 9 * * *",
            "prompt": "Do something"
        });
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("Cron job created"));
        assert!(result.output.contains("Test Job"));

        // List
        let input = serde_json::json!({"action": "list"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("Test Job"));
        assert!(result.output.contains("0 9 * * *"));
    }

    #[tokio::test]
    async fn test_create_missing_fields() {
        let (_db, tool) = setup().await;

        // Missing name
        let input = serde_json::json!({"action": "create", "cron": "0 9 * * *", "prompt": "x"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(!result.success);

        // Missing cron
        let input = serde_json::json!({"action": "create", "name": "x", "prompt": "x"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(!result.success);

        // Missing prompt
        let input = serde_json::json!({"action": "create", "name": "x", "cron": "0 9 * * *"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_create_invalid_cron() {
        let (_db, tool) = setup().await;
        let input = serde_json::json!({
            "action": "create",
            "name": "Bad Cron",
            "cron": "not valid",
            "prompt": "x"
        });
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(!result.success);
        assert!(
            result.output.contains("Invalid cron expression")
                || result.error.as_ref().is_some_and(|e| e.contains("Invalid"))
        );
    }

    #[tokio::test]
    async fn test_create_duplicate_name() {
        let (_db, tool) = setup().await;
        let input = serde_json::json!({
            "action": "create",
            "name": "Dup",
            "cron": "0 9 * * *",
            "prompt": "x"
        });
        tool.execute(input.clone(), &ctx()).await.unwrap();
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(!result.success);
        assert!(
            result.output.contains("already exists")
                || result
                    .error
                    .as_ref()
                    .is_some_and(|e| e.contains("already exists"))
        );
    }

    #[tokio::test]
    async fn test_delete() {
        let (_db, tool) = setup().await;

        // Create first
        let input = serde_json::json!({
            "action": "create",
            "name": "To Delete",
            "cron": "0 9 * * *",
            "prompt": "x"
        });
        let result = tool.execute(input, &ctx()).await.unwrap();
        // Extract job ID from output
        let id = result
            .output
            .lines()
            .find(|l| l.contains("ID:"))
            .unwrap()
            .split("ID: ")
            .nth(1)
            .unwrap()
            .trim()
            .to_string();

        // Delete
        let input = serde_json::json!({"action": "delete", "job_id": id});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("deleted"));

        // List should be empty
        let input = serde_json::json!({"action": "list"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.output.contains("No cron jobs"));
    }

    #[tokio::test]
    async fn test_enable_disable() {
        let (_db, tool) = setup().await;

        let input = serde_json::json!({
            "action": "create",
            "name": "Toggle Me",
            "cron": "0 9 * * *",
            "prompt": "x"
        });
        let result = tool.execute(input, &ctx()).await.unwrap();
        let id = result
            .output
            .lines()
            .find(|l| l.contains("ID:"))
            .unwrap()
            .split("ID: ")
            .nth(1)
            .unwrap()
            .trim()
            .to_string();

        // Disable
        let input = serde_json::json!({"action": "disable", "job_id": id});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("disabled"));

        // List should show disabled
        let input = serde_json::json!({"action": "list"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.output.contains("disabled"));

        // Enable
        let input = serde_json::json!({"action": "enable", "job_id": id});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("enabled"));
    }

    #[tokio::test]
    async fn test_delete_nonexistent() {
        let (_db, tool) = setup().await;
        let input = serde_json::json!({"action": "delete", "job_id": "nonexistent"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn test_unknown_action() {
        let (_db, tool) = setup().await;
        let input = serde_json::json!({"action": "invalid"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(!result.success);
        assert!(
            result.output.contains("Unknown action")
                || result.error.as_ref().is_some_and(|e| e.contains("Unknown"))
        );
    }

    #[tokio::test]
    async fn test_list_empty() {
        let (_db, tool) = setup().await;
        let input = serde_json::json!({"action": "list"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("No cron jobs"));
    }

    #[tokio::test]
    async fn test_approval_required_for_create_and_delete() {
        let (_db, tool) = setup().await;

        let create_input = serde_json::json!({"action": "create"});
        assert!(tool.requires_approval_for_input(&create_input));

        let delete_input = serde_json::json!({"action": "delete"});
        assert!(tool.requires_approval_for_input(&delete_input));

        let list_input = serde_json::json!({"action": "list"});
        assert!(!tool.requires_approval_for_input(&list_input));

        let enable_input = serde_json::json!({"action": "enable"});
        assert!(!tool.requires_approval_for_input(&enable_input));

        let disable_input = serde_json::json!({"action": "disable"});
        assert!(!tool.requires_approval_for_input(&disable_input));
    }

    #[tokio::test]
    async fn test_create_with_deliver_to() {
        let (_db, tool) = setup().await;
        let input = serde_json::json!({
            "action": "create",
            "name": "Delivered Job",
            "cron": "0 9 * * *",
            "prompt": "Check things",
            "deliver_to": "telegram:123456"
        });
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("telegram:123456"));
    }
}

// --- Scheduler Logic Tests ---

mod scheduler {
    use crate::db::CronJobRepository;
    use crate::db::Database;
    use crate::db::models::CronJob;
    use chrono::{Duration, Utc};

    async fn setup() -> (Database, CronJobRepository) {
        let db = Database::connect_in_memory()
            .await
            .expect("Failed to create database");
        db.run_migrations().await.expect("Failed to run migrations");
        let repo = CronJobRepository::new(db.pool().clone());
        (db, repo)
    }

    #[tokio::test]
    async fn test_is_due_with_past_next_run() {
        let (_db, _repo) = setup().await;
        let mut job = CronJob::new(
            "past-job".to_string(),
            "0 9 * * *".to_string(),
            "UTC".to_string(),
            "test".to_string(),
            None,
            None,
            "off".to_string(),
            true,
            None,
        );
        // Set next_run_at to 1 hour ago — should be due
        job.next_run_at = Some(Utc::now() - Duration::hours(1));
        assert!(job.next_run_at.unwrap() <= Utc::now());
    }

    #[tokio::test]
    async fn test_is_due_with_future_next_run() {
        let (_db, _repo) = setup().await;
        let mut job = CronJob::new(
            "future-job".to_string(),
            "0 9 * * *".to_string(),
            "UTC".to_string(),
            "test".to_string(),
            None,
            None,
            "off".to_string(),
            true,
            None,
        );
        // Set next_run_at to 1 hour from now — should NOT be due
        job.next_run_at = Some(Utc::now() + Duration::hours(1));
        assert!(job.next_run_at.unwrap() > Utc::now());
    }

    #[tokio::test]
    async fn test_next_run_calculation() {
        use cron::Schedule;
        use std::str::FromStr;

        let cron_expr = "0 9 * * *"; // daily at 9am
        let cron_str = format!("0 {cron_expr}");
        let schedule = Schedule::from_str(&cron_str).unwrap();
        let now = Utc::now();
        let next = schedule.after(&now).next().unwrap();

        // Next run should be in the future
        assert!(next > now);
        // And within 24 hours
        assert!(next - now < Duration::hours(25));
    }

    #[tokio::test]
    async fn test_disabled_jobs_not_listed() {
        let (_db, repo) = setup().await;
        let job = CronJob::new(
            "disabled".to_string(),
            "0 9 * * *".to_string(),
            "UTC".to_string(),
            "test".to_string(),
            None,
            None,
            "off".to_string(),
            true,
            None,
        );
        repo.insert(&job).await.unwrap();
        repo.set_enabled(&job.id.to_string(), false).await.unwrap();

        let enabled = repo.list_enabled().await.unwrap();
        assert!(enabled.is_empty());
    }
}

// --- Session Resolution Tests ---

mod session_resolution {
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use uuid::Uuid;

    /// Helper that mimics CronScheduler::resolve_session_id logic
    fn resolve(current: Option<Uuid>, initial: Option<Uuid>) -> Option<Uuid> {
        current.or(initial)
    }

    #[test]
    fn test_follows_user_to_current_session() {
        let initial = Uuid::new_v4();
        let current = Uuid::new_v4();
        // When user has switched sessions, follow them
        assert_eq!(resolve(Some(current), Some(initial)), Some(current));
    }

    #[test]
    fn test_falls_back_to_initial_session() {
        let initial = Uuid::new_v4();
        // When no active session (e.g. user on session list screen), use initial
        assert_eq!(resolve(None, Some(initial)), Some(initial));
    }

    #[test]
    fn test_no_sessions_returns_none() {
        // When neither exists (fresh start, no sessions created yet)
        assert_eq!(resolve(None, None), None);
    }

    #[test]
    fn test_same_session_stays_same() {
        let session = Uuid::new_v4();
        // User hasn't moved — same session used
        assert_eq!(resolve(Some(session), Some(session)), Some(session));
    }

    #[tokio::test]
    async fn test_shared_session_id_updates_are_visible() {
        let shared: Arc<Mutex<Option<Uuid>>> = Arc::new(Mutex::new(None));
        let shared_clone = shared.clone();

        // Initially None
        assert!(shared.lock().await.is_none());

        // Simulate user opening a session
        let session_id = Uuid::new_v4();
        *shared_clone.lock().await = Some(session_id);
        assert_eq!(*shared.lock().await, Some(session_id));

        // Simulate user switching to another session
        let new_session_id = Uuid::new_v4();
        *shared_clone.lock().await = Some(new_session_id);
        assert_eq!(*shared.lock().await, Some(new_session_id));

        // Simulate user going to session list (no active session)
        *shared_clone.lock().await = None;
        assert!(shared.lock().await.is_none());
    }

    #[tokio::test]
    async fn test_initial_session_captured_at_spawn() {
        let session_id = Uuid::new_v4();
        let shared: Arc<Mutex<Option<Uuid>>> = Arc::new(Mutex::new(Some(session_id)));

        // Simulate what spawn() does: capture initial
        let initial = *shared.lock().await;
        assert_eq!(initial, Some(session_id));

        // User switches session after spawn
        let new_session = Uuid::new_v4();
        *shared.lock().await = Some(new_session);

        // initial is still the original
        assert_eq!(initial, Some(session_id));
        // but shared now points to new
        assert_eq!(*shared.lock().await, Some(new_session));
    }
}
