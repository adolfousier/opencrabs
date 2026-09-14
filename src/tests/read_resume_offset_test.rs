//! #988 — truncation warnings must carry the exact resume offset.
//!
//! The original warning said "use start_line and line_count for pagination"
//! without saying FROM WHERE, so the model reconstructed the cut point by
//! counting output lines (expensive) or guessed, paying twice for re-reads
//! or skipping content.
//!
//! Since #986, a no-range read of a large file stops at the 128 KB output
//! budget long before the 100k line ceiling, so this test pins the
//! MAX_LINES truncation path explicitly: `start_line` present, `line_count`
//! absent. That combination bypasses the byte budget (user-driven window)
//! while `truncated` still fires at MAX_LINES.

use crate::brain::tools::read::ReadTool;
use crate::brain::tools::{Tool, ToolExecutionContext};
use std::io::{BufWriter, Write};
use uuid::Uuid;

#[tokio::test]
async fn truncation_warning_names_the_exact_resume_offset() {
    // 100_005 lines x ~106 bytes ≈ 10.6 MB: over the 10 MB large-file
    // threshold (buffered path), over MAX_LINES lines, and over the output
    // budget — which is exactly why the explicit start_line below matters.
    let mut f = tempfile::Builder::new().suffix(".log").tempfile().unwrap();
    let total = 100_005usize;
    {
        // Buffered: one writeln! per line straight at the file is one write
        // syscall per line (#1534).
        let mut w = BufWriter::new(f.as_file_mut());
        for i in 0..total {
            writeln!(w, "line {i:0>100}").unwrap();
        }
        w.flush().unwrap();
    }
    let path = f.path().to_str().unwrap().to_string();

    let tool = ReadTool;
    let ctx = ToolExecutionContext::new(Uuid::new_v4());
    // Explicit start, no line_count: budget bypassed, MAX_LINES is the cap,
    // and `truncated` still applies. The window is 100_000 lines, so the
    // first unread line is exactly 100_000.
    let result = tool
        .execute(serde_json::json!({ "path": path, "start_line": 0 }), &ctx)
        .await
        .expect("explicit-start read of large file must succeed");

    assert!(result.success, "large-file read must succeed");
    let warning = result
        .metadata
        .get("warning")
        .expect("truncated read must carry a warning");
    assert!(
        warning.contains("Output truncated at 100000 lines"),
        "warning must name the MAX_LINES cap: {warning}"
    );
    assert!(
        warning.contains("File has 100005 total lines"),
        "warning must name the true line total: {warning}"
    );
    assert!(
        warning.contains("Resume with start_line=100000 (0-indexed)"),
        "warning must name the exact resume offset: {warning}"
    );
}
