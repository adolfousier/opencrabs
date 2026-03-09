//! Evolve & Dual Binary Variant Tests
//!
//! Tests for variant detection, health-check CLI, evolve version comparison,
//! and binary naming logic.

use crate::brain::tools::evolve::is_newer;
use crate::cli::{Cli, Commands};
use clap::Parser;

// --- IS_FULL_BUILD compile-time constant ---

#[test]
fn test_is_full_build_matches_features() {
    // When compiled with --all-features, IS_FULL_BUILD should be true.
    // When compiled without local-stt/local-tts, it should be false.
    let expected = cfg!(all(feature = "local-stt", feature = "local-tts"));
    assert_eq!(crate::IS_FULL_BUILD, expected);
}

// --- health-check CLI subcommand ---

#[test]
fn test_health_check_parses() {
    let cli = Cli::try_parse_from(["opencrabs", "health-check"]).unwrap();
    assert!(matches!(cli.command, Some(Commands::HealthCheck)));
}

#[test]
fn test_health_check_hidden_from_help() {
    // health-check should parse but not appear in normal help output
    use clap::CommandFactory;
    let cmd = Cli::command();
    let subcmds: Vec<_> = cmd
        .get_subcommands()
        .filter(|s| !s.is_hide_set())
        .map(|s| s.get_name().to_string())
        .collect();
    assert!(
        !subcmds.contains(&"health-check".to_string()),
        "health-check should be hidden"
    );
}

#[test]
fn test_health_check_takes_no_args() {
    let result = Cli::try_parse_from(["opencrabs", "health-check", "--bogus"]);
    assert!(result.is_err());
}

// --- Version comparison (is_newer) ---

#[test]
fn test_is_newer_basic() {
    assert!(is_newer("0.3.0", "0.2.66"));
    assert!(is_newer("0.2.67", "0.2.66"));
    assert!(is_newer("1.0.0", "0.9.99"));
}

#[test]
fn test_is_newer_equal() {
    assert!(!is_newer("0.2.66", "0.2.66"));
    assert!(!is_newer("1.0.0", "1.0.0"));
}

#[test]
fn test_is_newer_older() {
    assert!(!is_newer("0.2.65", "0.2.66"));
    assert!(!is_newer("0.1.0", "0.2.0"));
}

#[test]
fn test_is_newer_major_bump() {
    assert!(is_newer("2.0.0", "1.99.99"));
    assert!(!is_newer("1.99.99", "2.0.0"));
}

#[test]
fn test_is_newer_patch_only() {
    assert!(is_newer("0.2.67", "0.2.66"));
    assert!(!is_newer("0.2.66", "0.2.67"));
}
