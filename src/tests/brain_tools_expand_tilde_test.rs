//! Tests for `expand_tilde` accepting the Windows backslash form (`~\...`)
//! in addition to `~/...`. Models on Windows routinely paste native
//! separators after the tilde; without expansion the `~` stays literal and
//! the path silently resolves against the working directory.

use crate::brain::tools::error::expand_tilde;

#[test]
fn expand_tilde_forward_slash_form() {
    let home = dirs::home_dir().expect("home dir");
    assert_eq!(expand_tilde("~"), home);
    assert_eq!(expand_tilde("~/projects/app"), home.join("projects/app"));
}

#[test]
fn expand_tilde_backslash_form_windows() {
    let home = dirs::home_dir().expect("home dir");
    assert_eq!(
        expand_tilde(r"~\.opencrabs\logs"),
        home.join(r".opencrabs\logs")
    );
    // `~\` alone is the home dir, like `~`.
    assert_eq!(expand_tilde(r"~\"), home.join(""));
}

#[test]
fn expand_tilde_leaves_non_tilde_paths_alone() {
    assert_eq!(
        expand_tilde("plain.txt"),
        std::path::PathBuf::from("plain.txt")
    );
    assert_eq!(
        expand_tilde(r"C:\Users\someone\file.rs"),
        std::path::PathBuf::from(r"C:\Users\someone\file.rs")
    );
    // A `~` in the middle is data, not a home reference.
    assert_eq!(expand_tilde("a~b/c"), std::path::PathBuf::from("a~b/c"));
}
