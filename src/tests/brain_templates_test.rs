use crate::tui::onboarding::TEMPLATE_FILES;

#[test]
fn template_files_contains_soul() {
    assert!(
        TEMPLATE_FILES.iter().any(|(name, _)| *name == "SOUL.md"),
        "TEMPLATE_FILES must include SOUL.md"
    );
}

#[test]
fn template_files_contains_identity() {
    assert!(
        TEMPLATE_FILES
            .iter()
            .any(|(name, _)| *name == "IDENTITY.md"),
        "TEMPLATE_FILES must include IDENTITY.md"
    );
}

#[test]
fn template_files_contains_code_md() {
    assert!(
        TEMPLATE_FILES.iter().any(|(name, _)| *name == "CODE.md"),
        "TEMPLATE_FILES must include CODE.md (added in 6b4677b)"
    );
}

#[test]
fn template_files_contains_security_md() {
    assert!(
        TEMPLATE_FILES
            .iter()
            .any(|(name, _)| *name == "SECURITY.md"),
        "TEMPLATE_FILES must include SECURITY.md (added in 6b4677b)"
    );
}

#[test]
fn template_files_contains_memory() {
    assert!(
        TEMPLATE_FILES.iter().any(|(name, _)| *name == "MEMORY.md"),
        "TEMPLATE_FILES must include MEMORY.md"
    );
}

#[test]
fn template_files_all_have_content() {
    for (name, content) in TEMPLATE_FILES {
        assert!(
            !content.trim().is_empty(),
            "Template {} must have non-empty content",
            name
        );
    }
}

#[test]
fn brain_files_in_memory_index_contains_code() {
    // Memory indexer's BRAIN_FILES array (src/memory/index.rs)
    // must include CODE.md so it gets indexed for semantic search.
    use crate::memory::BRAIN_FILES;
    assert!(
        BRAIN_FILES.contains(&"CODE.md"),
        "Memory index BRAIN_FILES must include CODE.md"
    );
}

#[test]
fn brain_files_in_memory_index_contains_security() {
    use crate::memory::BRAIN_FILES;
    assert!(
        BRAIN_FILES.contains(&"SECURITY.md"),
        "Memory index BRAIN_FILES must include SECURITY.md"
    );
}
