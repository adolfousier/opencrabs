//! Regression tests for read_file encoding tolerance.
//!
//! On Windows, non-UTF-8 text is routine: PowerShell `>` writes UTF-16LE
//! with a BOM, and native tools emit stray code-page or binary bytes. The
//! old `read_to_string`/`lines()` paths hard-failed with "stream did not
//! contain valid UTF-8" — the top read failure class on the Windows
//! ledger. Reads now decode BOM-aware and fall back to lossy with a
//! warning instead of failing.

use crate::brain::tools::read::ReadTool;
use crate::brain::tools::{Tool, ToolExecutionContext};
use uuid::Uuid;

async fn read(path: &std::path::Path) -> crate::brain::tools::ToolResult {
    let tool = ReadTool;
    let ctx = ToolExecutionContext::new(Uuid::new_v4());
    tool.execute(serde_json::json!({ "path": path.to_string_lossy() }), &ctx)
        .await
        .unwrap()
}

async fn read_ranged(
    path: &std::path::Path,
    start_line: usize,
    line_count: usize,
) -> crate::brain::tools::ToolResult {
    let tool = ReadTool;
    let ctx = ToolExecutionContext::new(Uuid::new_v4());
    tool.execute(
        serde_json::json!({
            "path": path.to_string_lossy(),
            "start_line": start_line,
            "line_count": line_count
        }),
        &ctx,
    )
    .await
    .unwrap()
}

/// Write UTF-16LE bytes with a BOM, exactly like PowerShell's `>` redirect.
fn write_utf16le(path: &std::path::Path, text: &str) {
    let mut bytes: Vec<u8> = vec![0xFF, 0xFE];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

#[tokio::test]
async fn utf16le_file_with_bom_decodes() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("powershell_out.txt");
    write_utf16le(&p, "line one\nline two wörld\n");

    let result = read(&p).await;
    assert!(
        result.success,
        "UTF-16LE read must succeed: {}",
        result.output
    );
    assert!(
        result.output.contains("line two wörld"),
        "content must decode correctly: {}",
        result.output
    );
    let warning = result
        .metadata
        .get("warning")
        .map(String::as_str)
        .unwrap_or("");
    assert!(
        warning.contains("UTF-16LE"),
        "warning must name the decode: {}",
        warning
    );
}

#[tokio::test]
async fn ranged_read_of_utf16le_file_works() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("ps_ranged.txt");
    write_utf16le(&p, "alpha\nbeta\ngamma\n");

    let result = read_ranged(&p, 1, 1).await;
    assert!(
        result.success,
        "ranged UTF-16 read must succeed: {}",
        result.output
    );
    assert!(
        result.output.contains("beta") && !result.output.contains("alpha"),
        "range window must be exact: {}",
        result.output
    );
}

#[tokio::test]
async fn utf8_bom_is_stripped() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("bom.txt");
    std::fs::write(&p, b"\xEF\xBB\xBFhello bom\n").unwrap();

    let result = read(&p).await;
    assert!(result.success);
    assert!(
        result.output.starts_with("hello bom"),
        "BOM must not leak into content: {:?}",
        result.output
    );
}

#[tokio::test]
async fn invalid_utf8_falls_back_lossy_with_warning() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("not-utf8.log");
    // 0xFF/0xFE in the MIDDLE of otherwise-ASCII text: not a BOM, not valid
    // UTF-8 — the classic stray-code-page-byte case.
    std::fs::write(&p, b"good line\ncaf\xE9 m\xFFangle\n").unwrap();

    let result = read(&p).await;
    assert!(
        result.success,
        "lossy fallback must not fail the read: {}",
        result.output
    );
    assert!(
        result.output.contains("good line"),
        "readable lines must survive: {}",
        result.output
    );
    let warning = result
        .metadata
        .get("warning")
        .map(String::as_str)
        .unwrap_or("");
    assert!(
        warning.contains("invalid UTF-8"),
        "warning must name the fallback: {}",
        warning
    );
}

#[tokio::test]
async fn plain_ascii_read_has_no_encoding_warning() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("plain.txt");
    std::fs::write(&p, b"just text\n").unwrap();

    let result = read(&p).await;
    assert!(result.success);
    assert_eq!(result.output, "just text");
    assert!(
        !result.metadata.contains_key("warning"),
        "no warning for plain UTF-8: {:?}",
        result.metadata.get("warning")
    );
}

#[tokio::test]
async fn binary_file_read_is_lossy_not_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("blob.marker");
    std::fs::write(&p, b"\0\0\x01\x02bin\xFF\xFE\0data").unwrap();

    let result = read(&p).await;
    assert!(result.success, "binary read must not be fatal");
    let warning = result
        .metadata
        .get("warning")
        .map(String::as_str)
        .unwrap_or("");
    assert!(
        warning.contains("binary"),
        "binary hint expected: {}",
        warning
    );
}

#[tokio::test]
async fn a_truncated_utf16_file_announces_the_dropped_half_unit() {
    // A UTF-16 file cut mid code unit: the last byte has no pair. `as_chunks`
    // drops it, so the decoded text is one character short of the file. Say
    // so, because text that is quietly not the file is the worse failure.
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("cut.txt");
    let mut bytes: Vec<u8> = vec![0xFF, 0xFE];
    for unit in "abc".encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes.push(0x64); // half of 'd'
    std::fs::write(&p, bytes).unwrap();

    let result = read(&p).await;
    assert!(result.success);
    let warning = result
        .metadata
        .get("warning")
        .map(String::as_str)
        .unwrap_or("");
    assert!(
        warning.contains("truncated"),
        "expected the dropped half unit to be announced, got: {warning}"
    );
    assert!(result.output.contains("abc"));
}

#[tokio::test]
async fn a_whole_utf16_file_reports_no_truncation() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("whole.txt");
    let mut bytes: Vec<u8> = vec![0xFF, 0xFE];
    for unit in "abcd".encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(&p, bytes).unwrap();

    let result = read(&p).await;
    assert!(result.success);
    let warning = result
        .metadata
        .get("warning")
        .map(String::as_str)
        .unwrap_or("");
    assert!(warning.contains("UTF-16LE"), "got: {warning}");
    assert!(
        !warning.contains("truncated"),
        "an intact file must not claim truncation: {warning}"
    );
    assert!(result.output.contains("abcd"));
}
