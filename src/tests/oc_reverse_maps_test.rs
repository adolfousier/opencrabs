//! #148 Step 3: after a simulated registration at each channel's single
//! write site, the resolver's reverse lookup answers `target → owning
//! session` for every authority. Also pins last-writer-wins semantics.

use uuid::Uuid;

use crate::channels::discord::DiscordState;
use crate::channels::slack::SlackState;
use crate::channels::whatsapp::WhatsAppState;

#[tokio::test]
async fn discord_reverse_map_resolves_channel_to_session() {
    let st = DiscordState::new();
    let s = Uuid::new_v4();
    st.register_session_channel(s, 987654321).await;
    assert_eq!(st.session_owner_by_channel(987654321).await, Some(s));
    assert_eq!(st.session_owner_by_channel(111111111).await, None);
}

#[tokio::test]
async fn discord_re_registration_last_writer_wins() {
    let st = DiscordState::new();
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    st.register_session_channel(a, 42).await;
    st.register_session_channel(b, 42).await;
    assert_eq!(st.session_owner_by_channel(42).await, Some(b));
}

#[tokio::test]
async fn slack_reverse_map_resolves_channel_to_session() {
    let st = SlackState::new();
    let s = Uuid::new_v4();
    st.register_session_channel(s, "C0123ABCDEF".to_string()).await;
    assert_eq!(
        st.session_owner_by_channel("C0123ABCDEF").await,
        Some(s)
    );
    assert_eq!(st.session_owner_by_channel("C999").await, None);
}

#[tokio::test]
async fn whatsapp_reverse_map_resolves_jid_to_session() {
    let st = WhatsAppState::default();
    let s = Uuid::new_v4();
    st.register_session_jid(s, "79991234567@s.whatsapp.net".to_string())
        .await;
    assert_eq!(
        st.session_owner_by_jid("79991234567@s.whatsapp.net").await,
        Some(s)
    );
    assert_eq!(st.session_owner_by_jid("100@s.whatsapp.net").await, None);
}

#[tokio::test]
#[cfg(feature = "telegram")]
async fn telegram_registration_is_bidirectional_both_ways() {
    // Telegram's register_session_chat was already bidirectional (#1220);
    // pin both directions here so a future refactor cannot silently drop
    // the reverse leg the resolver depends on.
    use crate::channels::telegram::TelegramState;
    let st = TelegramState::new();
    let s = Uuid::new_v4();
    st.register_session_chat(s, -100123, Some(7)).await;
    // reverse: (chat, topic) -> session
    assert_eq!(
        st.chat_session(-100123, Some(7)).await,
        Some(s)
    );
    // forward: session -> (chat, topic) via the ambient-binding probe
    assert_eq!(
        st.session_binding(s).await,
        Some((-100123, Some(7)))
    );
}
