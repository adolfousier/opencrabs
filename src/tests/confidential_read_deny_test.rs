//! read_file refuses confidential paths at the harness, not by prompt (OC-08).
//!
//! Confidential-file protection was documented but implemented only as prose in
//! a brain file, so a jailbreak or ignored instruction let read_file hand
//! keys.toml or an SSH private key straight into chat. This is the code-level
//! deny it stood in for.

use crate::brain::tools::confidential::is_confidential;
use std::path::Path;

#[test]
fn the_agents_secret_store_is_confidential() {
    assert!(is_confidential(Path::new("/home/u/.opencrabs/keys.toml")).is_some());
}

#[test]
fn ssh_material_is_confidential() {
    assert!(is_confidential(Path::new("/home/u/.ssh/id_ed25519")).is_some());
    assert!(is_confidential(Path::new("/home/u/.ssh/id_rsa")).is_some());
    assert!(is_confidential(Path::new("/home/u/.ssh/config")).is_some());
    // The whole .ssh directory is off-limits (.ssh/** pattern), including a
    // public key living there; the agent has no reason to read inside it.
    assert!(is_confidential(Path::new("/home/u/.ssh/id_ed25519.pub")).is_some());
    // A .pub file OUTSIDE .ssh is an ordinary file.
    assert!(is_confidential(Path::new("/home/u/exported_key.pub")).is_none());
}

#[test]
fn env_and_key_material_is_confidential() {
    assert!(is_confidential(Path::new("/app/.env")).is_some());
    assert!(is_confidential(Path::new("/app/.env.production")).is_some());
    assert!(is_confidential(Path::new("/etc/ssl/server.pem")).is_some());
    assert!(is_confidential(Path::new("/etc/ssl/server.key")).is_some());
    assert!(is_confidential(Path::new("/etc/shadow")).is_some());
    assert!(is_confidential(Path::new("/vault/aws_credentials")).is_some());
}

#[test]
fn ordinary_files_are_not_confidential() {
    assert!(is_confidential(Path::new("/home/u/project/src/main.rs")).is_none());
    assert!(is_confidential(Path::new("/home/u/notes.md")).is_none());
    assert!(is_confidential(Path::new("/home/u/config.toml")).is_none());
    assert!(is_confidential(Path::new("/home/u/data.json")).is_none());
}

#[test]
fn read_file_wires_the_deny() {
    let src = std::fs::read_to_string("src/brain/tools/read.rs").unwrap();
    assert!(
        src.contains("confidential::is_confidential"),
        "read_file must call the confidential deny (OC-08)"
    );
}
