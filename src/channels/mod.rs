//! Channel Integrations
//!
//! Messaging channel integrations (Telegram, WhatsApp, Discord, Slack) and the
//! shared factory for creating channel-specific agent services.

pub mod commands;
mod factory;
pub mod manager;
#[cfg(test)]
mod tests;
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

mod greeting;

pub use factory::ChannelFactory;
pub use greeting::generate_connection_greeting;
pub use manager::ChannelManager;
