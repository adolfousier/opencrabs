//! Owner alerts that do not depend on the channel that raised them (#1999).
//!
//! A WhatsApp ban or lock stops the WhatsApp channel, so the alert cannot go
//! out over WhatsApp. The channel manager builds one [`OwnerAlert`] from the
//! Telegram state and hands it to the channel that needs it.

use std::sync::Arc;

/// Sends one line to the bot owner. Cheap to clone; each call is fire-and-forget,
/// and a failed send is logged rather than dropped.
pub type OwnerAlert = Arc<dyn Fn(String) + Send + Sync>;

/// An alert sink that does nothing, for builds without Telegram.
#[cfg(not(feature = "telegram"))]
pub(crate) fn no_alert() -> OwnerAlert {
    Arc::new(|_| {})
}

/// Send the alert as a Telegram DM to the owner's chat, the same route the
/// config alerts use. Logs when Telegram is not connected or no owner chat is
/// known yet, so the alert is never silently lost.
#[cfg(feature = "telegram")]
pub(crate) fn telegram_owner_alert(
    telegram: Arc<crate::channels::telegram::TelegramState>,
) -> OwnerAlert {
    Arc::new(move |text: String| {
        let telegram = telegram.clone();
        tokio::spawn(async move {
            use teloxide::prelude::Requester;
            let (Some(bot), Some(owner)) = (telegram.bot().await, telegram.owner_chat_id().await)
            else {
                tracing::warn!(
                    "owner alert not sent (Telegram not connected or no owner chat yet): {text}"
                );
                return;
            };
            if let Err(e) = bot
                .send_message(teloxide::types::ChatId(owner), text.clone())
                .await
            {
                tracing::error!("owner alert failed over Telegram: {e}; alert was: {text}");
            }
        });
    })
}
