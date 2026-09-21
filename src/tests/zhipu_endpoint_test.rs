//! z.ai endpoint resolution (#1350): a configured base_url wins over the
//! endpoint-type default, and the models URL follows the chat URL.

use crate::brain::provider::zhipu_endpoint::{chat_url, default_idle_timeout_secs, models_url};

#[test]
fn the_default_follows_endpoint_type_on_api_z_ai() {
    assert_eq!(
        chat_url(None, None),
        "https://api.z.ai/api/paas/v4/chat/completions"
    );
    assert_eq!(
        chat_url(None, Some("api")),
        "https://api.z.ai/api/paas/v4/chat/completions"
    );
    assert_eq!(
        chat_url(None, Some("coding")),
        "https://api.z.ai/api/coding/paas/v4/chat/completions"
    );
    assert_eq!(
        models_url(None, Some("coding")),
        "https://api.z.ai/api/coding/paas/v4/models"
    );
}

#[test]
fn a_configured_base_url_wins_over_endpoint_type() {
    let bigmodel = Some("https://open.bigmodel.cn/api/paas/v4");
    assert_eq!(
        chat_url(bigmodel, Some("coding")),
        "https://open.bigmodel.cn/api/paas/v4/chat/completions"
    );
    assert_eq!(
        models_url(bigmodel, Some("coding")),
        "https://open.bigmodel.cn/api/paas/v4/models"
    );
}

#[test]
fn a_full_chat_url_or_a_trailing_slash_is_normalised() {
    for given in [
        "https://open.bigmodel.cn/api/paas/v4/",
        "https://open.bigmodel.cn/api/paas/v4/chat/completions",
        "https://open.bigmodel.cn/api/paas/v4/chat/completions/",
        "  https://open.bigmodel.cn/api/paas/v4  ",
    ] {
        assert_eq!(
            chat_url(Some(given), None),
            "https://open.bigmodel.cn/api/paas/v4/chat/completions",
            "{given}"
        );
        assert_eq!(
            models_url(Some(given), None),
            "https://open.bigmodel.cn/api/paas/v4/models",
            "{given}"
        );
    }
}

#[test]
fn an_empty_base_url_means_unset() {
    assert_eq!(
        chat_url(Some(""), Some("coding")),
        chat_url(None, Some("coding"))
    );
    assert_eq!(chat_url(Some("   "), None), chat_url(None, None));
}

/// #1666: the generic remote idle default is 20s, and `api.z.ai` is documented
/// in this very module as holding an idle stream to roughly 30s. A default that
/// cuts before the host does makes our client blame a connection it closed.
#[test]
fn the_api_z_ai_default_sits_above_the_host_idle_cut() {
    const HOST_IDLE_CUT_SECS: u64 = 30;

    for endpoint_type in [None, Some("api"), Some("coding")] {
        let secs = default_idle_timeout_secs(None, endpoint_type)
            .unwrap_or_else(|| panic!("no host-aware default for endpoint_type {endpoint_type:?}"));
        assert!(
            secs > HOST_IDLE_CUT_SECS,
            "{secs}s would cut before api.z.ai's own {HOST_IDLE_CUT_SECS}s close ({endpoint_type:?})"
        );
    }
}

#[test]
fn a_host_without_a_documented_cut_keeps_the_generic_default() {
    assert_eq!(
        default_idle_timeout_secs(Some("https://open.bigmodel.cn/api/paas/v4"), Some("coding")),
        None
    );
    assert_eq!(
        default_idle_timeout_secs(Some("https://proxy.internal/v1"), None),
        None
    );
}

#[test]
fn an_unset_base_url_still_resolves_to_the_api_z_ai_default() {
    assert_eq!(
        default_idle_timeout_secs(Some("  "), None),
        default_idle_timeout_secs(None, None)
    );
    assert_eq!(
        default_idle_timeout_secs(Some("https://api.z.ai/api/paas/v4/chat/completions"), None),
        default_idle_timeout_secs(None, None)
    );
}
