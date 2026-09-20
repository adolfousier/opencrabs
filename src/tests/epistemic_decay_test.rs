//! Decay policy resolution from `[epistemic]` config (piece C, #1657).
//!
//! Moved out of `src/brain/tools/epistemic.rs`: tests live under `src/tests/`,
//! never inline beside the logic they exercise (#1076).

use crate::brain::tools::epistemic::{DecaySettings, decay_settings_from, resolve_decay_settings};
use crate::brain::tools::plan_tool::EpistemicConfig;

fn cfg(hours: u64, decay_enabled: bool) -> EpistemicConfig {
    EpistemicConfig {
        decay_interval_hours: hours,
        decay_enabled,
        ..EpistemicConfig::default()
    }
}

#[test]
fn decay_settings_default_config_is_enabled_thirty_days() {
    let s = decay_settings_from(&EpistemicConfig::default());
    assert_eq!(
        s,
        DecaySettings {
            enabled: true,
            days: 30
        }
    );
}

#[test]
fn decay_settings_respects_disabled_switch() {
    let s = decay_settings_from(&cfg(720, false));
    assert_eq!(
        s,
        DecaySettings {
            enabled: false,
            days: 30
        }
    );
}

#[test]
fn decay_settings_converts_hours_to_whole_days() {
    assert_eq!(decay_settings_from(&cfg(48, true)).days, 2);
    assert_eq!(decay_settings_from(&cfg(100, true)).days, 4); // truncates
}

#[test]
fn decay_settings_floors_sub_day_interval_to_one_day() {
    // A sub-day interval must not become "decay everything immediately".
    assert_eq!(decay_settings_from(&cfg(12, true)).days, 1);
    assert_eq!(decay_settings_from(&cfg(0, true)).days, 1);
}

#[test]
fn resolve_decay_settings_reads_ralph_loop_config() {
    // The profile home's own ralph_loop.toml wins (#947 precedence), so the
    // real machine-wide safety/ralph_loop.toml is never consulted here.
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        tmp.path().join("ralph_loop.toml"),
        "[epistemic]\ndecay_enabled = false\ndecay_interval_hours = 168\n",
    )
    .expect("write config");
    crate::config::profile::with_home_override(tmp.path().to_path_buf(), || {
        let s = resolve_decay_settings();
        assert_eq!(
            s,
            DecaySettings {
                enabled: false,
                days: 7
            }
        );
    });
}
