//! Tests for atomic file writes in brain tools (issue #167).
//!
//! Verifies that atomic_write_file replaces files atomically via rename,
//! resulting in a new inode (so running scripts / open file descriptors
//! are not truncated in place) while preserving original file permissions.

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use tempfile::tempdir;

use crate::brain::tools::fs_util::atomic_write_file;

#[tokio::test]
async fn test_atomic_write_creates_new_file() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("new_file.txt");

    atomic_write_file(&file_path, b"hello world")
        .await
        .expect("atomic write new file");

    let content = fs::read_to_string(&file_path).expect("read file");
    assert_eq!(content, "hello world");
}

#[tokio::test]
async fn test_atomic_write_changes_inode_and_preserves_content() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("script.sh");

    fs::write(&file_path, b"#!/bin/sh\necho original\n").expect("initial write");

    #[cfg(unix)]
    let original_inode = {
        let meta = fs::metadata(&file_path).expect("metadata");
        meta.ino()
    };

    atomic_write_file(&file_path, b"#!/bin/sh\necho updated\n")
        .await
        .expect("atomic overwrite");

    let content = fs::read_to_string(&file_path).expect("read updated file");
    assert_eq!(content, "#!/bin/sh\necho updated\n");

    #[cfg(unix)]
    {
        let new_inode = fs::metadata(&file_path).expect("metadata").ino();
        assert_ne!(
            original_inode, new_inode,
            "atomic_write_file must replace file with new inode to prevent in-place script truncation"
        );
    }
}

#[tokio::test]
#[cfg(unix)]
async fn test_atomic_write_preserves_permissions() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("executable.sh");

    fs::write(&file_path, b"#!/bin/sh\necho run\n").expect("write");

    // Set 0755 permissions
    let mut perms = fs::metadata(&file_path).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&file_path, perms).expect("set permissions");

    let original_mode = fs::metadata(&file_path)
        .expect("metadata")
        .permissions()
        .mode();

    atomic_write_file(&file_path, b"#!/bin/sh\necho new run\n")
        .await
        .expect("atomic overwrite");

    let new_perms = fs::metadata(&file_path).expect("metadata").permissions();
    assert_eq!(
        original_mode & 0o777,
        new_perms.mode() & 0o777,
        "atomic_write_file must preserve existing permissions (mode 0755)"
    );
}

#[tokio::test]
async fn test_atomic_write_no_temp_leak_on_success() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("clean.txt");

    atomic_write_file(&file_path, b"content")
        .await
        .expect("write");

    let entries: Vec<_> = fs::read_dir(dir.path())
        .expect("read_dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();

    assert_eq!(entries, vec!["clean.txt"]);
}
