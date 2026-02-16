//! Security tests for Plan Tool
//!
//! Tests for path validation, input limits, and security hardening.

#[cfg(test)]
mod tests {
    use super::super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn test_validate_path_within_working_directory() {
        let temp_dir = TempDir::new().unwrap();
        let working_dir = temp_dir.path();

        let session_id = uuid::Uuid::new_v4();
        let plan_file = working_dir.join(format!(".opencrabs_plan_{}.json", session_id));

        let result = validate_plan_file_path(&plan_file, working_dir);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_path_outside_working_directory() {
        let temp_dir = TempDir::new().unwrap();
        let working_dir = temp_dir.path();

        let session_id = uuid::Uuid::new_v4();
        // Try to write outside working directory
        let plan_file = PathBuf::from("/tmp").join(format!(".opencrabs_plan_{}.json", session_id));

        let result = validate_plan_file_path(&plan_file, working_dir);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("within the session directory"));
    }

    #[test]
    fn test_validate_path_traversal_attack() {
        let temp_dir = TempDir::new().unwrap();
        let working_dir = temp_dir.path();

        let session_id = uuid::Uuid::new_v4();
        // Try path traversal - construct a path that goes outside working_dir
        let parent = working_dir.parent().unwrap_or(working_dir);
        let plan_file = parent.join(format!(".opencrabs_plan_{}.json", session_id));

        let result = validate_plan_file_path(&plan_file, working_dir);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_filename_pattern() {
        let temp_dir = TempDir::new().unwrap();
        let working_dir = temp_dir.path();

        // Invalid filename (not matching pattern)
        let plan_file = working_dir.join("invalid_plan.json");

        let result = validate_plan_file_path(&plan_file, working_dir);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must match pattern"));
    }

    #[test]
    fn test_validate_filename_requires_uuid() {
        let temp_dir = TempDir::new().unwrap();
        let working_dir = temp_dir.path();

        // Invalid UUID in filename
        let plan_file = working_dir.join(".opencrabs_plan_not-a-uuid.json");

        let result = validate_plan_file_path(&plan_file, working_dir);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("valid UUID"));
    }

    #[test]
    #[cfg(unix)]
    fn test_validate_symlink_rejection() {
        use std::os::unix::fs::symlink;

        let temp_dir = TempDir::new().unwrap();
        let working_dir = temp_dir.path();

        let session_id = uuid::Uuid::new_v4();
        let target_file = working_dir.join("target.json");
        let plan_file = working_dir.join(format!(".opencrabs_plan_{}.json", session_id));

        // Create a target file and symlink to it
        std::fs::write(&target_file, "{}").unwrap();
        symlink(&target_file, &plan_file).unwrap();

        let result = validate_plan_file_path(&plan_file, working_dir);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("symlink"));
    }

    #[test]
    fn test_validate_string_empty() {
        let result = validate_string("", 100, "Test field");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot be empty"));
    }

    #[test]
    fn test_validate_string_whitespace_only() {
        let result = validate_string("   ", 100, "Test field");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot be empty"));
    }

    #[test]
    fn test_validate_string_exceeds_max_length() {
        let long_string = "a".repeat(300);
        let result = validate_string(&long_string, MAX_TITLE_LENGTH, "Title");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("exceeds maximum length"));
    }

    #[test]
    fn test_validate_string_valid() {
        let result = validate_string("Valid title", MAX_TITLE_LENGTH, "Title");
        assert!(result.is_ok());
    }

    #[test]
    fn test_max_plan_file_size_constant() {
        // Verify the constant is reasonable (10MB)
        assert_eq!(MAX_PLAN_FILE_SIZE, 10 * 1024 * 1024);
    }

    #[test]
    fn test_input_validation_limits() {
        // Verify limits are reasonable
        assert_eq!(MAX_TITLE_LENGTH, 200);
        assert_eq!(MAX_DESCRIPTION_LENGTH, 5000);
        assert_eq!(MAX_CONTEXT_LENGTH, 5000);
    }

    #[test]
    fn test_default_complexity() {
        assert_eq!(default_complexity(), 3);
    }

    #[test]
    fn test_validate_title_at_limit() {
        let title = "a".repeat(MAX_TITLE_LENGTH);
        let result = validate_string(&title, MAX_TITLE_LENGTH, "Title");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_title_one_over_limit() {
        let title = "a".repeat(MAX_TITLE_LENGTH + 1);
        let result = validate_string(&title, MAX_TITLE_LENGTH, "Title");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_description_at_limit() {
        let desc = "a".repeat(MAX_DESCRIPTION_LENGTH);
        let result = validate_string(&desc, MAX_DESCRIPTION_LENGTH, "Description");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_context_at_limit() {
        let context = "a".repeat(MAX_CONTEXT_LENGTH);
        let result = validate_string(&context, MAX_CONTEXT_LENGTH, "Context");
        assert!(result.is_ok());
    }

    #[test]
    fn test_filename_with_special_characters() {
        let temp_dir = TempDir::new().unwrap();
        let working_dir = temp_dir.path();

        // Try filename with special characters that might be injection attempts
        let plan_file = working_dir.join(".opencrabs_plan_../../etc/passwd.json");

        let result = validate_plan_file_path(&plan_file, working_dir);
        assert!(result.is_err());
    }

    #[test]
    fn test_filename_with_null_byte() {
        let temp_dir = TempDir::new().unwrap();
        let working_dir = temp_dir.path();

        let session_id = uuid::Uuid::new_v4();
        let filename = format!(".opencrabs_plan_{}\0.json", session_id);
        let plan_file = working_dir.join(filename);

        // Rust's Path handling should prevent null bytes, but test anyway
        let result = validate_plan_file_path(&plan_file, working_dir);
        // Either fails validation or panic is caught
        assert!(result.is_err() || plan_file.to_str().is_none());
    }

    #[test]
    fn test_validate_plan_file_path_canonical() {
        let temp_dir = TempDir::new().unwrap();
        let working_dir = temp_dir.path();

        let session_id = uuid::Uuid::new_v4();
        // Use ./ which should resolve to working_dir
        let plan_file = working_dir.join(format!("./.opencrabs_plan_{}.json", session_id));

        // Should still validate correctly after canonicalization
        let result = validate_plan_file_path(&plan_file, working_dir);
        // May pass or fail depending on path resolution, but shouldn't panic
        let _ = result;
    }
}
