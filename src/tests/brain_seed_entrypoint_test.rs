//! Regression tests for entrypoint brain seeding (#1382).
//!
//! Bug: brain templates only reached disk when the onboarding wizard
//! completed. Daemon-only, docker, channel-first, `init`, and aborted
//! wizard installs ran with an empty brain forever — users saw a
//! personality-less agent. Fix: `ensure_brain_seeded()` at every CLI
//! entrypoint, reusing the never-overwrite `seed_brain_templates`.
//!
//! These tests drive the REAL `ensure_brain_seeded()` (resolution included)
//! through the profile-home override into a throwaway directory, so they
//! never touch the developer's actual `~/.opencrabs/`.
//!
//! The home is resolved ONCE and handed to both the seeding call and the
//! assertions (#1536). Each test used to resolve it twice and independently —
//! `home_for_profile()` to build the expected path, then `with_profile_home()`
//! recomputing it internally — and both reads end at `dirs::home_dir()`. Any
//! `$HOME` swap landing between them seeded one directory while the assertion
//! read another, so `SOUL.md` was genuinely absent from the path checked.
//!
//! A `TempDir` rather than a named profile under the real base directory: the
//! old helper wrote to `~/.opencrabs/profiles/brain-seed-test-*` and tidied up
//! at the end, which leaves the directory behind on any panic (#1535 is the
//! same mistake next door). The override is task-local, so unlike pointing
//! `$HOME` at a tempdir it stays correct under a parallel run (#912).

const ALL_NINE: [&str; 9] = [
    "SOUL.md",
    "USER.md",
    "AGENTS.md",
    "TOOLS.md",
    "MEMORY.md",
    "CODE.md",
    "SECURITY.md",
    "BOOT.md",
    "HEARTBEAT.md",
];

/// Run `f` against a brain home that exists only for this test.
///
/// `f` receives the path, so nothing recomputes it: the one value is both what
/// `ensure_brain_seeded()` writes to and what the assertions read.
///
/// `ensure_brain_seeded()` resolves through `opencrabs_home()` alone and never
/// consults the profile NAME, so the home override is the whole requirement
/// here; scoping a name as well would imply a dependency that does not exist.
fn in_throwaway_home<T>(f: impl FnOnce(&std::path::Path) -> T) -> T {
    let home = tempfile::TempDir::new().expect("tempdir");
    let path = home.path().to_path_buf();
    crate::config::profile::with_home_override(path.clone(), || f(&path))
}

#[test]
fn empty_home_gets_full_brain_on_first_open() {
    in_throwaway_home(|home| {
        crate::config::profile::ensure_brain_seeded();
        for f in ALL_NINE {
            assert!(home.join(f).exists(), "first open must seed {f}");
        }
        // Belief base rides along (#881) — without it the Orient gate is inert.
        assert!(
            home.join("safety").join("brain_verify.toml").exists(),
            "first open must seed safety/brain_verify.toml"
        );
    });
}

#[test]
fn reseed_never_overwrites_user_content() {
    in_throwaway_home(|home| {
        crate::config::profile::ensure_brain_seeded();
        let soul = home.join("SOUL.md");
        std::fs::write(&soul, "MY HAND-EDITED SOUL — do not clobber").unwrap();
        crate::config::profile::ensure_brain_seeded(); // second boot
        assert_eq!(
            std::fs::read_to_string(&soul).unwrap(),
            "MY HAND-EDITED SOUL — do not clobber",
            "re-seeding must never overwrite user content"
        );
    });
}

#[test]
fn partial_brain_completed_without_touching_custom_soul() {
    // The exact user report shape: the wizard's BrainSetup wrote an
    // AI-personalized SOUL.md, then onboarding aborted before finalize —
    // so ONLY SOUL.md exists. Entrypoint seeding must complete the rest
    // of the brain while leaving the personality file untouched.
    in_throwaway_home(|home| {
        std::fs::write(home.join("SOUL.md"), "AI-GENERATED PERSONALITY").unwrap();
        crate::config::profile::ensure_brain_seeded();
        assert_eq!(
            std::fs::read_to_string(home.join("SOUL.md")).unwrap(),
            "AI-GENERATED PERSONALITY",
            "AI-generated personality must survive entrypoint seeding"
        );
        for f in ALL_NINE {
            assert!(
                home.join(f).exists(),
                "partial brain must be completed: {f}"
            );
        }
    });
}

/// The coupling behind #1536, pinned so the helper above has a stated reason.
///
/// `ensure_brain_seeded()` names no path: it asks `opencrabs_home()`, which
/// consults the task-local override before falling through to `$HOME`. That
/// indirection is what let two reads in one test disagree, and it is also what
/// makes the override work — so if the override ever stops redirecting the
/// seed, this fails and the helper can be revisited rather than trusted.
#[test]
fn seeding_lands_under_the_overridden_home_and_not_the_real_one() {
    let home = tempfile::TempDir::new().expect("tempdir");
    let live = crate::config::profile::base_opencrabs_dir();
    // A developer's real home legitimately holds a SOUL.md, so its presence
    // proves nothing either way. What must hold is that this test does not
    // CHANGE that, so the state is sampled before and compared after.
    let live_soul_before = live.join("SOUL.md").exists();

    crate::config::profile::with_home_override(home.path().to_path_buf(), || {
        crate::config::profile::ensure_brain_seeded();
    });

    assert!(
        home.path().join("SOUL.md").is_file(),
        "the override must redirect the seed into the throwaway home"
    );
    assert_eq!(
        live_soul_before,
        live.join("SOUL.md").exists(),
        "seeding must not create anything in the real profile home: {}",
        live.display()
    );
}
