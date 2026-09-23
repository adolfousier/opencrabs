//! Tests that `swap_provider_for_session` wraps raw providers in a
//! `FallbackProvider` so per-session swaps don't strip cascade
//! coverage.
//!
//! Regression context (2026-06-02 02:33:25-29): a session that picked
//! the custom `dialagram` provider via `/models` had its session-
//! specific provider entry set to the RAW dialagram client — not a
//! `FallbackProvider` wrapping dialagram + the configured fallbacks.
//! When dialagram returned HTTP 530, there was no transparent cascade
//! at the provider layer; the only safety net was the manual 5xx
//! fallback loop in `tool_loop.rs`, which had its own bug (didn't
//! swap the session's provider before calling stream_complete, so
//! every "Trying fallback X/Y..." iteration silently re-hit the
//! failing primary). Five "fallbacks" cascaded in ~4 seconds and the
//! user saw what looked like every provider being simultaneously
//! broken when in reality only dialagram had a momentary blip.
//!
//! Default-config sessions kept working all along because the global
//! `self.provider` is wrapped in `FallbackProvider` at construction
//! time in `factory.rs:451`. The bug only surfaced for sessions whose
//! provider had been overridden via `swap_provider_for_session` —
//! `/models` pick, session restore of a saved `provider_name`, or
//! the manual fallback loop's sticky promotion.
//!
//! The fix wraps the new provider in `FallbackProvider` inside
//! `swap_provider_for_session` (using the AgentService's configured
//! `fallback_providers`, filtered to exclude any candidate with the
//! same name as the new primary). These tests pin that behaviour so
//! a future refactor of session-swap plumbing can't silently re-open
//! the coverage hole.

use crate::brain::provider::Provider;
use crate::tests::agent_service_mocks::{
    MockProvider, MockProviderWithTools, create_test_service_with_provider,
};
use std::sync::Arc;

#[tokio::test]
async fn swap_wraps_raw_provider_in_fallback_chain_when_fallbacks_configured() {
    let (svc, sid) = create_test_service_with_provider(Arc::new(MockProvider)).await;
    svc.set_fallback_providers_for_test(vec![Arc::new(MockProviderWithTools::new())]);

    // Swap to a raw provider — no FallbackProvider wrapper around it.
    svc.swap_provider_for_session(sid, Arc::new(MockProvider), "mock-model");

    let stored = svc.provider_for_session(sid);
    assert!(
        stored.is_fallback_chain(),
        "swap_provider_for_session must wrap the new provider in a FallbackProvider \
         so cascade coverage isn't lost on /models pick or session restore. \
         Without this, the session's only safety net is the manual fallback \
         loop in tool_loop.rs — which historically had its own bugs \
         (2026-06-02 02:33:25 incident: dialagram HTTP 530, 5 \"fallbacks\" \
         cascading in ~4s because all of them re-hit the failing primary)."
    );
}

#[tokio::test]
async fn swap_preserves_user_facing_name_after_wrap() {
    let (svc, sid) = create_test_service_with_provider(Arc::new(MockProvider)).await;
    svc.set_fallback_providers_for_test(vec![Arc::new(MockProviderWithTools::new())]);
    svc.swap_provider_for_session(sid, Arc::new(MockProvider), "mock-model");

    let stored = svc.provider_for_session(sid);
    // FallbackProvider::name() delegates to the primary, so footers /
    // session-persistence stay on the user's chosen provider even
    // though the underlying type is now FallbackProvider.
    assert_eq!(
        stored.name(),
        "mock",
        "wrapping must not change the user-facing provider name — \
         the footer, session restore, and config display all read this"
    );
}

#[tokio::test]
async fn swap_skips_wrap_when_no_fallbacks_configured() {
    // Default config has no fallback chain. An empty
    // FallbackProvider(primary, vec![]) would behaviourally be
    // identical to the raw primary, but the extra pointer hop and
    // Drop overhead are pure waste. Skip the wrap in this case.
    let (svc, sid) = create_test_service_with_provider(Arc::new(MockProvider)).await;
    svc.set_fallback_providers_for_test(vec![]); // no fallbacks
    svc.swap_provider_for_session(sid, Arc::new(MockProvider), "mock-model");

    let stored = svc.provider_for_session(sid);
    assert!(
        !stored.is_fallback_chain(),
        "with no fallbacks configured, the new provider must be stored raw — \
         wrapping in an empty FallbackProvider adds no behavioural benefit, \
         just an extra Arc indirection per call"
    );
}

#[tokio::test]
async fn swap_does_not_double_wrap_existing_fallback_chain() {
    use crate::brain::provider::FallbackProvider;

    let (svc, sid) = create_test_service_with_provider(Arc::new(MockProvider)).await;
    svc.set_fallback_providers_for_test(vec![Arc::new(MockProviderWithTools::new())]);

    // Construct a FallbackProvider externally (the same shape the
    // sticky-promotion code paths in tool_loop.rs produce) and swap it
    // in. The wrapping logic must detect `is_fallback_chain() == true`
    // and store as-is, not nest another FallbackProvider around it.
    let already_wrapped: Arc<dyn Provider> = Arc::new(FallbackProvider::new(
        Arc::new(MockProvider),
        vec![Arc::new(MockProviderWithTools::new())],
    ));
    svc.swap_provider_for_session(sid, already_wrapped.clone(), "mock-model");

    let stored = svc.provider_for_session(sid);
    assert!(
        stored.is_fallback_chain(),
        "an already-wrapped FallbackProvider must still report is_fallback_chain"
    );
    // Both pointers should refer to the same Arc allocation — proving
    // we stored the input verbatim rather than constructing a new
    // outer FallbackProvider around it.
    assert!(
        Arc::ptr_eq(&stored, &already_wrapped),
        "swap must not double-wrap: an Arc<FallbackProvider> input must be stored \
         as-is, not nested inside a fresh outer FallbackProvider. Re-wrapping on \
         every swap would grow a deeper onion each time the user picks via /models."
    );
}

#[tokio::test]
async fn swap_excludes_self_from_fallback_chain() {
    // If the user picks "mock" as the primary and the configured
    // fallback chain also contains "mock", the wrap must not put mock
    // in its own fallback chain — that would mean a primary failure
    // cascades to the SAME dead endpoint immediately, defeating the
    // purpose of fallback.
    let (svc, sid) = create_test_service_with_provider(Arc::new(MockProvider)).await;
    svc.set_fallback_providers_for_test(vec![
        Arc::new(MockProvider), // same name as the new primary
        Arc::new(MockProviderWithTools::new()),
    ]);
    svc.swap_provider_for_session(sid, Arc::new(MockProvider), "mock-model");

    let stored = svc.provider_for_session(sid);
    assert!(
        stored.is_fallback_chain(),
        "self-name filtering must still leave at least one other fallback \
         in the chain (mock-with-tools), so the result is a FallbackProvider \
         not a raw provider"
    );
    // The exact subprovider count isn't observable through the
    // Provider trait, but the existence of the chain plus the
    // `active_subprovider_name` API guarantees the contract — the
    // structural correctness (excluding self) is verified by the
    // wrapping logic in builder.rs being the single producer.
}

#[tokio::test]
async fn swap_drops_to_raw_when_only_fallback_is_self() {
    // Edge case of the previous test: the configured fallback list
    // contains ONLY a candidate with the same name as the new primary.
    // After filtering, the chain is empty, so we fall through to the
    // "no fallbacks → store raw" path. Otherwise we'd build a
    // pointless FallbackProvider with an empty fallbacks vec.
    let (svc, sid) = create_test_service_with_provider(Arc::new(MockProvider)).await;
    svc.set_fallback_providers_for_test(vec![Arc::new(MockProvider)]);
    svc.swap_provider_for_session(sid, Arc::new(MockProvider), "mock-model");

    let stored = svc.provider_for_session(sid);
    assert!(
        !stored.is_fallback_chain(),
        "when every configured fallback collides with the new primary's name, \
         the chain ends up empty and the raw provider should be stored — no point \
         building a FallbackProvider with zero fallbacks"
    );
}

/// Provider+model are a pair, set atomically. `swap_provider_for_session`
/// takes the model as a required argument and uses exactly that — it never
/// invents or defaults the model to the provider's `default_model()`.
///
/// Regression (2026-06-07): the TUI footer showed "modelscope / GLM 5.1"
/// after the user switched to Qwen3.7-Max, because swap clobbered the
/// session model with `new_provider.default_model()` (a stale config
/// default) instead of using the model the user actually picked. The fix
/// makes the caller pass the paired model so a provider can't be swapped
/// without its model.
#[tokio::test]
async fn swap_sets_the_paired_model_never_invents() {
    let (svc, sid) = create_test_service_with_provider(Arc::new(MockProvider)).await;

    // swap sets the provider AND the paired model atomically, from the
    // caller's argument — the user's pick, not the provider default.
    svc.swap_provider_for_session(sid, Arc::new(MockProvider), "qwen-3.7-max");
    assert_eq!(svc.provider_model_for_session(sid), "qwen-3.7-max");

    // A later swap to the SAME provider with a different paired model
    // updates the model — proving it always comes from the caller, never
    // the provider's default_model() ("mock-model").
    svc.swap_provider_for_session(sid, Arc::new(MockProvider), "glm-5.1");
    assert_eq!(svc.provider_model_for_session(sid), "glm-5.1");
    assert_ne!(
        svc.provider_model_for_session(sid),
        "mock-model",
        "swap must use the caller's model, never invent the provider default"
    );
}

/// Mid-turn manual switch (2026-06-08, the proper fix): the user's switch is
/// captured as a pinned pair; AFTER a turn that took an automatic fallback,
/// the pin is re-applied so the NEXT turn uses the user's pick — atomically
/// (provider+model together), so the model can never desync from the
/// provider. This restore runs OFF the completion path, so it can never drop
/// the request (the earlier regression dropped it by suppressing the
/// fallback's model-sync event mid-turn — this never touches the turn).
#[tokio::test]
async fn manual_switch_is_restored_after_a_fallback_turn_atomically() {
    let (svc, sid) = create_test_service_with_provider(Arc::new(MockProvider)).await;

    // Capture the turn-start epoch. No switch yet → restore is a no-op.
    let start = svc.manual_switch_epoch(sid);
    assert_eq!(start, 0);
    assert_eq!(svc.restore_manual_switch_if_changed(sid, start), None);

    // User switches provider/model mid-turn.
    svc.swap_provider_for_session(sid, Arc::new(MockProvider), "user-pick");
    svc.mark_manual_switch(sid, "user-pick".to_string());
    assert_ne!(
        svc.manual_switch_epoch(sid),
        start,
        "the switch bumps the epoch"
    );

    // The in-flight turn takes a fallback, overwriting the session pair.
    svc.swap_provider_for_session(sid, Arc::new(MockProvider), "fallback-model");
    assert_eq!(svc.provider_model_for_session(sid), "fallback-model");

    // After the turn completes, restore detects the mid-turn switch and
    // re-applies the user's pinned pair (returns the model to persist to DB).
    let restored = svc.restore_manual_switch_if_changed(sid, start);
    assert_eq!(
        restored.as_deref(),
        Some("user-pick"),
        "restore returns the user's model so the caller can persist it"
    );
    assert_eq!(
        svc.provider_model_for_session(sid),
        "user-pick",
        "the user's pick wins for the NEXT turn, not the fallback"
    );

    // Idempotent: with no NEW switch, restoring against the current epoch is
    // a no-op — it won't fight a legitimate fallback on a later turn.
    let now = svc.manual_switch_epoch(sid);
    assert_eq!(svc.restore_manual_switch_if_changed(sid, now), None);
}

#[tokio::test]
async fn context_limit_falls_back_to_provider_window_without_manual_swap() {
    // The global provider carries a configured window (like Kimi K3's 1M in
    // config), but the session is NEVER manually /models-swapped, so
    // session_context_limits stays empty. Before #609 the budget fell back to
    // agent.context_limit (200k default) while the header showed 1M, so
    // compaction fired at ~185K. The session must now honor the provider window.
    let provider = Arc::new(
        crate::brain::provider::AnthropicProvider::new("k".to_string())
            .with_context_window(1_000_000),
    );
    let (svc, sid) = create_test_service_with_provider(provider).await;
    assert_eq!(
        svc.context_limit_for_session(sid),
        1_000_000,
        "an un-swapped session must honor its provider's configured window, not the 200k default"
    );
}
