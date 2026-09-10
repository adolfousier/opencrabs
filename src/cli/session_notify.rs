//! `opencrabs session notify` — mechanical session notifications for
//! tooling (#23).
//!
//! The CLI is a SEPARATE PROCESS from the daemon that owns the in-memory
//! route table, so the verb posts over the profile's A2A gateway — the
//! daemon's HTTP surface — where the `session/notify` method hands the
//! message to `deliver_to_session`, the same route the agent's
//! session_notify tool uses (including #19 redirect-with-framing for
//! archived/replaced sessions and #1206 parking).
//!
//! Exit codes are the machine contract for tooling (oc-deploy fan-out, #24):
//!
//! | exit | meaning                                             |
//! |------|-----------------------------------------------------|
//! | 0    | delivered / redirected / parked — the message is safe |
//! | 2    | unknown or dead uuid — nothing sent, nothing created |
//! | 3    | refused: target mid-turn and `--interrupt` not set  |
//! | 4    | transport/config: a2a disabled, unreachable, bad response |
//!
//! SENDER LABEL (#23, owner amendment "Overridable"): the CLI lane has no
//! sender session, so the recipient's echo shows the carried label —
//! default "CLI tooling", overridable with `--sender`.

use crate::cli::args::OutputFormat;
use crate::config::Config;
use anyhow::Result;

pub const EXIT_OK: i32 = 0;
pub const EXIT_NO_ROUTE: i32 = 2;
pub const EXIT_REFUSED: i32 = 3;
pub const EXIT_TRANSPORT: i32 = 4;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(
    config: &Config,
    id_raw: &str,
    text: Option<&str>,
    title: Option<&str>,
    sender: Option<&str>,
    interrupt: bool,
    mode: Option<&str>,
    quiet_for_secs: Option<u64>,
    max_delay_secs: Option<u64>,
    confirm: bool,
    status: bool,
    format: OutputFormat,
) -> Result<()> {
    // Status mode (fork #146): poll a notification receipt by id instead of
    // sending. Rides the same A2A surface (); the id
    // is passed as the positional arg,  is unused.
    if status {
        let Some(notify_id) = text else {
            return finish(
                format,
                id_raw,
                "usage_error",
                EXIT_TRANSPORT,
                "--status requires the notification id as the <ID> argument",
            );
        };
        return run_status(config, notify_id, id_raw, format).await;
    }

    // Local usage errors: the gateway would reject these with INVALID_PARAMS
    // anyway, but failing here keeps the journal honest about who noticed.
    let target: uuid::Uuid = match id_raw.parse() {
        Ok(id) => id,
        Err(_) => {
            return finish(
                format,
                id_raw,
                "no_route",
                EXIT_NO_ROUTE,
                &format!("'{id_raw}' is not a valid session UUID"),
            );
        }
    };
    let Some(text) = text else {
        return finish(
            format,
            &target.to_string(),
            "usage_error",
            EXIT_TRANSPORT,
            "--text is required for sending (or pass --status to poll a receipt id)",
        );
    };
    if text.trim().is_empty() {
        return finish(
            format,
            &target.to_string(),
            "transport_error",
            EXIT_TRANSPORT,
            "--text must not be empty",
        );
    }
    // Shared sender validation (fork #146): the SAME policy rules the A2A
    // handler and the agent tool run — one copy, no drift. Failing here
    // keeps the journal honest about who noticed.
    if let Some(raw) = sender {
        let label = raw.trim();
        if label.is_empty() {
            return finish(
                format,
                &target.to_string(),
                "transport_error",
                EXIT_TRANSPORT,
                "--sender must not be empty",
            );
        }
        if let Err(e) = crate::brain::agent::service::notify_policy::validate_sender_label(label) {
            return finish(
                format,
                &target.to_string(),
                "transport_error",
                EXIT_TRANSPORT,
                &format!("--sender: {e}"),
            );
        }
    }
    if !config.a2a.enabled {
        return finish(
            format,
            &target.to_string(),
            "transport_error",
            EXIT_TRANSPORT,
            "the [a2a] gateway is disabled in this profile's config — the daemon cannot be reached",
        );
    }

    // A bind of 0.0.0.0/:: is a listening address, not a connectable one —
    // same-box callers always dial loopback.
    let host = match config.a2a.bind.as_str() {
        "0.0.0.0" | "::" | "[::]" => "127.0.0.1",
        other => other,
    };
    let url = format!("http://{}:{}/a2a/v1", host, config.a2a.port);
    let mut params = serde_json::json!({
        "session_id": target.to_string(),
        "message": text,
        "interrupt": interrupt,
    });
    if let Some(t) = title {
        params["title"] = serde_json::json!(t);
    }
    if let Some(s) = sender {
        params["sender"] = serde_json::json!(s.trim());
    }
    // Delivery policy (fork #146): the full v2 ontology rides to the
    // A2A method, which resolves it through the shared policy module.
    if interrupt || mode.is_some() || quiet_for_secs.is_some() || max_delay_secs.is_some() {
        let mut delivery = serde_json::json!({});
        if let Some(m) = mode {
            delivery["mode"] = serde_json::json!(m);
        }
        if let Some(q) = quiet_for_secs {
            delivery["quiet_for_secs"] = serde_json::json!(q);
        }
        if let Some(m) = max_delay_secs {
            delivery["max_delay_secs"] = serde_json::json!(m);
        }
        params["delivery"] = delivery;
    }
    if confirm {
        params["confirm"] = serde_json::json!(true);
    }
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "session/notify",
        "params": params,
    });

    let mut req = reqwest::Client::new()
        .post(&url)
        .timeout(std::time::Duration::from_secs(10))
        .json(&body);
    if let Some(key) = config.a2a.api_key.as_deref() {
        req = req.bearer_auth(key);
    }

    let (outcome, exit_code, detail) = match req.send().await {
        Err(e) => (
            "transport_error".to_string(),
            EXIT_TRANSPORT,
            format!("cannot reach the A2A gateway at {url}: {e}"),
        ),
        Ok(resp) => {
            let status = resp.status();
            match resp.json::<crate::a2a::types::JsonRpcResponse>().await {
                Err(e) => (
                    "transport_error".into(),
                    EXIT_TRANSPORT,
                    format!("gateway at {url} returned HTTP {status} without a JSON-RPC body: {e}"),
                ),
                Ok(rpc) => {
                    if let Some(err) = rpc.error {
                        (
                            "transport_error".into(),
                            EXIT_TRANSPORT,
                            format!("gateway error {}: {}", err.code, err.message),
                        )
                    } else if let Some(result) = rpc.result {
                        let outcome = result
                            .get("outcome")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("unknown")
                            .to_string();
                        let detail = result
                            .get("detail")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let code = match outcome.as_str() {
                            "delivered" | "parked" | "deferred" => EXIT_OK,
                            "no_route" => EXIT_NO_ROUTE,
                            "refused_in_flight" => EXIT_REFUSED,
                            _ => EXIT_TRANSPORT,
                        };
                        (outcome, code, detail)
                    } else {
                        (
                            "transport_error".into(),
                            EXIT_TRANSPORT,
                            "gateway response carried neither result nor error".into(),
                        )
                    }
                }
            }
        }
    };

    finish(format, &target.to_string(), &outcome, exit_code, &detail)
}

/// Status mode (fork #146): POST the notify id to the A2A
/// `session/notify-status` method and render the receipt lifecycle.
/// Exit codes: 0 = injected (consumed by the machinery) or queued-but-live;
/// 2 = unknown id (not tracked — in-memory receipts die with the process);
/// 4 = transport.
async fn run_status(
    config: &Config,
    notify_id: &str,
    id_raw: &str,
    format: OutputFormat,
) -> Result<()> {
    if notify_id.parse::<uuid::Uuid>().is_err() {
        return finish(
            format,
            id_raw,
            "unknown_id",
            EXIT_NO_ROUTE,
            &format!("'{notify_id}' is not a valid notification UUID"),
        );
    }
    if !config.a2a.enabled {
        return finish(
            format,
            id_raw,
            "transport_error",
            EXIT_TRANSPORT,
            "the [a2a] gateway is disabled in this profile's config — the daemon cannot be reached",
        );
    }
    let host = match config.a2a.bind.as_str() {
        "0.0.0.0" | "::" | "[::]" => "127.0.0.1",
        other => other,
    };
    let url = format!("http://{}:{}/a2a/v1", host, config.a2a.port);
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "session/notify-status",
        "params": { "notify_id": notify_id },
    });
    let mut req = reqwest::Client::new()
        .post(&url)
        .timeout(std::time::Duration::from_secs(10))
        .json(&body);
    if let Some(key) = config.a2a.api_key.as_deref() {
        req = req.bearer_auth(key);
    }
    let (outcome, exit_code, detail) = match req.send().await {
        Err(e) => (
            "transport_error".to_string(),
            EXIT_TRANSPORT,
            format!("cannot reach the A2A gateway at {url}: {e}"),
        ),
        Ok(resp) => {
            let status = resp.status();
            match resp.json::<crate::a2a::types::JsonRpcResponse>().await {
                Err(e) => (
                    "transport_error".into(),
                    EXIT_TRANSPORT,
                    format!("gateway at {url} returned HTTP {status} without a JSON-RPC body: {e}"),
                ),
                Ok(rpc) => {
                    if let Some(err) = rpc.error {
                        (
                            "transport_error".into(),
                            EXIT_TRANSPORT,
                            format!("gateway error {}: {}", err.code, err.message),
                        )
                    } else if let Some(result) = rpc.result {
                        let outcome = result
                            .get("outcome")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("unknown")
                            .to_string();
                        let detail = result
                            .get("detail")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let code = match outcome.as_str() {
                            "injected" | "queued" => EXIT_OK,
                            "unknown_id" => EXIT_NO_ROUTE,
                            _ => EXIT_TRANSPORT,
                        };
                        (outcome, code, detail)
                    } else {
                        (
                            "transport_error".into(),
                            EXIT_TRANSPORT,
                            "gateway response carried neither result nor error".into(),
                        )
                    }
                }
            }
        }
    };
    finish(format, id_raw, &outcome, exit_code, &detail)
}

/// Journal + output + exit. One append-only journal line per invocation,
/// written BEFORE the process exits — the journal, not the exit code, is the
/// durable record (tool-logging law).
fn finish(
    format: OutputFormat,
    target: &str,
    outcome: &str,
    exit_code: i32,
    detail: &str,
) -> Result<()> {
    journal_line(target, outcome, exit_code, detail);
    match format {
        OutputFormat::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "ok": exit_code == EXIT_OK,
                    "target": target,
                    "outcome": outcome,
                    "detail": detail,
                    "exit": exit_code,
                }))?
            );
        }
        _ => {
            if exit_code == EXIT_OK {
                println!("✅ {outcome}: {detail}");
            } else {
                eprintln!("❌ {outcome}: {detail} (exit {exit_code})");
            }
        }
    }
    std::process::exit(exit_code)
}

fn journal_line(target: &str, outcome: &str, exit_code: i32, detail: &str) {
    use std::io::Write;

    let path = crate::logging::log_dir().join("session-notify.journal");
    let ts = chrono::Utc::now().to_rfc3339();
    let caller = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());
    // Truncate detail so one pathological message cannot bloat the line.
    let detail: String = detail.chars().take(500).collect();
    let line = format!(
        "{ts}\tcaller={caller}\ttarget={target}\toutcome={outcome}\texit={exit_code}\tdetail={detail}\n"
    );
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| f.write_all(line.as_bytes()));
}
