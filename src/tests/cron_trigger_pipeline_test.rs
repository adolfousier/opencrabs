//! Tests for trigger-gated cron pipeline with mechanical pre-flight and goal dispatch (issue #233).

use crate::cron::pipeline::{interpolate_template, PipelineExecutor, TriggerOutcome};
use crate::cron::trigger::{TriggerCondition, TriggerResult, TriggerRunner};
use crate::db::models::CronJob;

#[test]
fn test_trigger_condition_parse() {
    assert_eq!(TriggerCondition::parse(None), TriggerCondition::NonEmpty);
    assert_eq!(
        TriggerCondition::parse(Some("non_empty")),
        TriggerCondition::NonEmpty
    );
    assert_eq!(
        TriggerCondition::parse(Some("exit_non_zero")),
        TriggerCondition::ExitNonZero
    );
    assert_eq!(
        TriggerCondition::parse(Some("exit_zero")),
        TriggerCondition::ExitZero
    );
    assert_eq!(
        TriggerCondition::parse(Some("exitzero")),
        TriggerCondition::ExitZero
    );
    assert_eq!(
        TriggerCondition::parse(Some("regex:disk [0-9]+%")),
        TriggerCondition::Regex("disk [0-9]+%".into())
    );
    assert_eq!(
        TriggerCondition::parse(Some("re:ERROR.*")),
        TriggerCondition::Regex("ERROR.*".into())
    );
    assert_eq!(
        TriggerCondition::parse(Some("always")),
        TriggerCondition::Always
    );
    assert_eq!(
        TriggerCondition::parse(Some("unknown_custom")),
        TriggerCondition::NonEmpty
    );
}

#[test]
fn test_trigger_condition_exit_zero() {
    let cond = TriggerCondition::ExitZero;

    let res_success = TriggerResult {
        stdout: "ok".into(),
        stderr: String::new(),
        exit_code: 0,
    };
    assert!(cond.should_fire(&res_success));

    let res_failure = TriggerResult {
        stdout: String::new(),
        stderr: "err".into(),
        exit_code: 1,
    };
    assert!(!cond.should_fire(&res_failure));
}

#[test]
fn test_trigger_condition_regex() {
    let cond = TriggerCondition::Regex("alert:\\s*([0-9]+)".into());

    let res_match = TriggerResult {
        stdout: "alert: 42 anomalies detected".into(),
        stderr: String::new(),
        exit_code: 0,
    };
    assert!(cond.should_fire(&res_match));

    let res_no_match = TriggerResult {
        stdout: "all systems normal".into(),
        stderr: String::new(),
        exit_code: 0,
    };
    assert!(!cond.should_fire(&res_no_match));
}

#[test]
fn test_trigger_condition_non_empty() {
    let cond = TriggerCondition::NonEmpty;

    let res_empty = TriggerResult {
        stdout: String::new(),
        stderr: String::new(),
        exit_code: 0,
    };
    assert!(!cond.should_fire(&res_empty));

    let res_stdout = TriggerResult {
        stdout: "found 1 item".into(),
        stderr: String::new(),
        exit_code: 0,
    };
    assert!(cond.should_fire(&res_stdout));

    let res_stderr = TriggerResult {
        stdout: String::new(),
        stderr: "warning: disk usage high".into(),
        exit_code: 0,
    };
    assert!(cond.should_fire(&res_stderr));

    let res_whitespace_only = TriggerResult {
        stdout: "   \n\t  ".into(),
        stderr: String::new(),
        exit_code: 0,
    };
    assert!(!cond.should_fire(&res_whitespace_only));
}

#[test]
fn test_trigger_condition_exit_non_zero() {
    let cond = TriggerCondition::ExitNonZero;

    let res_zero = TriggerResult {
        stdout: "some output".into(),
        stderr: String::new(),
        exit_code: 0,
    };
    assert!(!cond.should_fire(&res_zero));

    let res_err = TriggerResult {
        stdout: String::new(),
        stderr: "failure".into(),
        exit_code: 1,
    };
    assert!(cond.should_fire(&res_err));

    let res_exit_2 = TriggerResult {
        stdout: String::new(),
        stderr: String::new(),
        exit_code: 2,
    };
    assert!(cond.should_fire(&res_exit_2));
}

#[test]
fn test_trigger_condition_always() {
    let cond = TriggerCondition::Always;

    let res_empty = TriggerResult {
        stdout: String::new(),
        stderr: String::new(),
        exit_code: 0,
    };
    assert!(cond.should_fire(&res_empty));

    let res_err = TriggerResult {
        stdout: "err".into(),
        stderr: String::new(),
        exit_code: 127,
    };
    assert!(cond.should_fire(&res_err));
}

#[test]
fn test_interpolate_template() {
    let result = TriggerResult {
        stdout: "line 1\nline 2".into(),
        stderr: "warn".into(),
        exit_code: 0,
    };

    let tmpl = "Output:\n{output}\nStdout: {stdout}\nStderr: {stderr}\nCode: {exit_code}";
    let interpolated = interpolate_template(tmpl, &result);

    assert!(interpolated.contains("Output:\nline 1\nline 2\nwarn"));
    assert!(interpolated.contains("Stdout: line 1\nline 2"));
    assert!(interpolated.contains("Stderr: warn"));
    assert!(interpolated.contains("Code: 0"));
}

#[tokio::test]
async fn test_trigger_runner_echo() {
    let runner = TriggerRunner::default();
    let res = runner.run("echo 'trigger test'").await.expect("run echo");
    assert_eq!(res.exit_code, 0);
    assert_eq!(res.stdout.trim(), "trigger test");
    assert!(res.stderr.is_empty());
}

#[tokio::test]
async fn test_trigger_runner_exit_code() {
    let runner = TriggerRunner::default();
    let res = runner.run("sh -c 'exit 42'").await.expect("run exit 42");
    assert_eq!(res.exit_code, 42);
}

#[tokio::test]
async fn test_pipeline_evaluate_no_trigger() {
    let job = CronJob::new(
        "no-trigger-job".into(),
        "0 0 * * *".into(),
        "UTC".into(),
        "do something".into(),
        None,
        None,
        "off".into(),
        true,
        None,
        None,
    );

    match PipelineExecutor::evaluate_trigger(&job).await {
        TriggerOutcome::NoTrigger => {}
        other => panic!("expected NoTrigger, got {other:?}"),
    }
}

#[tokio::test]
async fn test_pipeline_evaluate_fired() {
    let job = CronJob::new_with_trigger(
        "fired-job".into(),
        "0 0 * * *".into(),
        "UTC".into(),
        "do something".into(),
        None,
        None,
        "off".into(),
        true,
        None,
        None,
        Some("echo 'work required'".into()),
        Some("non_empty".into()),
        false,
        None,
    );

    match PipelineExecutor::evaluate_trigger(&job).await {
        TriggerOutcome::Fired(res) => {
            assert_eq!(res.stdout.trim(), "work required");
        }
        other => panic!("expected Fired, got {other:?}"),
    }
}

#[tokio::test]
async fn test_pipeline_evaluate_skipped() {
    let job = CronJob::new_with_trigger(
        "skipped-job".into(),
        "0 0 * * *".into(),
        "UTC".into(),
        "do something".into(),
        None,
        None,
        "off".into(),
        true,
        None,
        None,
        Some("true".into()),
        Some("non_empty".into()),
        false,
        None,
    );

    match PipelineExecutor::evaluate_trigger(&job).await {
        TriggerOutcome::Skipped(res) => {
            assert!(res.stdout.is_empty());
        }
        other => panic!("expected Skipped, got {other:?}"),
    }
}

#[tokio::test]
async fn test_cron_set_goal_requires_session() {
    use crate::brain::tools::cron_manage::CronManageTool;
    use crate::brain::tools::r#trait::{Tool, ToolExecutionContext};
    use serde_json::json;

    let db = crate::db::Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let pool = db.pool().clone();
    let repo = crate::db::repository::CronJobRepository::new(pool.clone());
    let tool = CronManageTool::new(repo);
    let ctx = ToolExecutionContext::new(uuid::Uuid::new_v4());

    // 1. Create with set_goal = true but no deliver_to -> REJECTED
    let input_no_deliver = json!({
        "action": "create",
        "name": "goal-job-1",
        "cron": "0 0 * * *",
        "tz": "UTC",
        "prompt": "do something",
        "set_goal": true
    });
    let res = tool.execute(input_no_deliver, &ctx).await.unwrap();
    assert!(!res.success, "set_goal without deliver_to must be rejected");
    assert!(
        res.error
            .unwrap_or_default()
            .contains("set_goal requires oc://session"),
        "error message must explain set_goal requires session target"
    );

    // 2. Create with set_goal = true and channel delivery -> REJECTED
    let input_channel_deliver = json!({
        "action": "create",
        "name": "goal-job-2",
        "cron": "0 0 * * *",
        "tz": "UTC",
        "prompt": "do something",
        "deliver_to": "https://example.com/webhook",
        "set_goal": true
    });
    let res2 = tool.execute(input_channel_deliver, &ctx).await.unwrap();
    assert!(
        !res2.success,
        "set_goal with channel delivery must be rejected"
    );
    assert!(
        res2.error
            .unwrap_or_default()
            .contains("set_goal requires oc://session"),
        "error message must explain channel delivery is passive"
    );

    // 3. Create with set_goal = true and session delivery -> SUCCESS
    let input_session_deliver = json!({
        "action": "create",
        "name": "goal-job-3",
        "cron": "0 0 * * *",
        "tz": "UTC",
        "prompt": "do something",
        "deliver_to": "oc://session/12345678-1234-1234-1234-123456789abc",
        "set_goal": true
    });
    let res3 = tool.execute(input_session_deliver, &ctx).await.unwrap();
    assert!(
        res3.success,
        "set_goal with session delivery must succeed: {:?}",
        res3.error
    );
}
