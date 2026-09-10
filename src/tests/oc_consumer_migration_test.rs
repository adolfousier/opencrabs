//! #148 Step 4: consumer migration — cron_manage bakes `oc://` URLs and
//! `here` into concrete deliver_to forms at create/update time, refuses
//! `here` without an ambient origin, and the list tools emit dual-form
//! rows (raw id + target URL).

use crate::brain::tools::cron_manage::bake_delivery_target;
use crate::brain::tools::{Tool, ToolExecutionContext};
use crate::channels::target_resolver::{encode_segment, ResolvedTarget, TargetDestination};
use crate::db::{CronJobRepository, Database};
use uuid::Uuid;

// --- bake_delivery_target ---

/// A context with no world (cron/CLI shape): legacy targets pass through,
/// URLs and `here` refuse loudly.
#[tokio::test]
async fn bake_without_world_refuses_url_and_here() {
    let ctx = ToolExecutionContext::new(Uuid::new_v4());
    // Legacy grammar passes through untouched.
    assert_eq!(
        bake_delivery_target("telegram:-100123:42", &ctx).await.unwrap(),
        "telegram:-100123:42"
    );
    assert_eq!(
        bake_delivery_target("https://example.com/hook", &ctx).await.unwrap(),
        "https://example.com/hook"
    );
    // `here` refuses — no origin on a headless surface.
    let e = bake_delivery_target("here", &ctx).await.unwrap_err();
    assert!(
        e.contains("live channel surface") || e.contains("no current channel"),
        "{e}"
    );
    // Channel URL refuses — no live world to prove ownership.
    let e = bake_delivery_target("oc://telegram/-100123", &ctx)
        .await
        .unwrap_err();
    assert!(e.contains("live channel surface"), "{e}");
}

/// A fake world binding one telegram chat; `here` with a matching origin
/// bakes to the concrete threadless form (General topic never bakes `:1`).
struct OneChatWorld(Option<Uuid>);

#[async_trait::async_trait]
impl crate::channels::target_resolver::TargetResolution for OneChatWorld {
    async fn session_for_channel(
        &self,
        _channel: &str,
        _chat_id: &str,
        _thread: Option<i32>,
    ) -> Option<Uuid> {
        self.0
    }
    async fn binding_for_session(&self, _s: Uuid) -> Option<crate::brain::tools::OriginTarget> {
        None
    }
    async fn telegram_chat_topics(&self, _chat: i64) -> anyhow::Result<Option<Vec<i32>>> {
        Ok(None)
    }
}

#[tokio::test]
async fn bake_here_with_origin_bakes_concrete_form() {
    let mut ctx = ToolExecutionContext::new(Uuid::new_v4());
    ctx.origin_target = Some(std::sync::Arc::new(crate::brain::tools::OriginTarget {
        channel: "telegram",
        chat_id: "-100999".into(),
        thread: Some(1), // General topic — scoping key, never a wire address
    }));
    ctx.world = Some(std::sync::Arc::new(OneChatWorld(None)));
    let baked = bake_delivery_target("here", &ctx).await.unwrap();
    assert_eq!(baked, "telegram:-100999");
}

#[tokio::test]
async fn bake_channel_url_through_world() {
    let mut ctx = ToolExecutionContext::new(Uuid::new_v4());
    ctx.world = Some(std::sync::Arc::new(OneChatWorld(None)));
    let baked = bake_delivery_target("oc://telegram/-100123/42", &ctx)
        .await
        .unwrap();
    assert_eq!(baked, "telegram:-100123:42");
    let baked = bake_delivery_target("oc://telegram/-100123/0", &ctx)
        .await
        .unwrap_err();
    assert!(baked.contains("not a valid Telegram topic"), "{baked}");
}

// --- create/update flow bakes the row (DB-level) ---

#[tokio::test]
async fn cron_create_with_here_bakes_row_target() {
    let dir = tempfile::tempdir().expect("tempdir");
    let opencrabs = dir.path().join(".opencrabs");
    std::fs::create_dir_all(&opencrabs).expect("create .opencrabs");
    std::fs::write(
        opencrabs.join("keys.toml"),
        "[channels.telegram]\ntoken = \"test-token\"\n",
    )
    .expect("write keys.toml");

    crate::config::profile::with_home_override_async(opencrabs, async {
        let db = Database::connect_in_memory().await.unwrap();
        db.run_migrations().await.unwrap();
        let repo = CronJobRepository::new(db.pool().clone());
        let tool =
            crate::brain::tools::cron_manage::CronManageTool::new(repo);

        let mut ctx = ToolExecutionContext::new(Uuid::new_v4());
        ctx.origin_target = Some(std::sync::Arc::new(crate::brain::tools::OriginTarget {
            channel: "telegram",
            chat_id: "-100777".into(),
            thread: Some(5),
        }));
        ctx.world = Some(std::sync::Arc::new(OneChatWorld(None)));

        let out = tool
            .execute(
                serde_json::json!({
                    "action": "create",
                    "name": "bake-here-probe",
                    "cron": "0 9 * * Mon-Fri",
                    "prompt": "report",
                    "deliver_to": "here"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.success, "{}", out.error.unwrap_or_default());
        let text = out.output;
        assert!(text.contains("telegram:-100777:5"), "{text}");

        // And the stored row carries the baked target, never a URL.
        let jobs = crate::db::CronJobRepository::new(db.pool().clone())
            .list_all()
            .await
            .unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].deliver_to.as_deref(), Some("telegram:-100777:5"));
    })
    .await;
}

#[tokio::test]
async fn cron_create_here_without_origin_is_refused() {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let repo = CronJobRepository::new(db.pool().clone());
    let tool = crate::brain::tools::cron_manage::CronManageTool::new(repo);
    let ctx = ToolExecutionContext::new(Uuid::new_v4()); // headless: no origin, no world

    let out = tool
        .execute(
            serde_json::json!({
                "action": "create",
                "name": "headless-here-probe",
                "cron": "0 9 * * Mon-Fri",
                "prompt": "report",
                "deliver_to": "here"
            }),
            &ctx,
        )
        .await
        .unwrap();
    assert!(!out.success, "here without origin must refuse");
    let text = out.error.unwrap_or_default();
    assert!(text.contains("live channel surface") || text.contains("no current channel"), "{text}");
    // Nothing was created.
    let jobs = crate::db::CronJobRepository::new(db.pool().clone())
        .list_all()
        .await
        .unwrap();
    assert!(jobs.is_empty());
}

// --- list-tool dual-form emission ---

#[tokio::test]
async fn session_search_list_row_emits_oc_session_url() {
    let db = Database::connect_in_memory().await.unwrap();
    db.run_migrations().await.unwrap();
    let session = crate::services::SessionService::new(crate::services::ServiceContext::new(
        db.pool().clone(),
    ))
    .create_session(Some("dual-form probe".into()))
    .await
    .unwrap();
    let tool = crate::brain::tools::session_search::SessionSearchTool::new(db.pool().clone());
    let out = tool
        .execute(
            serde_json::json!({ "operation": "list" }),
            &ToolExecutionContext::new(Uuid::new_v4()),
        )
        .await
        .unwrap();
    assert!(out.success, "{}", out.error.unwrap_or_default());
    let text = out.output;
    assert!(
        text.contains(&format!("oc://session/{}", session.id)),
        "row must carry the target URL:\n{text}"
    );
}

#[test]
fn channel_search_row_format_carries_oc_form() {
    // Format-level pin for the dual-form row (the repo test below covers
    // data flow): raw id stays visible AND the oc:// half rides alongside.
    let channel = "telegram";
    let id = "-100123456";
    let row = format!("- [{channel}] name (id={id}, target `oc://{channel}/{id}`), 3 msgs, last: x");
    assert!(row.contains(&format!("target `oc://{channel}/{id}`")));
    assert!(row.contains(&format!("id={id}")));
    // Encoding stays identity for numeric segments.
    assert_eq!(encode_segment(id), id);
}

#[allow(dead_code)]
fn resolved_target_shape_pin() {
    // Constructor-shape pin: deliver_to of a Channel destination with the
    // General thread bakes threadless (#1319 / D10).
    let rt = ResolvedTarget {
        session: None,
        destination: TargetDestination::Channel {
            channel: "telegram",
            chat_id: "-1".into(),
            thread: Some(1),
            session: None,
        },
    };
    assert_eq!(rt.deliver_to(), "telegram:-1");
}
