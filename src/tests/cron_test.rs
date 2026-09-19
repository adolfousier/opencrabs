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
                assert_eq!(prompt, Some("Check emails and summarize".to_string()));
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
                assert_eq!(prompt, Some("Hello".to_string()));
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
    fn test_cron_add_with_trigger_cmd_without_prompt() {
        let cli = Cli::try_parse_from([
            "opencrabs",
            "cron",
            "add",
            "--name",
            "Test",
            "--cron",
            "0 9 * * *",
            "--trigger-cmd",
            "git status",
        ])
        .unwrap();
        match cli.command {
            Some(Commands::Cron {
                operation:
                    CronCommands::Add {
                        name,
                        cron,
                        prompt,
                        trigger_cmd,
                        ..
                    },
            }) => {
                assert_eq!(name, "Test");
                assert_eq!(cron, "0 9 * * *");
                assert_eq!(prompt, None);
                assert_eq!(trigger_cmd, Some("git status".to_string()));
            }
            _ => panic!("Expected Cron Add command"),
        }
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
    use crate::db::models::CronJob;
    use crate::db::repository::CronJobPatch;
    use crate::db::CronJobRepository;
    use crate::db::Database;

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
            None,
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

    // --- update_fields (patch-in-place, #966) ---

    #[tokio::test]
    async fn test_update_fields_partial_patch_keeps_everything_else() {
        let (_db, repo) = setup().await;
        let job = make_job("patch-job", "0 9 * * *");
        let id = job.id.to_string();
        repo.insert(&job).await.unwrap();

        // The exact Adi use case: change the provider override, keep prompt.
        let patch = CronJobPatch {
            provider: Some(Some("openai".to_string())),
            ..Default::default()
        };
        let updated = repo.update_fields(&id, patch).await.unwrap();
        assert!(updated);

        let found = repo.find_by_id(&id).await.unwrap().unwrap();
        assert_eq!(found.id.to_string(), id, "row id must never change");
        assert_eq!(found.provider.as_deref(), Some("openai"));
        assert_eq!(found.prompt, "Test prompt", "prompt must stay intact");
        assert_eq!(found.cron_expr, "0 9 * * *");
        assert_eq!(found.timezone, "UTC");
        assert!(found.enabled);
    }

    #[tokio::test]
    async fn test_update_fields_clears_optional_override() {
        let (_db, repo) = setup().await;
        let mut job = make_job("clear-job", "0 9 * * *");
        job.provider = Some("anthropic".to_string());
        let id = job.id.to_string();
        repo.insert(&job).await.unwrap();

        // Some(None) = clear back to NULL.
        let patch = CronJobPatch {
            provider: Some(None),
            ..Default::default()
        };
        repo.update_fields(&id, patch).await.unwrap();

        let found = repo.find_by_id(&id).await.unwrap().unwrap();
        assert!(found.provider.is_none());
    }

    #[tokio::test]
    async fn test_update_fields_reset_next_run() {
        let (_db, repo) = setup().await;
        let job = make_job("reset-job", "0 9 * * *");
        let id = job.id.to_string();
        repo.insert(&job).await.unwrap();

        // trigger_now stamps next_run_at; a schedule change must clear it so
        // the scheduler recomputes from the new expression.
        repo.trigger_now(&id).await.unwrap();
        let found = repo.find_by_id(&id).await.unwrap().unwrap();
        assert!(found.next_run_at.is_some());

        let patch = CronJobPatch {
            cron_expr: Some("30 18 * * Mon-Fri".to_string()),
            reset_next_run: true,
            ..Default::default()
        };
        repo.update_fields(&id, patch).await.unwrap();

        let found = repo.find_by_id(&id).await.unwrap().unwrap();
        assert_eq!(found.cron_expr, "30 18 * * Mon-Fri");
        assert!(
            found.next_run_at.is_none(),
            "reset_next_run must NULL next_run_at"
        );
    }

    #[tokio::test]
    async fn test_update_fields_empty_patch_is_noop() {
        let (_db, repo) = setup().await;
        let job = make_job("noop-job", "0 9 * * *");
        let id = job.id.to_string();
        repo.insert(&job).await.unwrap();

        let updated = repo
            .update_fields(&id, CronJobPatch::default())
            .await
            .unwrap();
        assert!(!updated, "empty patch must not write");
    }

    #[tokio::test]
    async fn test_update_fields_nonexistent_job() {
        let (_db, repo) = setup().await;
        let patch = CronJobPatch {
            prompt: Some("x".to_string()),
            ..Default::default()
        };
        let updated = repo.update_fields("no-such-id", patch).await.unwrap();
        assert!(!updated);
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
    async fn test_create_shows_next_runs_for_validation() {
        let (_db, tool) = setup().await;
        let input = serde_json::json!({
            "action": "create",
            "name": "Weekday Briefing",
            "cron": "0 9 * * Mon-Fri",
            "tz": "America/New_York",
            "prompt": "brief me"
        });
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.success);
        // The confirmation feedback the agent reads back to catch a wrong
        // schedule before telling the user it's set.
        assert!(
            result.output.contains("Next runs:"),
            "create must echo upcoming run times: {}",
            result.output
        );
        assert!(
            result.output.contains("Verify"),
            "create must prompt the agent to verify the schedule: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn test_create_rejects_unknown_timezone() {
        let (_db, tool) = setup().await;
        let input = serde_json::json!({
            "action": "create",
            "name": "Bad TZ",
            "cron": "0 9 * * *",
            "tz": "Mars/Phobos",
            "prompt": "x"
        });
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(!result.success);
        assert!(
            result.output.contains("Unknown timezone")
                || result
                    .error
                    .as_ref()
                    .is_some_and(|e| e.contains("Unknown timezone")),
            "unknown tz must be rejected (it's honored by the scheduler now): {:?}",
            result.output
        );
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
        // Delete without confirm - should show warning
        let input = serde_json::json!({"action": "delete", "job_id": id});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("DELETE REQUEST"));
        assert!(result.output.contains("confirm=true"));

        // Delete with confirm - should actually delete
        let input = serde_json::json!({"action": "delete", "job_id": id, "confirm": true});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("deleted"));
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
        assert!(tool.requires_approval_for_input(&disable_input)); // disable requires approval now
    }

    #[tokio::test]
    async fn test_create_with_deliver_to() {
        // The create path fail-fast-validates deliver_to against the channel
        // credential (#107), so the test home must carry a telegram token in
        // keys.toml — exactly what a working deployment has. Async override:
        // the validation awaits nothing, but the test body is async and
        // task-locals survive `.await` only via the async scope.
        let dir = tempfile::tempdir().expect("tempdir");
        let opencrabs = dir.path().join(".opencrabs");
        std::fs::create_dir_all(&opencrabs).expect("create .opencrabs");
        std::fs::write(
            opencrabs.join("keys.toml"),
            "[channels.telegram]\ntoken = \"test-token\"\n",
        )
        .expect("write keys.toml");

        crate::config::profile::with_home_override_async(opencrabs, async {
            let (_db, tool) = setup().await;
            let input = serde_json::json!({
                "action": "create",
                "name": "Delivered Job",
                "cron": "0 9 * * *",
                "prompt": "Check things",
                "deliver_to": "telegram:123456"
            });
            let result = tool.execute(input, &ctx()).await.unwrap();
            assert!(result.success, "create rejected: {}", result.output);
            assert!(result.output.contains("telegram:123456"));
        })
        .await;
    }

    // --- update action (#966) ---

    async fn create_job(tool: &CronManageTool, name: &str) -> String {
        let input = serde_json::json!({
            "action": "create",
            "name": name,
            "cron": "0 9 * * *",
            "prompt": "Keep this prompt"
        });
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.success, "setup create failed: {}", result.output);
        // Pull the id back out of the confirmation output.
        result
            .output
            .lines()
            .find_map(|l| l.trim().strip_prefix("ID: "))
            .expect("create output must contain the job ID")
            .to_string()
    }

    #[tokio::test]
    async fn test_update_provider_keeps_prompt_and_id() {
        let (_db, tool) = setup().await;
        let id = create_job(&tool, "Adi Job").await;

        // The incident scenario: switch the provider override without
        // touching the prompt (previously required raw sqlite).
        let input = serde_json::json!({
            "action": "update",
            "job_id": id,
            "provider": "openrouter"
        });
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.success, "update failed: {}", result.output);
        assert!(result.output.contains("provider -> openrouter"));
        assert!(result.output.contains("updated"));

        // Verify through list that the prompt survived.
        let list = tool
            .execute(serde_json::json!({"action": "list"}), &ctx())
            .await
            .unwrap();
        assert!(list.output.contains("Keep this prompt"));
    }

    #[tokio::test]
    async fn test_update_accepts_name_as_job_id() {
        let (_db, tool) = setup().await;
        create_job(&tool, "Named Job").await;

        let input = serde_json::json!({
            "action": "update",
            "job_id": "Named Job",
            "thinking": "on"
        });
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.success, "update by name failed: {}", result.output);
        assert!(result.output.contains("thinking -> on"));
    }

    #[tokio::test]
    async fn test_update_schedule_recomputes_next_runs() {
        let (_db, tool) = setup().await;
        let id = create_job(&tool, "Reschedule Me").await;

        let input = serde_json::json!({
            "action": "update",
            "job_id": id,
            "cron": "30 18 * * Mon-Fri",
            "tz": "Europe/London"
        });
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.success, "update failed: {}", result.output);
        assert!(
            result.output.contains("Next runs (recomputed)"),
            "a changed schedule must echo recomputed fire times: {}",
            result.output
        );
        assert!(result.output.contains("Verify"));
    }

    #[tokio::test]
    async fn test_update_rejects_invalid_cron() {
        let (_db, tool) = setup().await;
        let id = create_job(&tool, "Bad Cron").await;

        let input = serde_json::json!({
            "action": "update",
            "job_id": id,
            "cron": "99 99 * * *"
        });
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(!result.success);
        assert!(
            result.output.contains("Invalid cron expression")
                || result
                    .error
                    .as_ref()
                    .is_some_and(|e| e.contains("Invalid cron expression")),
            "got: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn test_update_rejects_unknown_timezone() {
        let (_db, tool) = setup().await;
        let id = create_job(&tool, "Bad TZ Update").await;

        let input = serde_json::json!({
            "action": "update",
            "job_id": id,
            "tz": "Mars/Phobos"
        });
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(!result.success);
        assert!(
            result.output.contains("Unknown timezone")
                || result
                    .error
                    .as_ref()
                    .is_some_and(|e| e.contains("Unknown timezone")),
            "got: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn test_update_clears_provider_with_empty_string() {
        let (_db, tool) = setup().await;
        let id = create_job(&tool, "Clear Override").await;

        let set = serde_json::json!({
            "action": "update",
            "job_id": id,
            "provider": "openai"
        });
        assert!(tool.execute(set, &ctx()).await.unwrap().success);

        let clear = serde_json::json!({
            "action": "update",
            "job_id": id,
            "provider": ""
        });
        let result = tool.execute(clear, &ctx()).await.unwrap();
        assert!(result.success, "clear failed: {}", result.output);
        assert!(result.output.contains("provider -> (cleared)"));
    }

    #[tokio::test]
    async fn test_update_with_no_fields_errors() {
        let (_db, tool) = setup().await;
        let id = create_job(&tool, "Nothing To Do").await;

        let input = serde_json::json!({"action": "update", "job_id": id});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(!result.success);
        assert!(
            result.output.contains("Nothing to update")
                || result
                    .error
                    .as_ref()
                    .is_some_and(|e| e.contains("Nothing to update")),
            "got: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn test_update_identical_values_is_a_noop_success() {
        let (_db, tool) = setup().await;
        let id = create_job(&tool, "Same Same").await;

        let input = serde_json::json!({
            "action": "update",
            "job_id": id,
            "cron": "0 9 * * *",
            "prompt": "Keep this prompt"
        });
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.success);
        assert!(
            result.output.contains("unchanged"),
            "identical values must report a no-op: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn test_update_missing_job() {
        let (_db, tool) = setup().await;
        let input = serde_json::json!({
            "action": "update",
            "job_id": "ghost",
            "provider": "openai"
        });
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(!result.success);
        assert!(
            result.output.contains("No cron job found")
                || result
                    .error
                    .as_ref()
                    .is_some_and(|e| e.contains("No cron job found")),
            "got: {}",
            result.output
        );
    }

    #[tokio::test]
    async fn test_update_requires_approval() {
        let (_db, tool) = setup().await;
        let input = serde_json::json!({"action": "update", "job_id": "x"});
        assert!(tool.requires_approval_for_input(&input));
    }
}

// --- Scheduler Logic Tests ---

mod scheduler {
    use crate::db::models::CronJob;
    use crate::db::CronJobRepository;
    use crate::db::Database;
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
            None,
        );
        // Set next_run_at to 1 hour ago — should be due
        job.next_run_at = Some(Utc::now() - Duration::hours(1));
        assert!(job.next_run_at.unwrap() <= Utc::now());
    }

    #[tokio::test]
    async fn test_next_run_anchored_to_now_not_past_date() {
        use chrono::TimeZone;
        use cron::Schedule;
        use std::str::FromStr;

        // Regression test for #432: cron_manage(action="test") sets next_run_at
        // to year 2000. The scheduler must calculate next run from `now`, not from
        // the ancient timestamp, to avoid infinite catch-up loops.
        let cron_expr = "0 4 * * *"; // daily at 4am
        let cron_str = format!("0 {cron_expr}");
        let schedule = Schedule::from_str(&cron_str).unwrap();

        // Simulate the fix: calculate next run from `now` (not from year 2000)
        let now = Utc::now();
        let next_from_now = schedule.after(&now).next().unwrap();

        // Next run should be in the future (relative to now)
        assert!(
            next_from_now > now,
            "Next run anchored to `now` must be in the future"
        );

        // And within 24 hours (daily schedule)
        assert!(
            next_from_now - now < Duration::hours(25),
            "Next run should be within 24 hours for daily schedule"
        );

        // Contrast: if we anchored to year 2000, next run would be in year 2000
        let year_2000 = Utc.with_ymd_and_hms(2000, 1, 1, 0, 0, 0).unwrap();
        let next_from_2000 = schedule.after(&year_2000).next().unwrap();

        // This would be in year 2000, not in the future
        assert!(
            next_from_2000 < now,
            "Next run anchored to year 2000 would be in the past (the bug)"
        );
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
            None,
        );
        repo.insert(&job).await.unwrap();
        repo.set_enabled(&job.id.to_string(), false).await.unwrap();

        let enabled = repo.list_enabled().await.unwrap();
        assert!(enabled.is_empty());
    }

    /// Background `/rebuild` (2026-06-07): scheduling queues a single
    /// reserved one-shot job carrying the session id to resume, and
    /// scheduling again REPLACES it rather than stacking a second build.
    #[tokio::test]
    async fn schedule_background_rebuild_queues_one_shot_resumable_job() {
        use crate::cron::{schedule_background_rebuild, REBUILD_JOB_NAME};

        let (db, repo) = setup().await;
        let sid = uuid::Uuid::new_v4();

        schedule_background_rebuild(db.pool().clone(), sid, Some("telegram:123".into()))
            .await
            .unwrap();

        let rebuilds: Vec<_> = repo
            .list_all()
            .await
            .unwrap()
            .into_iter()
            .filter(|j| j.name == REBUILD_JOB_NAME)
            .collect();
        assert_eq!(rebuilds.len(), 1, "exactly one rebuild job queued");
        assert_eq!(
            rebuilds[0].prompt,
            sid.to_string(),
            "carries the originating session id so the restart resumes it"
        );
        assert!(
            rebuilds[0].enabled,
            "must be enabled so the scheduler runs it"
        );
        assert_eq!(rebuilds[0].deliver_to.as_deref(), Some("telegram:123"));

        // Scheduling again must REPLACE, never stack two concurrent builds.
        let sid2 = uuid::Uuid::new_v4();
        schedule_background_rebuild(db.pool().clone(), sid2, None)
            .await
            .unwrap();

        let rebuilds2: Vec<_> = repo
            .list_all()
            .await
            .unwrap()
            .into_iter()
            .filter(|j| j.name == REBUILD_JOB_NAME)
            .collect();
        assert_eq!(rebuilds2.len(), 1, "re-scheduling replaces, never stacks");
        assert_eq!(rebuilds2[0].prompt, sid2.to_string());
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

// --- Cron Config Default Provider Resolution Tests ---

mod config_defaults {
    use crate::config::CronConfig;

    /// Simulate the provider resolution logic from execute_job:
    /// job override > config default > None (system default)
    fn resolve_provider(job_provider: Option<&str>, config: &CronConfig) -> Option<String> {
        job_provider
            .map(|s| s.to_string())
            .or_else(|| config.default_provider.clone())
    }

    fn resolve_model(job_model: Option<&str>, config: &CronConfig) -> Option<String> {
        job_model
            .map(|s| s.to_string())
            .or_else(|| config.default_model.clone())
    }

    #[test]
    fn test_job_provider_takes_priority() {
        let config = CronConfig {
            default_provider: Some("minimax".to_string()),
            default_model: Some("MiniMax-M2.7".to_string()),
        };
        // Job has explicit provider — config default ignored
        assert_eq!(
            resolve_provider(Some("anthropic"), &config),
            Some("anthropic".to_string())
        );
    }

    #[test]
    fn test_config_default_used_when_job_has_none() {
        let config = CronConfig {
            default_provider: Some("minimax".to_string()),
            default_model: Some("MiniMax-M2.7".to_string()),
        };
        // Job has no provider — falls back to config default
        assert_eq!(resolve_provider(None, &config), Some("minimax".to_string()));
        assert_eq!(
            resolve_model(None, &config),
            Some("MiniMax-M2.7".to_string())
        );
    }

    #[test]
    fn test_no_config_default_returns_none() {
        let config = CronConfig::default();
        // No job provider, no config default — returns None (system default)
        assert_eq!(resolve_provider(None, &config), None);
        assert_eq!(resolve_model(None, &config), None);
    }

    #[test]
    fn test_job_model_overrides_config_default() {
        let config = CronConfig {
            default_provider: Some("minimax".to_string()),
            default_model: Some("MiniMax-M2.7".to_string()),
        };
        assert_eq!(
            resolve_model(Some("MiniMax-M2.5"), &config),
            Some("MiniMax-M2.5".to_string())
        );
    }

    #[test]
    fn test_cron_config_default_is_empty() {
        let config = CronConfig::default();
        assert!(config.default_provider.is_none());
        assert!(config.default_model.is_none());
    }
}

// --- BLOB-typed prompt regression (Mission Control empty-schedule bug) ---
// 2026-05-17: 4 of 7 cron_jobs rows had `prompt` stored as BLOB instead
// of TEXT (sqlx-era binding glitch — SQLite's flexible typing accepted
// the byte-buffer insert despite the `TEXT NOT NULL` schema). The
// `row.get::<String, _>("prompt")` decode then failed, list_all returned
// Err, Mission Control silently swallowed the error and rendered "No
// scheduled jobs." Defensive fix: `text_or_blob_col` reads either storage
// class. Data heal: migration 20260517...cron_jobs_text_recast.
#[cfg(test)]
mod blob_prompt_regression {
    use crate::db::repository::CronJobRepository;
    use rusqlite::params;

    async fn pool_with_cron_table() -> deadpool_sqlite::Pool {
        let cfg = deadpool_sqlite::Config::new(":memory:");
        let pool = cfg
            .create_pool(deadpool_sqlite::Runtime::Tokio1)
            .expect("create pool");
        pool.get()
            .await
            .unwrap()
            .interact(|conn| {
                conn.execute_batch(
                    "CREATE TABLE cron_jobs (
                        id TEXT PRIMARY KEY NOT NULL,
                        name TEXT NOT NULL,
                        cron_expr TEXT NOT NULL,
                        timezone TEXT NOT NULL DEFAULT 'UTC',
                        prompt TEXT NOT NULL,
                        provider TEXT,
                        model TEXT,
                        thinking TEXT NOT NULL DEFAULT 'off',
                        auto_approve INTEGER NOT NULL DEFAULT 1,
                        deliver_to TEXT,
                        deliver_api_key TEXT,
                        enabled INTEGER NOT NULL DEFAULT 1,
                        last_run_at TEXT,
                        next_run_at TEXT,
                        created_at TEXT NOT NULL,
                        updated_at TEXT NOT NULL,
                        profile_name TEXT,
                        trigger_cmd TEXT,
                        trigger_on TEXT,
                        set_goal INTEGER NOT NULL DEFAULT 0,
                        goal_template TEXT
                    );",
                )
            })
            .await
            .unwrap()
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn list_all_tolerates_blob_typed_prompt() {
        let pool = pool_with_cron_table().await;
        // Insert one row with prompt bound as BYTES (mimics the sqlx-era
        // glitch) and one with prompt as a normal string. Both should
        // come back as String through the tolerant reader.
        pool.get()
            .await
            .unwrap()
            .interact(|conn| {
                let prompt_bytes: &[u8] = b"Check wacore crates.io update";
                conn.execute(
                    "INSERT INTO cron_jobs (id, name, cron_expr, timezone, prompt, thinking,
                        auto_approve, enabled, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        "11111111-1111-1111-1111-111111111111",
                        "blob row",
                        "0 9 * * *",
                        "UTC",
                        prompt_bytes,
                        "off",
                        1,
                        1,
                        "2026-05-01T00:00:00Z",
                        "2026-05-01T00:00:00Z",
                    ],
                )?;
                conn.execute(
                    "INSERT INTO cron_jobs (id, name, cron_expr, timezone, prompt, thinking,
                        auto_approve, enabled, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        "22222222-2222-2222-2222-222222222222",
                        "text row",
                        "0 10 * * *",
                        "UTC",
                        "GitHub repo stats",
                        "off",
                        1,
                        1,
                        "2026-05-01T00:00:00Z",
                        "2026-05-01T00:00:00Z",
                    ],
                )?;
                Ok::<_, rusqlite::Error>(())
            })
            .await
            .unwrap()
            .unwrap();

        // Sanity-check the storage-class typeof — proves the test
        // scenario actually reproduces the bug (without the tolerant
        // reader, list_all would fail on the blob row).
        let typeofs: Vec<String> = pool
            .get()
            .await
            .unwrap()
            .interact(|conn| {
                let mut stmt =
                    conn.prepare("SELECT typeof(prompt) FROM cron_jobs ORDER BY name")?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(typeofs, vec!["blob".to_string(), "text".to_string()]);

        let repo = CronJobRepository::new(pool);
        let jobs = repo.list_all().await.expect("list_all must tolerate BLOB");
        assert_eq!(jobs.len(), 2);
        let blob_job = jobs
            .iter()
            .find(|j| j.name == "blob row")
            .expect("blob row found");
        assert_eq!(blob_job.prompt, "Check wacore crates.io update");
        let text_job = jobs
            .iter()
            .find(|j| j.name == "text row")
            .expect("text row found");
        assert_eq!(text_job.prompt, "GitHub repo stats");
    }
}
