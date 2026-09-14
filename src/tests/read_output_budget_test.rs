//! Per-line clamp and output-byte budget for `read_file` (#986).
//!
//! Without these, a minified one-line bundle or a large default read lands in
//! context whole. Every clamp and every budget stop must announce itself and
//! name the exact resume offset, per the Command Code read_file audit
//! (surfaced by adi805).

use crate::brain::tools::read::ReadTool;
use crate::brain::tools::{Tool, ToolExecutionContext};
use std::io::{BufWriter, Write};
use uuid::Uuid;

async fn read(json: serde_json::Value) -> crate::brain::tools::ToolResult {
    let tool = ReadTool;
    let ctx = ToolExecutionContext::new(Uuid::new_v4());
    tool.execute(json, &ctx).await.unwrap()
}

#[tokio::test]
async fn long_line_is_clamped_with_original_length_announced() {
    let mut f = tempfile::Builder::new().suffix(".js").tempfile().unwrap();
    let long_line = "x".repeat(5_000);
    writeln!(f, "short line\n{long_line}\ntail").unwrap();
    f.flush().unwrap();

    let result = read(serde_json::json!({ "path": f.path().to_str().unwrap() })).await;
    assert!(result.success);
    assert!(
        result
            .output
            .contains("[line truncated: 5000 chars total, showing first 2000]"),
        "clamped line must announce its original length; got: {}",
        &result.output[..result.output.len().min(300)]
    );
    // The tail line survives: clamping is per-line, not whole-file.
    assert!(result.output.contains("tail"));
}

#[tokio::test]
async fn default_read_stops_at_output_budget_with_resume_offset() {
    // ~200 KB of short lines, under the 10 MB large-file threshold, no range:
    // exercises the whole-file budget path.
    let mut f = tempfile::Builder::new().suffix(".log").tempfile().unwrap();
    let total = 4_000usize;
    {
        // Buffered: one writeln! per line straight at the file is one write
        // syscall per line (#1534).
        let mut w = BufWriter::new(f.as_file_mut());
        for i in 0..total {
            writeln!(w, "log entry {i:0>40}").unwrap();
        }
        w.flush().unwrap();
    }

    let result = read(serde_json::json!({ "path": f.path().to_str().unwrap() })).await;
    assert!(result.success);
    let warning = result
        .metadata
        .get("warning")
        .expect("budget-truncated read must carry a warning");
    assert!(
        warning.contains("128 KB output budget"),
        "warning: {warning}"
    );
    assert!(warning.contains("4000 total lines"), "warning: {warning}");
    assert!(
        result.output.len() <= 128 * 1024,
        "budget must bound emitted bytes"
    );
    let resume = warning
        .split("start_line=")
        .nth(1)
        .and_then(|s| s.split(' ').next())
        .and_then(|s| s.parse::<usize>().ok())
        .expect("warning must carry a numeric resume offset");
    let emitted = result.output.lines().count();
    assert_eq!(
        resume, emitted,
        "resume offset must equal the first unread line"
    );
    assert!(emitted < total, "budget must stop before the file ends");
}

#[tokio::test]
async fn explicit_range_bypasses_budget_but_never_the_clamp() {
    // Same ~200 KB file, read with an explicit range: the budget must not
    // fire, while the per-line clamp still applies.
    let mut f = tempfile::Builder::new().suffix(".log").tempfile().unwrap();
    let total = 4_000usize;
    {
        let mut w = BufWriter::new(f.as_file_mut());
        for i in 0..total {
            writeln!(w, "log entry {i:0>40}").unwrap();
        }
        writeln!(w, "{}", "y".repeat(5_000)).unwrap(); // one long line at the end
        w.flush().unwrap();
    }

    let result = read(serde_json::json!({
        "path": f.path().to_str().unwrap(),
        "start_line": 0,
        "line_count": 4001
    }))
    .await;
    assert!(result.success);
    let warning = result
        .metadata
        .get("warning")
        .map(String::as_str)
        .unwrap_or("");
    assert!(
        !warning.contains("output budget"),
        "explicit range must bypass the budget: {warning}"
    );
    assert!(
        warning.contains("exceeded 2000 chars"),
        "clamp must still announce: {warning}"
    );
    assert!(result.output.contains("[line truncated: 5000 chars total"));
}

#[tokio::test]
async fn buffered_budget_path_counts_the_rejected_line() {
    // Over the 10 MB large-file threshold, no range: exercises the BUFFERED
    // budget path (read_with_buffer), unlike the whole-file budget path the
    // test above covers. The line that overflows the budget is consumed by
    // the reader before the break; it must still be counted in the total.
    // Regression for the observed "File has 100004 total lines" under-report.
    let mut f = tempfile::Builder::new().suffix(".log").tempfile().unwrap();
    let total = 100_005usize;
    {
        let mut w = BufWriter::new(f.as_file_mut());
        for i in 0..total {
            writeln!(w, "line {i:0>100}").unwrap();
        }
        w.flush().unwrap();
    }

    let result = read(serde_json::json!({ "path": f.path().to_str().unwrap() })).await;
    assert!(result.success);
    let warning = result
        .metadata
        .get("warning")
        .expect("buffered budget truncation must carry a warning");
    assert!(
        warning.contains("128 KB output budget"),
        "warning: {warning}"
    );
    assert!(
        warning.contains("100005 total lines"),
        "the consumed-but-rejected line must be counted: {warning}"
    );
}
