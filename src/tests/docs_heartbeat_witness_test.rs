// Witness for #1621: README.md and src/docs/ shipped a heartbeat subsystem
// the binary never implemented — intervals (`heartbeat.every`, JSON5
// `heartbeat: { every }`), `HEARTBEAT_OK` ack suppression, `ackMaxChars`,
// and config paths under `agents.defaults.*` that the schema does not
// accept. None of it exists. Periodic checks are cron jobs (cron_manage)
// whose prompt reads HEARTBEAT.md; that file is a plain checklist.
//
// This test scans the shipped docs so the phantom claims cannot return.

use std::fs;
use std::path::Path;

/// Phrases that advertise the nonexistent subsystem or schema-invalid
/// config paths. Plain mentions of the HEARTBEAT.md brain file are fine.
const BANNED: &[&str] = &[
    "heartbeat.every",
    "heartbeat: {",
    "HEARTBEAT_OK",
    "ackMaxChars",
    "agents.defaults",
];

fn scanned_docs() -> Vec<(String, String)> {
    let mut out = vec![(
        "README.md".to_string(),
        fs::read_to_string("README.md").expect("README.md readable"),
    )];
    collect_markdown(Path::new("src/docs"), &mut out);
    out
}

fn collect_markdown(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in fs::read_dir(dir).expect("src/docs readable") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_markdown(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "md") {
            let name = path.display().to_string();
            let content = fs::read_to_string(&path).unwrap_or_default();
            out.push((name, content));
        }
    }
}

#[test]
fn docs_never_advertise_the_phantom_heartbeat_subsystem() {
    let docs = scanned_docs();
    assert!(
        docs.len() > 10,
        "doc scan found suspiciously few files ({}) — did the layout move?",
        docs.len()
    );
    for (name, content) in docs {
        for banned in BANNED {
            assert!(
                !content.contains(banned),
                "{name} advertises '{banned}' — the heartbeat subsystem does not \
                 exist in the binary and `agents.defaults.*` is not a valid config \
                 path (see #1621). Periodic checks are cron jobs reading \
                 HEARTBEAT.md."
            );
        }
    }
}
