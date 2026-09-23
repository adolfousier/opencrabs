//! `[providers.fallback]` reloads on config change like the primary does
//! (#1249).
//!
//! The ConfigWatcher hot-swapped the PRIMARY provider on every config write and
//! left the fallback chain frozen at process start, because the chain was built
//! once in `AgentService::new` and stored in a plain `Vec`. Editing
//! `fallback_chain` therefore did nothing until a restart: a provider deleted
//! from the config kept being handed live traffic for as long as the process
//! ran, which reads exactly like "config hot reload is broken".
//!
//! These tests pin the two halves of the fix: the chain is swappable at
//! runtime, and rebuilding from a config with no fallback section CLEARS it —
//! removal has to be representable, or deleting a provider stays impossible.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, watch};

use crate::brain::provider::Provider;
use crate::channels::ChannelFactory;
use crate::config::Config;
use crate::db::Database;
use crate::services::ServiceContext;
use crate::tests::agent_service_mocks::{
    MockProvider, MockProviderWithTools, create_test_service_with_provider,
};

#[tokio::test]
async fn reload_clears_a_chain_that_config_no_longer_declares() {
    let (svc, _sid) = create_test_service_with_provider(Arc::new(MockProvider)).await;
    svc.set_fallback_providers_for_test(vec![Arc::new(MockProviderWithTools::new())]);
    assert!(
        svc.has_fallback_provider(),
        "precondition: the runtime is holding a chain"
    );

    // `Config::default()` declares no `[providers.fallback]` — the state after
    // a user deletes the section (or empties `fallback_chain`) and saves.
    svc.reload_fallback_providers(&Config::default()).await;

    assert!(
        !svc.has_fallback_provider(),
        "#1249: a provider removed from config must stop receiving traffic \
         without a restart"
    );
    assert!(svc.fallback_chain_snapshot().is_empty());
}

/// The snapshot accessor is what every walk reads. It must reflect the live
/// slot, not a copy captured at construction.
#[tokio::test]
async fn snapshot_reflects_the_current_chain() {
    let (svc, _sid) = create_test_service_with_provider(Arc::new(MockProvider)).await;

    assert!(
        svc.fallback_chain_snapshot().is_empty(),
        "test config carries no fallbacks"
    );

    svc.set_fallback_providers_for_test(vec![Arc::new(MockProviderWithTools::new())]);
    assert_eq!(svc.fallback_chain_snapshot().len(), 1);

    svc.set_fallback_providers_for_test(vec![]);
    assert!(svc.fallback_chain_snapshot().is_empty());
}

/// Per-session swaps wrap the CURRENT chain. Before the fix this read a frozen
/// vec, so a session switched after a config edit still inherited providers the
/// user had removed.
#[tokio::test]
async fn session_swap_wraps_the_reloaded_chain() {
    let (svc, sid) = create_test_service_with_provider(Arc::new(MockProvider)).await;
    svc.set_fallback_providers_for_test(vec![Arc::new(MockProviderWithTools::new())]);

    svc.swap_provider_for_session(sid, Arc::new(MockProvider), "mock-model");
    assert!(
        svc.provider_for_session(sid).is_fallback_chain(),
        "precondition: a chain exists, so the swap wraps"
    );

    // User empties the chain, then switches model again.
    svc.reload_fallback_providers(&Config::default()).await;
    svc.swap_provider_for_session(sid, Arc::new(MockProvider), "mock-model");

    assert!(
        !svc.provider_for_session(sid).is_fallback_chain(),
        "#1249: with the chain removed there is nothing to wrap — the session \
         must not keep cascading into providers the config no longer lists"
    );
}

// ---------------------------------------------------------------------------
// #1700: the same two halves, for every agent ChannelFactory builds.
//
// The three tests above construct an AgentService directly. In production most
// of them come from ChannelFactory (chat channels, the A2A gateway, cron), and
// the factory carried neither a reloadable provider slot nor any way back to
// the instances it handed out. So `reload_fallback_providers` had exactly one
// production caller, the TUI's own service, and a provider deleted from
// `[providers.fallback]` kept serving those sessions until restart.
//
// Real construction through `create_agent_service_full`, not a source scan:
// these fail if only the TUI path reloads.
// ---------------------------------------------------------------------------

async fn test_factory() -> ChannelFactory {
    let db = Database::connect_in_memory().await.unwrap();
    let context = ServiceContext::new(db.pool().clone());
    let (_, config_rx) = watch::channel(Config::default());
    ChannelFactory::new(
        Arc::new(MockProvider) as Arc<dyn Provider>,
        context,
        "test brain".to_string(),
        PathBuf::from("/tmp"),
        PathBuf::from("/tmp/oc_test_brain_1700"),
        Arc::new(Mutex::new(None)),
        config_rx,
    )
}

/// The registry has to reach an agent that is already live.
#[tokio::test]
async fn factory_reload_clears_a_live_agent_chain() {
    let factory = test_factory().await;
    let service = factory.create_agent_service_full(None, None).await;

    // Seeded through the Arc. The seam takes `&self` (builder.rs); while it took
    // `&mut self` this was unreachable, because the factory registers a Weak and
    // `Arc::get_mut` requires weak_count == 0.
    service.set_fallback_providers_for_test(vec![Arc::new(MockProviderWithTools::new())]);
    assert!(
        service.has_fallback_provider(),
        "precondition: the built agent is holding a chain"
    );

    // The ConfigWatcher's call, carrying a config with no fallback section.
    factory.reload_providers(&Config::default(), None).await;

    assert!(
        !service.has_fallback_provider(),
        "#1700: a factory-built agent must drop a provider the config no longer \
         declares, without a restart"
    );
}

/// An already-built agent must serve on the rebuilt primary.
#[tokio::test]
async fn factory_reload_swaps_a_live_agent_primary() {
    let factory = test_factory().await;
    let service = factory.create_agent_service_full(None, None).await;
    assert_eq!(
        service.provider_name(),
        "mock",
        "precondition: the factory starts on its ctor provider"
    );

    factory
        .reload_providers(
            &Config::default(),
            Some(Arc::new(MockProviderWithTools::new()) as Arc<dyn Provider>),
        )
        .await;

    assert_eq!(
        service.provider_name(),
        "mock-with-tools",
        "#1700: the live agent was handed the rebuilt primary"
    );
}

/// The freeze #1700 does not name: the factory's own provider field was a bare
/// `Arc`, so a channel spawned AFTER a key rotation was still built on the
/// pre-rotation instance even though the rotation had already happened.
#[tokio::test]
async fn agents_built_after_a_reload_get_the_reloaded_primary() {
    let factory = test_factory().await;
    factory
        .reload_providers(
            &Config::default(),
            Some(Arc::new(MockProviderWithTools::new()) as Arc<dyn Provider>),
        )
        .await;

    let after = factory.create_agent_service_full(None, None).await;
    assert_eq!(
        after.provider_name(),
        "mock-with-tools",
        "#1700: the provider slot must not stay frozen at the ctor instance"
    );
}

/// Channels stop. A dead entry in the registry must not abort the reload for
/// the agents that are still live.
#[tokio::test]
async fn reload_prunes_dropped_agents_and_still_reloads_the_rest() {
    let factory = test_factory().await;
    let transient = factory.create_agent_service_full(None, None).await;
    let retained = factory.create_agent_service_full(None, None).await;
    drop(transient);

    factory
        .reload_providers(
            &Config::default(),
            Some(Arc::new(MockProviderWithTools::new()) as Arc<dyn Provider>),
        )
        .await;

    assert_eq!(
        retained.provider_name(),
        "mock-with-tools",
        "#1700: one dropped channel must not shield the live ones from a reload"
    );
}
