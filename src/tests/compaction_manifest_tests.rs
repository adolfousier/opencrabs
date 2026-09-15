use crate::brain::agent::service::AgentService;
use crate::brain::agent::service::context::parse_context_manifest;
use std::collections::HashSet;

#[test]
fn test_parse_context_manifest_yaml() {
    let summary = r#"
## 0. IMMEDIATE TASK
CONTINUE THIS TASK: Do something.

## 10. Context Manifest
```context-manifest
active_skills:
  - opencrabs-dev
  - coding-process
discard_skills:
  - grill-for-unknowns
required_tools:
  - telegram_send
  - browser_navigate
```
"#;

    let manifest = parse_context_manifest(summary).expect("should parse manifest");
    assert_eq!(
        manifest.active_skills,
        vec!["opencrabs-dev".to_string(), "coding-process".to_string()]
    );
    assert_eq!(
        manifest.discard_skills,
        vec!["grill-for-unknowns".to_string()]
    );
    assert_eq!(
        manifest.required_tools,
        vec!["telegram_send".to_string(), "browser_navigate".to_string()]
    );
}

#[test]
fn test_parse_context_manifest_json_fallback() {
    let summary = r#"
## 10. Context Manifest
```context-manifest
{
  "active_skills": ["meta-factory"],
  "discard_skills": ["a2a-gateway"],
  "required_tools": ["cron_manage"]
}
```
"#;

    let manifest = parse_context_manifest(summary).expect("should parse json manifest");
    assert_eq!(manifest.active_skills, vec!["meta-factory".to_string()]);
    assert_eq!(manifest.discard_skills, vec!["a2a-gateway".to_string()]);
    assert_eq!(manifest.required_tools, vec!["cron_manage".to_string()]);
}

#[test]
fn test_parse_context_manifest_inline_flow_yaml() {
    let summary = r#"
```context-manifest
active_skills: [opencrabs-dev, github]
discard_skills: []
required_tools: [telegram_send]
```
"#;

    let manifest = parse_context_manifest(summary).expect("should parse inline yaml");
    assert_eq!(
        manifest.active_skills,
        vec!["opencrabs-dev".to_string(), "github".to_string()]
    );
    assert!(manifest.discard_skills.is_empty());
    assert_eq!(manifest.required_tools, vec!["telegram_send".to_string()]);
}

#[test]
fn test_parse_context_manifest_missing_or_malformed_fails_soft() {
    let empty_summary = "No manifest here";
    assert_eq!(parse_context_manifest(empty_summary), None);

    let empty_block = "```context-manifest\n```";
    assert_eq!(parse_context_manifest(empty_block), None);

    let malformed_block = "```context-manifest\nrandom gibberish with no colons\n```";
    assert_eq!(parse_context_manifest(malformed_block), None);
}

#[test]
fn test_parse_skill_spec() {
    assert_eq!(
        crate::brain::skills::parse_skill_spec("opencrabs-dev"),
        ("opencrabs-dev".to_string(), None)
    );
    assert_eq!(
        crate::brain::skills::parse_skill_spec("/opencrabs-dev"),
        ("opencrabs-dev".to_string(), None)
    );
    assert_eq!(
        crate::brain::skills::parse_skill_spec("opencrabs-dev/editor.md"),
        ("opencrabs-dev".to_string(), Some("editor.md".to_string()))
    );
    assert_eq!(
        crate::brain::skills::parse_skill_spec("/opencrabs-dev/fleet-directives.md"),
        (
            "opencrabs-dev".to_string(),
            Some("fleet-directives.md".to_string())
        )
    );
}

#[test]
fn test_parse_context_manifest_with_aux_files_and_service_wiring() {
    let summary = r#"
## 10. Context Manifest
```context-manifest
active_skills:
  - opencrabs-dev/editor.md
  - /opencrabs-dev/fleet-directives.md
  - miidas-factory
discard_skills:
  - opencrabs-dev/hq.md
  - /a2a-gateway
required_tools:
  - telegram_send
```
"#;

    let manifest = parse_context_manifest(summary).expect("should parse manifest with aux paths");
    assert_eq!(
        manifest.active_skills,
        vec![
            "opencrabs-dev/editor.md".to_string(),
            "/opencrabs-dev/fleet-directives.md".to_string(),
            "miidas-factory".to_string(),
        ]
    );
    assert_eq!(
        manifest.discard_skills,
        vec![
            "opencrabs-dev/hq.md".to_string(),
            "/a2a-gateway".to_string(),
        ]
    );

    let session_id = uuid::Uuid::new_v4();

    for active in &manifest.active_skills {
        let (slug, aux_file) = crate::brain::skills::parse_skill_spec(active);
        crate::brain::tools::seen_skills::mark_seen(session_id, &slug);
        if let Some(file) = aux_file {
            crate::brain::tools::seen_skills::mark_aux_seen(session_id, &slug, &file);
        }
    }

    let seen = crate::brain::tools::seen_skills::seen_for_session(session_id);
    assert!(seen.iter().any(|s| s == "opencrabs-dev"));
    assert!(seen.iter().any(|s| s == "miidas-factory"));

    let aux = crate::brain::tools::seen_skills::aux_seen_for_session(session_id);
    let dev_aux = aux.get("opencrabs-dev").expect("opencrabs-dev aux files");
    assert_eq!(
        dev_aux,
        &vec!["editor.md".to_string(), "fleet-directives.md".to_string()]
    );

    // Discard single aux file
    let (slug, aux_file) = crate::brain::skills::parse_skill_spec("opencrabs-dev/editor.md");
    if let Some(file) = aux_file {
        crate::brain::tools::seen_skills::unmark_aux_seen(session_id, &slug, &file);
    }
    let aux_after = crate::brain::tools::seen_skills::aux_seen_for_session(session_id);
    let dev_aux_after = aux_after
        .get("opencrabs-dev")
        .expect("opencrabs-dev aux files");
    assert_eq!(dev_aux_after, &vec!["fleet-directives.md".to_string()]);
    // Skill itself remains seen
    assert!(
        crate::brain::tools::seen_skills::seen_for_session(session_id)
            .iter()
            .any(|s| s == "opencrabs-dev")
    );

    // Discard entire skill
    let (slug, aux_file) = crate::brain::skills::parse_skill_spec("opencrabs-dev");
    if aux_file.is_none() {
        crate::brain::tools::seen_skills::clear_aux_seen(session_id, &slug);
        crate::brain::tools::seen_skills::unmark_seen(session_id, &slug);
    }
    assert!(
        !crate::brain::tools::seen_skills::seen_for_session(session_id)
            .iter()
            .any(|s| s == "opencrabs-dev")
    );
    let aux_final = crate::brain::tools::seen_skills::aux_seen_for_session(session_id);
    assert!(!aux_final.contains_key("opencrabs-dev"));
}

#[test]
fn test_format_context_inventory() {
    let mut active_skills = HashSet::new();
    active_skills.insert("cost-estimate".to_string());

    let mut active_tools = HashSet::new();
    active_tools.insert("telegram_send".to_string());

    let table = AgentService::format_context_inventory(
        200_000,
        &active_skills,
        &std::collections::HashMap::new(),
        &active_tools,
        None,
    );
    // The table must render the canonical bare slug. A slashed cell here would
    // contradict the `<skill-slug>` rule documented in the very same prompt —
    // that contradiction is #179 F2, so pin both directions.
    assert!(table.contains("| `cost-estimate` | Skill |"));
    assert!(!table.contains("| `/cost-estimate` | Skill |"));
    assert!(table.contains("| `telegram_send` | Lazy Tool |"));
    assert!(table.contains("Guidance: aim to keep active skills and lazy tools <= 5% target"));
}

#[test]
fn test_format_context_inventory_with_auxiliary() {
    let mut active_skills = HashSet::new();
    active_skills.insert("opencrabs-dev".to_string());

    let mut seen_aux = std::collections::HashMap::new();
    seen_aux.insert("opencrabs-dev".to_string(), vec!["editor.md".to_string()]);

    let active_tools = HashSet::new();

    let table = AgentService::format_context_inventory(
        200_000,
        &active_skills,
        &seen_aux,
        &active_tools,
        None,
    );
    assert!(table.contains("| `opencrabs-dev` | Skill |"));
    assert!(table.contains("| `opencrabs-dev/editor.md` | Skill aux |"));
}
