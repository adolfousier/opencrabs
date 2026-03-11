use crate::tui::render::collapse_build_output;

#[test]
fn collapse_compiling_block() {
    let input = "   Compiling foo v1.0.0\n   Compiling bar v2.0.0\n   Compiling baz v3.0.0";
    let result = collapse_build_output(input);
    assert_eq!(result, vec!["   Compiled 3 crates"]);
}

#[test]
fn collapse_downloading_block() {
    let input = "  Downloading crates ...\n  Downloaded serde v1.0.0\n  Downloaded tokio v1.0.0";
    let result = collapse_build_output(input);
    // Downloading and Downloaded are different verbs → separate groups
    assert_eq!(result.len(), 2);
    assert!(result[0].contains("Downloaded"));
    assert!(result[1].contains("Downloaded"));
}

#[test]
fn collapse_mixed_verbs_separate_summaries() {
    let input = "   Compiling foo v1.0.0\n   Compiling bar v2.0.0\n   Checking baz v3.0.0";
    let result = collapse_build_output(input);
    assert_eq!(result.len(), 2);
    assert!(result[0].contains("Compiled") && result[0].contains('2'));
    assert!(result[1].contains("Compiled") && result[1].contains('1'));
}

#[test]
fn collapse_passthrough_non_build_lines() {
    let input = "error[E0308]: mismatched types\n   Compiling foo v1.0.0\n   Compiling bar v2.0.0\nwarning: unused variable";
    let result = collapse_build_output(input);
    assert_eq!(result.len(), 3);
    assert_eq!(result[0], "error[E0308]: mismatched types");
    assert!(result[1].contains("Compiled 2"));
    assert_eq!(result[2], "warning: unused variable");
}

#[test]
fn collapse_empty_input() {
    let result = collapse_build_output("");
    assert!(result.is_empty() || result == vec![""]);
}

#[test]
fn collapse_no_build_lines_passthrough() {
    let input = "hello world\nfoo bar";
    let result = collapse_build_output(input);
    assert_eq!(result, vec!["hello world", "foo bar"]);
}

#[test]
fn collapse_finished_line_passes_through() {
    let input =
        "   Compiling foo v1.0.0\n    Finished `dev` profile [unoptimized] target(s) in 5.2s";
    let result = collapse_build_output(input);
    assert_eq!(result.len(), 2);
    assert!(result[0].contains("Compiled 1"));
    assert!(result[1].contains("Finished"));
}

#[test]
fn collapse_fresh_and_locking() {
    let input = "       Fresh foo v1.0.0\n       Fresh bar v2.0.0\n     Locking baz v3.0.0";
    let result = collapse_build_output(input);
    assert_eq!(result.len(), 2);
    assert!(result[0].contains("Processed 2"));
    assert!(result[1].contains("Processed 1"));
}

#[test]
fn collapse_large_block() {
    let lines: Vec<String> = (0..100)
        .map(|i| format!("   Compiling crate-{} v0.1.0", i))
        .collect();
    let input = lines.join("\n");
    let result = collapse_build_output(&input);
    assert_eq!(result, vec!["   Compiled 100 crates"]);
}
