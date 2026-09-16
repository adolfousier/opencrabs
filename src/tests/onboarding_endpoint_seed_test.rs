//! Onboarding endpoint-type seeding (#1596).
//!
//! `OnboardingWizard::from_config` must seed the z.ai and Xiaomi endpoint
//! pickers from the saved `endpoint_type`, the way the Moonshot branch
//! already does. Without the seed, `/models` opens showing General API on a
//! coding/token-plan account and a no-change Confirm writes the default back
//! over the saved choice. The Xiaomi branch itself was missing entirely from
//! the selection chain — a config with only xiaomi enabled never selected it.

use crate::config::profile::with_home_override;
use crate::tui::onboarding::{OnboardingStep, OnboardingWizard};

fn config_from_toml(toml_str: &str) -> crate::config::Config {
    toml::from_str(toml_str).expect("fixture config parses")
}

#[test]
fn from_config_seeds_zai_coding_endpoint_type() {
    let config = config_from_toml(
        r#"
[providers.zai]
enabled = true
endpoint_type = "coding"
default_model = "glm-4.7"
"#,
    );
    let wizard = OnboardingWizard::from_config(&config);
    assert_eq!(wizard.ps.selected_provider, 6, "z.ai is picker index 6");
    assert_eq!(
        wizard.ps.zhipu_endpoint_type, 1,
        "coding plan seeds the picker at 1"
    );
    assert_eq!(wizard.ps.custom_model, "glm-4.7");
}

#[test]
fn from_config_seeds_zai_api_endpoint_type() {
    let config = config_from_toml(
        r#"
[providers.zai]
enabled = true
endpoint_type = "api"
"#,
    );
    let wizard = OnboardingWizard::from_config(&config);
    assert_eq!(
        wizard.ps.zhipu_endpoint_type, 0,
        "api plan seeds the picker at 0"
    );
}

#[test]
fn from_config_seeds_xiaomi_token_plan_endpoint_type() {
    let config = config_from_toml(
        r#"
[providers.xiaomi]
enabled = true
endpoint_type = "token-plan"
default_model = "mimo"
"#,
    );
    let wizard = OnboardingWizard::from_config(&config);
    let xiaomi_idx = crate::tui::provider_selector::index_of_provider("xiaomi")
        .expect("xiaomi is a known provider");
    assert_eq!(
        wizard.ps.selected_provider, xiaomi_idx,
        "xiaomi branch selects xiaomi"
    );
    assert_eq!(
        wizard.ps.xiaomi_endpoint_type, 1,
        "token-plan seeds the picker at 1"
    );
    assert_eq!(wizard.ps.custom_model, "mimo");
}

#[test]
fn from_config_seeds_xiaomi_api_endpoint_type() {
    let config = config_from_toml(
        r#"
[providers.xiaomi]
enabled = true
endpoint_type = "api"
"#,
    );
    let wizard = OnboardingWizard::from_config(&config);
    assert_eq!(
        wizard.ps.xiaomi_endpoint_type, 0,
        "api plan seeds the picker at 0"
    );
}

#[test]
fn from_config_zai_defaults_to_api_when_endpoint_type_unset() {
    // Configs written before endpoint_type existed carry no value.
    let config = config_from_toml(
        r#"
[providers.zai]
enabled = true
"#,
    );
    let wizard = OnboardingWizard::from_config(&config);
    assert_eq!(wizard.ps.zhipu_endpoint_type, 0, "unset means General API");
}

// ── No-change Confirm writes the saved value back ───────────────────────────

fn in_temp_home(f: impl FnOnce()) {
    let dir = tempfile::tempdir().expect("tempdir");
    let opencrabs = dir.path().join(".opencrabs");
    std::fs::create_dir_all(&opencrabs).expect("create .opencrabs");
    with_home_override(opencrabs, f);
}

#[test]
fn confirm_without_changes_preserves_zai_coding_endpoint_type() {
    in_temp_home(|| {
        let home = crate::config::opencrabs_home();
        std::fs::create_dir_all(&home).expect("home dir");
        let cfg = home.join("config.toml");
        std::fs::write(
            &cfg,
            "[providers.zai]\nenabled = true\nendpoint_type = \"coding\"\ndefault_model = \"glm-4.7\"\n",
        )
        .expect("seed config");

        let config = crate::config::Config::load().expect("load");
        let wizard = OnboardingWizard::from_config(&config);
        assert_eq!(wizard.ps.zhipu_endpoint_type, 1);

        // Confirm with no edits: the provider write path must persist the
        // seeded picker state, i.e. the same "coding" it read.
        wizard
            .apply_step_config(OnboardingStep::ProviderAuth)
            .expect("provider-scoped write");

        let after = std::fs::read_to_string(&cfg).unwrap();
        assert!(
            after.contains("endpoint_type = \"coding\""),
            "coding must survive a no-change Confirm, got: {after}"
        );
        assert!(
            !after.contains("endpoint_type = \"api\""),
            "the api default must not overwrite the saved coding choice"
        );
    });
}

#[test]
fn confirm_without_changes_preserves_xiaomi_token_plan_endpoint_type() {
    in_temp_home(|| {
        let home = crate::config::opencrabs_home();
        std::fs::create_dir_all(&home).expect("home dir");
        let cfg = home.join("config.toml");
        std::fs::write(
            &cfg,
            "[providers.xiaomi]\nenabled = true\nendpoint_type = \"token-plan\"\n",
        )
        .expect("seed config");

        let config = crate::config::Config::load().expect("load");
        let wizard = OnboardingWizard::from_config(&config);
        assert_eq!(wizard.ps.xiaomi_endpoint_type, 1);

        wizard
            .apply_step_config(OnboardingStep::ProviderAuth)
            .expect("provider-scoped write");

        let after = std::fs::read_to_string(&cfg).unwrap();
        assert!(
            after.contains("endpoint_type = \"token-plan\""),
            "token-plan must survive a no-change Confirm, got: {after}"
        );
        assert!(
            !after.contains("endpoint_type = \"api\""),
            "the api default must not overwrite the saved token-plan choice"
        );
    });
}
