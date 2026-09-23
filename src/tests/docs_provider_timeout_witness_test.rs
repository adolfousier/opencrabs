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

/// #1688: the resolution chain is part of the contract, not an implementation
/// detail. A user who sets `[agent] timeout_secs` has to be able to read that it
/// does something — the whole bug was that it did something nowhere.
#[test]
fn the_resolution_order_is_documented() {
    let readme = readme();
    for token in [
        "three tiers",
        "`[providers.<name>] timeout_secs`",
        "`[agent] timeout_secs`",
        "`[agent] stream_idle_timeout_secs`",
    ] {
        assert!(
            readme.contains(token),
            "README no longer documents the {token} tier of the timeout \
             resolution chain (#1688). The chain is the user-facing contract; \
             dropping a tier from the docs is how a configured key becomes a \
             silent no-op again."
        );
    }
}

/// Stream idle has no compiled floor on purpose. If the README ever claims one,
/// a user will look for a default that does not exist and mis-read the runtime
/// table as an override.
#[test]
fn stream_idle_is_documented_as_having_no_compiled_floor() {
    let readme = readme();
    assert!(
        readme.contains("no compiled floor"),
        "README must state that `stream_idle_timeout_secs` has no compiled \
         default tier, so an unset key defers to the runtime table (#1688)"
    );
}

/// #1689: the two native families used to read neither key at any tier, which
/// made the README's `[providers.anthropic] timeout_secs = 120` example a
/// documented no-op. If the docs ever stop saying that all three families
/// honour the chain, that example is a lie again and this test is the tripwire.
#[test]
fn every_provider_family_is_documented_as_honouring_the_chain() {
    let readme = readme();
    assert!(
        readme.contains("All three families"),
        "README no longer states that every provider family reads both timeout \
         keys (#1689). The anthropic/gemini half of the contract is the part \
         that was silently missing."
    );
    for family in ["anthropic", "gemini"] {
        assert!(
            readme.contains(&format!("`{family}`")),
            "README no longer names the {family} family alongside the timeout \
             chain (#1689)"
        );
    }
}
