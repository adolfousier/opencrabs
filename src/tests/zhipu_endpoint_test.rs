//! z.ai endpoint resolution (#1350): a configured base_url wins over the
//! endpoint-type default, and the models URL follows the chat URL.

use crate::brain::provider::zhipu_endpoint::{chat_url, models_url};

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
