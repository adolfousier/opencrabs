//! Small, reusable validators for values that cross the desktop IPC boundary.
//!
//! Every desktop command mutates local state (config, brain files, the cron
//! table, the dynamic-tool table). Before any of those mutations reach disk we
//! bound and shape the incoming strings so a buggy or hostile frontend payload
//! cannot store unbounded garbage, control characters, or malformed
//! identifiers that the agent would later execute.

/// Reject empty, oversized, or control-character-laden free-form text.
pub(crate) fn bounded_text(label: &str, value: &str, max: usize) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if value.len() > max {
        return Err(format!("{label} must be at most {max} characters"));
    }
    if value.chars().any(|c| c.is_control()) {
        return Err(format!("{label} must not contain control characters"));
    }
    Ok(())
}

/// Validate an identifier used as a tool or skill name. It must read like a
/// stable token (letters, digits, `_`, `-`) so it can never collide with a
/// command flag or be smuggled into a shell invocation.
pub(crate) fn identifier(label: &str, value: &str, max: usize) -> Result<(), String> {
    bounded_text(label, value, max)?;
    let mut chars = value.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    let rest_ok = chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !first_ok || !rest_ok {
        return Err(format!(
            "{label} may only contain letters, digits, '_' and '-', and must start with a letter or '_'"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_oversized_text() {
        assert!(bounded_text("prompt", "", 10).is_err());
        assert!(bounded_text("prompt", "   ", 10).is_err());
        assert!(bounded_text("prompt", "ok", 10).is_ok());
        assert!(bounded_text("prompt", "this is way too long", 5).is_err());
    }

    #[test]
    fn rejects_control_characters() {
        assert!(bounded_text("prompt", "line\nbreak", 100).is_err());
        assert!(bounded_text("prompt", "clean text", 100).is_ok());
    }

    #[test]
    fn accepts_well_formed_identifiers() {
        assert!(identifier("tool", "read_file", 64).is_ok());
        assert!(identifier("tool", "my-tool-2", 64).is_ok());
        assert!(identifier("tool", "_under", 64).is_ok());
    }

    #[test]
    fn rejects_identifiers_that_could_be_smuggled() {
        // leading digit, shell metacharacters, and spaces are all refused.
        assert!(identifier("tool", "2tool", 64).is_err());
        assert!(identifier("tool", "rm -rf", 64).is_err());
        assert!(identifier("tool", "tool;oops", 64).is_err());
        assert!(identifier("tool", "", 64).is_err());
        assert!(identifier("tool", "waytoolongwaytoolongwaytoolongwaytoolongX", 32).is_err());
    }
}
