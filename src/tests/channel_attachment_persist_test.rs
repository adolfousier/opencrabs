//! Tests for the shared channel-attachment persistence (#1729): the durable
//! store helper every channel routes through, its filename sanitization, and
//! the telegram delegate pointing at the same canonical root.

use crate::channels::{
    channel_attachments_dir, persist_channel_attachment_in, sanitize_attachment_name,
};

fn temp_base(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "opencrabs_1729_{}_{}_{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn persist_writes_under_platform_dir_with_prefixed_name() {
    let base = temp_base("happy");
    let out = persist_channel_attachment_in(&base, "whatsapp", "photo.jpg", b"jpegbytes");
    let path = out.expect("persist must succeed on a writable base");
    assert_eq!(
        path.parent().map(|p| p.to_path_buf()),
        Some(base.join("whatsapp")),
        "file must land inside the platform subdir"
    );
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    assert!(
        name.ends_with("-photo.jpg"),
        "readable name must survive as the suffix, got: {name}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"jpegbytes");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn hostile_filename_cannot_escape_the_platform_dir() {
    let base = temp_base("traversal");
    let out = persist_channel_attachment_in(&base, "whatsapp", "../../etc/passwd", b"x");
    let path = out.expect("persist must still succeed with a hostile name");
    assert_eq!(
        path.parent().map(|p| p.to_path_buf()),
        Some(base.join("whatsapp")),
        "traversal segments must be flattened, never resolved"
    );
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn sanitize_collapses_dotfiles_and_empty_names() {
    assert_eq!(sanitize_attachment_name(".hidden"), "hidden");
    assert_eq!(sanitize_attachment_name(""), "attachment.bin");
    assert_eq!(sanitize_attachment_name("..."), "attachment.bin");
    assert_eq!(sanitize_attachment_name("a b/c.d"), "a_b_c.d");
}

#[test]
fn unwritable_base_returns_none_not_panic() {
    // A base that is an existing FILE makes create_dir_all fail; persistence
    // is best-effort so the helper must degrade to None (#1729 contract).
    let base = temp_base("blocked");
    std::fs::write(&base, b"i am a file").unwrap();
    let out = persist_channel_attachment_in(&base, "whatsapp", "x.bin", b"x");
    assert!(out.is_none(), "unwritable base must yield None");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn telegram_delegate_resolves_the_same_canonical_root() {
    // #1729 promoted the dir helper to crate::channels; the telegram wrapper
    // must delegate, not resolve a second divergent root.
    assert_eq!(
        crate::channels::telegram::media::channel_attachments_dir(),
        channel_attachments_dir(),
        "telegram delegate must equal the canonical shared root"
    );
}
