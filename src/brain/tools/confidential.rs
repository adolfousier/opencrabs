//! Harness-level confidential-path denylist (OC-08).
//!
//! Confidential-file protection was documented as a feature but implemented
//! only as prose in a brain file, i.e. an instruction to the model. A jailbreak,
//! an ignored brain file, or an RSI-weakened SECURITY.md then let `read_file`
//! hand `~/.opencrabs/keys.toml` or `~/.ssh/id_ed25519` straight into the
//! conversation, the DB, and (via a channel) a group chat, with no approval.
//! This is the code-level deny the prose stood in for: it does not depend on the
//! model choosing to obey.
//!
//! `read_file` is where a match matters most, because it returns file contents.
//! The channel-output redactor (OC-05) is the second layer for anything that
//! still reaches a channel; this is the first.

use std::path::Path;

/// The reason `path` is confidential and must not be read through a tool, or
/// None when it is an ordinary file. Matches on the resolved path, so `..` and
/// `~` games do not change the answer.
pub(crate) fn is_confidential(path: &Path) -> Option<&'static str> {
    let full = path.to_string_lossy().to_ascii_lowercase();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    // Anything under a .ssh directory: private keys, known_hosts, config.
    if full.contains("/.ssh/") || full.ends_with("/.ssh") {
        return Some("SSH directory");
    }
    // SSH private keys by name, wherever they live. Public keys (.pub) are fine.
    if (name == "id_rsa"
        || name == "id_ed25519"
        || name == "id_ecdsa"
        || name == "id_dsa"
        || name.starts_with("id_"))
        && !name.ends_with(".pub")
    {
        return Some("SSH private key");
    }
    // The agent's own secret store.
    if name == "keys.toml" {
        return Some("keys.toml (provider keys and channel tokens)");
    }
    // Environment files: .env, .env.local, .env.production, …
    if name.starts_with(".env") {
        return Some("environment file");
    }
    // Private key / certificate material.
    if name.ends_with(".pem") || name.ends_with(".key") || name.ends_with(".pfx") {
        return Some("private key or certificate");
    }
    // System credential stores.
    if full.ends_with("/etc/shadow") || full == "/etc/shadow" || full.ends_with("/etc/gshadow") {
        return Some("system password file");
    }
    // A file that names itself a credential.
    if name.contains("credential") || name.contains("secret") {
        return Some("credentials file");
    }
    None
}
