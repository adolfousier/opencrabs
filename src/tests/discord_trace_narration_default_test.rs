//! #1871: `trace_narration` must default to TRUE on every path.
//!
//! Context: the flag folds intermediate narration into the turn's
//! tool-group bubble as dim subtext instead of posting each intermediate
//! as a separate message. It shipped default-off, so fresh installs got
//! the per-intermediate spam out of the box while the comparable folding
//! on Telegram/Slack is on by default. These guards pin the flip on both
//! default paths (struct default and a `[channels.discord]` section that
//! omits the key), plus the explicit-false opt-out.

use crate::config::DiscordConfig;

#[test]
fn default_discord_config_folds_narration() {
    assert!(
        DiscordConfig::default().trace_narration,
        "#1871: trace_narration must default on; opt out with trace_narration = false"
    );
}

#[test]
fn omitted_key_deserializes_to_on() {
    let empty: DiscordConfig =
        toml::from_str("").expect("a [channels.discord] section can be empty");
    assert!(
        empty.trace_narration,
        "serde default must match the struct default (on)"
    );

    let partial: DiscordConfig = toml::from_str("auto_thread_min_chars = 1800\n")
        .expect("a partial [channels.discord] section must deserialize");
    assert!(
        partial.trace_narration,
        "presence of sibling keys must not resurrect the old false default"
    );
}

#[test]
fn explicit_false_still_opts_out() {
    let cfg: DiscordConfig =
        toml::from_str("trace_narration = false\n").expect("opt-out line must parse");
    assert!(
        !cfg.trace_narration,
        "the opt-out must survive the default flip; installs that want per-intermediate \
         messages keep the way back"
    );
}
