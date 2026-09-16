//! Tests for the skill glob gate (issue #150): match semantics, exemption,
//! fail-open laws, and pattern resolution.
//!
//! Extracted from an inline `#[cfg(test)]` block in
//! `src/brain/tools/skill_gate.rs`; project policy (CONTRIBUTING.md)
//! requires all tests under `src/tests/`.

use serde_json::json;
use uuid::Uuid;

use crate::brain::skills::{Skill, SkillSource};
use crate::brain::tools::error::expand_tilde;
use crate::brain::tools::seen_skills;
use crate::brain::tools::skill_gate::{
    EXEMPT_TOOLS, GateVerdict, compile_pattern, harvest_candidates,
};

fn skill(globs: &[&str]) -> Skill {
    let fm = format!(
        "---\nname: guard-skill\ndescription: gated\nglobs: {}\n---\nBODY-MARKER\n",
        globs.join(", ")
    );
    Skill::parse("guard-skill", &fm, SkillSource::Builtin).unwrap()
}

fn verdict_with_skills(
    session: Uuid,
    tool: &str,
    input: serde_json::Value,
    skills: Vec<Skill>,
    enabled: bool,
) -> GateVerdict {
    if enabled && !skills.is_empty() && !EXEMPT_TOOLS.contains(&tool) {
        let candidates = harvest_candidates(tool, &input, std::path::Path::new("/work"));
        for s in &skills {
            for g in &s.globs {
                let Ok(pattern) = compile_pattern(g, std::path::Path::new("/work")) else {
                    continue;
                };
                let options = glob::MatchOptions {
                    case_sensitive: false,
                    require_literal_separator: true,
                    require_literal_leading_dot: false,
                };
                for c in &candidates {
                    if pattern.matches_path_with(c, options) {
                        if seen_skills::seen_since_compaction(session, &s.name) {
                            return GateVerdict::Pass;
                        }
                        return GateVerdict::Block {
                            skill: s.name.clone(),
                            matched_path: c.display().to_string(),
                            body: s.prompt_body(),
                            globs: s.globs.clone(),
                        };
                    }
                }
            }
        }
    }
    GateVerdict::Pass
}

#[test]
fn match_blocks_with_body_present() {
    let s = skill(&["**/skills/guard-skill/**"]);
    let v = verdict_with_skills(
        Uuid::new_v4(),
        "edit_file",
        json!({"path": "/root/.opencrabs/skills/guard-skill/SKILL.md", "old_text": "a", "new_text": "b"}),
        vec![s],
        true,
    );
    let GateVerdict::Block {
        body, matched_path, ..
    } = v
    else {
        panic!("expected Block, got {v:?}");
    };
    assert!(body.contains("BODY-MARKER"));
    assert_eq!(matched_path, "/root/.opencrabs/skills/guard-skill/SKILL.md");
}

#[test]
fn seen_skill_passes() {
    let session = Uuid::new_v4();
    let s = skill(&["**/guard/**"]);
    seen_skills::mark_seen(session, "guard-skill");
    let v = verdict_with_skills(
        session,
        "write_file",
        json!({"path": "/x/guard/file.md", "content": "c"}),
        vec![s],
        true,
    );
    assert_eq!(v, GateVerdict::Pass);
}

#[test]
fn fresh_session_blocks() {
    let s = skill(&["**/guard/**"]);
    let v = verdict_with_skills(
        Uuid::new_v4(),
        "write_file",
        json!({"path": "/x/guard/file.md", "content": "c"}),
        vec![s],
        true,
    );
    assert!(matches!(v, GateVerdict::Block { .. }));
}

#[test]
fn exempt_tools_pass() {
    for tool in EXEMPT_TOOLS {
        let s = skill(&["**/**"]);
        let v = verdict_with_skills(
            Uuid::new_v4(),
            tool,
            json!({"path": "/x/guard/file.md"}),
            vec![s],
            true,
        );
        assert_eq!(v, GateVerdict::Pass, "tool {tool} must be exempt");
    }
}

#[test]
fn bash_command_with_matching_path_blocks() {
    let s = skill(&["**/secrets/**"]);
    let v = verdict_with_skills(
        Uuid::new_v4(),
        "bash",
        json!({"command": "cat /etc/secrets/key.pem"}),
        vec![s],
        true,
    );
    assert!(matches!(v, GateVerdict::Block { .. }));
}

#[test]
fn disabled_config_passes() {
    let s = skill(&["**/guard/**"]);
    let v = verdict_with_skills(
        Uuid::new_v4(),
        "write_file",
        json!({"path": "/x/guard/file.md", "content": "c"}),
        vec![s],
        false,
    );
    assert_eq!(v, GateVerdict::Pass);
}

#[test]
fn malformed_glob_fails_open() {
    let s = skill(&["[invalid"]);
    let v = verdict_with_skills(
        Uuid::new_v4(),
        "write_file",
        json!({"path": "/x/guard/file.md", "content": "c"}),
        vec![s],
        true,
    );
    assert_eq!(v, GateVerdict::Pass);
}

#[test]
fn relative_path_resolves_against_cwd() {
    let s = skill(&["/work/guard/**"]);
    let input = json!({"path": "guard/file.md", "content": "c"});
    let candidates = harvest_candidates("write_file", &input, std::path::Path::new("/work"));
    assert!(candidates.contains(&std::path::PathBuf::from("/work/guard/file.md")));
    let v = verdict_with_skills(Uuid::new_v4(), "write_file", input, vec![s], true);
    assert!(matches!(v, GateVerdict::Block { .. }));
}

#[test]
fn star_matches_one_segment_only() {
    // Cursor table via require_literal_separator: `*` must NOT cross `/`.
    let s = skill(&["/work/*"]);
    let v1 = verdict_with_skills(
        Uuid::new_v4(),
        "write_file",
        json!({"path": "/work/file.md", "content": "c"}),
        vec![s.clone()],
        true,
    );
    assert!(matches!(v1, GateVerdict::Block { .. }));
    let v2 = verdict_with_skills(
        Uuid::new_v4(),
        "write_file",
        json!({"path": "/work/sub/file.md", "content": "c"}),
        vec![s],
        true,
    );
    assert_eq!(v2, GateVerdict::Pass);
}

#[test]
fn grep_pattern_never_harvested() {
    let candidates = harvest_candidates(
        "grep",
        &json!({"pattern": "/etc/secrets/**", "path": "/tmp/x"}),
        std::path::Path::new("/work"),
    );
    assert_eq!(candidates, vec![std::path::PathBuf::from("/tmp/x")]);
}

#[test]
fn tilde_pattern_matches_home_path() {
    // Regression: the frontmatter form a skill author actually writes
    // (`~/repo/**`) must match — it must not compile to a literal `~`.
    let s = skill(&["~/guard/**"]);
    let v = verdict_with_skills(
        Uuid::new_v4(),
        "write_file",
        json!({"path": "~/guard/file.md", "content": "c"}),
        vec![s],
        true,
    );
    let GateVerdict::Block { matched_path, .. } = v else {
        panic!("expected Block for a `~/...` glob, got {v:?}");
    };
    let home = expand_tilde("~");
    assert_eq!(
        matched_path,
        home.join("guard").join("file.md").display().to_string()
    );
}

#[test]
fn relative_pattern_resolves_against_cwd() {
    // A bare relative pattern (`guard/**`) resolves against the tool cwd,
    // mirroring how candidate paths are resolved.
    let s = skill(&["guard/**"]);
    let v = verdict_with_skills(
        Uuid::new_v4(),
        "write_file",
        json!({"path": "guard/file.md", "content": "c"}),
        vec![s],
        true,
    );
    assert!(matches!(v, GateVerdict::Block { .. }));
}
