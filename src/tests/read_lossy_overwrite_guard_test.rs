//! A transformed decode must not arm a whole-file overwrite.
//!
//! `read_file` decodes BOM-aware and falls back to lossy so a stray binary
//! or a PowerShell UTF-16 log does not hard-fail the read. That tolerance
//! must not leak into `write_file`: the decoded text is not a byte-faithful
//! view of the file, so replacing the file with it re-encodes UTF-16 as
//! UTF-8 at best and overwrites a binary with U+FFFD soup at worst.
//!
//! The staleness guard cannot catch it either. `write_file` reads the
//! current file with `read_to_string`, which yields `None` on those same
//! bytes, and `is_stale_write` treats `None` as "not stale". So the
//! partial-view guard (#1168) is the only thing standing there, and it only
//! holds if a lossy read declines to mark the file fully read.

use crate::brain::tools::read::ReadTool;
use crate::brain::tools::write::WriteTool;
use crate::brain::tools::{Tool, ToolExecutionContext};
use serde_json::json;
use uuid::Uuid;

fn ctx_in(dir: &std::path::Path) -> ToolExecutionContext {
    let mut ctx = ToolExecutionContext::new(Uuid::new_v4());
    ctx.working_directory = dir.to_path_buf();
    ctx
}

fn tmp_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lossyguard_{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Read the whole file, then try to replace it. Returns (read_ok, write_ok).
async fn read_then_overwrite(dir: &std::path::Path, name: &str) -> (bool, bool) {
    let ctx = ctx_in(dir);
    let path = dir.join(name);
    let read = ReadTool
        .execute(json!({ "path": path.to_string_lossy() }), &ctx)
        .await
        .unwrap();
    let write = WriteTool
        .execute(
            json!({ "path": path.to_string_lossy(), "content": "replaced" }),
            &ctx,
        )
        .await
        .unwrap();
    (read.success, write.success)
}

#[tokio::test]
async fn a_binary_file_reads_but_does_not_unlock_an_overwrite() {
    let dir = tmp_dir();
    // NUL bytes plus an invalid UTF-8 sequence: a database, not text.
    std::fs::write(dir.join("db.bin"), [0x00, 0x01, 0xFF, 0xFE_u8, 0x00, 0x42]).unwrap();

    let (read_ok, write_ok) = read_then_overwrite(&dir, "db.bin").await;

    assert!(read_ok, "the read itself must still succeed, not hard-fail");
    assert!(
        !write_ok,
        "a lossy read must not arm a wholesale overwrite of a binary file"
    );
    // The bytes are still on disk, untouched.
    assert_eq!(
        std::fs::read(dir.join("db.bin")).unwrap(),
        vec![0x00, 0x01, 0xFF, 0xFE, 0x00, 0x42]
    );
}

#[tokio::test]
async fn a_utf16_file_reads_but_does_not_unlock_an_overwrite() {
    let dir = tmp_dir();
    let mut bytes: Vec<u8> = vec![0xFF, 0xFE];
    for unit in "hello\r\nworld\r\n".encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(dir.join("log.txt"), &bytes).unwrap();

    let (read_ok, write_ok) = read_then_overwrite(&dir, "log.txt").await;

    assert!(read_ok, "UTF-16 must still decode and read cleanly");
    assert!(
        !write_ok,
        "writing decoded text back would silently re-encode the file as UTF-8"
    );
    assert_eq!(std::fs::read(dir.join("log.txt")).unwrap(), bytes);
}

#[tokio::test]
async fn a_plain_utf8_file_still_unlocks_an_overwrite() {
    let dir = tmp_dir();
    std::fs::write(dir.join("notes.md"), "one\ntwo\n").unwrap();

    let (read_ok, write_ok) = read_then_overwrite(&dir, "notes.md").await;

    assert!(read_ok);
    assert!(
        write_ok,
        "the ordinary whole-file read path must keep unlocking overwrites (#1168)"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("notes.md")).unwrap(),
        "replaced"
    );
}

#[tokio::test]
async fn the_read_says_why_the_overwrite_will_refuse() {
    let dir = tmp_dir();
    std::fs::write(dir.join("x.bin"), [0x00, 0xFF, 0xFE_u8]).unwrap();
    let ctx = ctx_in(&dir);

    let result = ReadTool
        .execute(json!({ "path": dir.join("x.bin").to_string_lossy() }), &ctx)
        .await
        .unwrap();

    let note = format!("{:?}", result.metadata);
    assert!(
        note.contains("overwrite_read_confirm"),
        "a refusal the model cannot anticipate becomes a retry loop; got: {note}"
    );
}

#[tokio::test]
async fn an_explicit_confirm_still_overrides_the_guard() {
    let dir = tmp_dir();
    std::fs::write(dir.join("y.bin"), [0x00, 0xFF_u8]).unwrap();
    let ctx = ctx_in(&dir);
    let path = dir.join("y.bin");

    ReadTool
        .execute(json!({ "path": path.to_string_lossy() }), &ctx)
        .await
        .unwrap();
    let write = WriteTool
        .execute(
            json!({
                "path": path.to_string_lossy(),
                "content": "intentional",
                "overwrite_read_confirm": true
            }),
            &ctx,
        )
        .await
        .unwrap();

    assert!(
        write.success,
        "the guard is a safety net, not a lock: an explicit confirm still writes"
    );
}
