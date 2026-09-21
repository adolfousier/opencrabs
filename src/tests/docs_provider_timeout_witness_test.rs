// Witness for #1668: `stream_idle_timeout_secs` existed, was honoured by the
// factory and was tested, with zero occurrences in README.md and zero in
// src/docs/. It is the only user-side control over the timer that produces
// "connection likely dropped" (#1666), so an undiscoverable knob left users
// with no lever at all.
//
// This test pins the documentation so the knob cannot go dark again.

use std::fs;
use std::path::Path;

fn readme() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"))
        .expect("README.md must be readable")
}

#[test]
fn the_stream_idle_knob_is_documented() {
    let readme = readme();
    assert!(
        readme.contains("stream_idle_timeout_secs"),
        "stream_idle_timeout_secs is a live per-provider config key with no README entry"
    );
}

/// The three defaults a user needs before the knob means anything: what they
/// get on a CLI or local provider, on the z.ai host with a documented cut, and
/// on every other remote provider.
#[test]
fn the_documented_defaults_name_all_three_tiers() {
    let readme = readme();
    for token in ["3600s", "45s", "20s"] {
        assert!(
            readme.contains(token),
            "README does not state the {token} idle-timeout default tier"
        );
    }
}
