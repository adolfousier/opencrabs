//! #1574 — `/cd <path>` typed in the TUI must honor its argument, the way
//! the channel and agent-tool dispatchers both do. The extraction is the
//! single place where the argument used to be dropped silently; the
//! applier itself is shared with the picker's confirmation code path, so
//! the two cannot drift.

use crate::tui::app::messaging::cd_args_from_input;

#[test]
fn bare_cd_has_no_argument_and_opens_the_picker() {
    assert_eq!(cd_args_from_input("/cd"), None);
    assert_eq!(cd_args_from_input("/cd   "), None);
}

#[test]
fn path_argument_is_extracted_with_ends_trimmed() {
    assert_eq!(cd_args_from_input("/cd /tmp/x"), Some("/tmp/x"));
    assert_eq!(
        cd_args_from_input("/cd  ~/srv/rs/opencrabs  "),
        Some("~/srv/rs/opencrabs")
    );
    // Directory names may contain spaces: only the ends are trimmed,
    // the interior spacing must survive untouched.
    assert_eq!(cd_args_from_input("/cd /tmp/my dir"), Some("/tmp/my dir"));
}

#[test]
fn commands_that_merely_start_with_cd_are_not_cd() {
    assert_eq!(cd_args_from_input("/cdutils"), None);
    assert_eq!(cd_args_from_input("/goal /tmp"), None);
}
