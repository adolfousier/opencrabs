//! Channel Integrations
//!
//! Messaging channel integrations (Telegram, WhatsApp, Discord, Slack) and the
//! shared factory for creating channel-specific agent services.

mod factory;
pub mod voice;

#[cfg(feature = "discord")]
pub mod discord;
#[cfg(feature = "slack")]
pub mod slack;
#[cfg(feature = "telegram")]
pub mod telegram;
#[cfg(feature = "trello")]
pub mod trello;
#[cfg(feature = "whatsapp")]
pub mod whatsapp;

pub use factory::ChannelFactory;
