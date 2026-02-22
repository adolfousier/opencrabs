//! Secure secret management
//!
//! This module provides secure handling of sensitive data like API keys,
//! ensuring they are properly zeroized from memory when dropped.

use serde::{Deserialize, Serialize};
use std::fmt;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// A secure string that zeroizes its contents on drop
///
/// This type should be used for any sensitive data like API keys,
/// passwords, or tokens to ensure they are properly cleared from memory.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecretString {
    inner: String,
}

impl SecretString {
    /// Create a new SecretString from a String
    pub fn new(value: String) -> Self {
        Self { inner: value }
    }

    /// Create a new SecretString from a &str
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Self {
        Self {
            inner: value.to_string(),
        }
    }

    /// Get a reference to the inner string
    ///
    /// # Security Warning
    /// Use with caution! This exposes the sensitive data.
    /// Avoid logging or displaying the returned value.
    pub fn expose_secret(&self) -> &str {
        &self.inner
    }

    /// Check if the secret is empty
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Get the length of the secret
    pub fn len(&self) -> usize {
        self.inner.len()
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

// Custom Serialize implementation to prevent accidental serialization
impl Serialize for SecretString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Never serialize the actual secret
        serializer.serialize_str("[REDACTED]")
    }
}

// Custom Deserialize implementation
impl<'de> Deserialize<'de> for SecretString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(SecretString::new(s))
    }
}

impl From<String> for SecretString {
    fn from(s: String) -> Self {
        SecretString::new(s)
    }
}

impl From<&str> for SecretString {
    fn from(s: &str) -> Self {
        SecretString::from_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secret_string_creation() {
        let secret = SecretString::from_str("my-secret-key");
        assert_eq!(secret.expose_secret(), "my-secret-key");
        assert_eq!(secret.len(), 13);
        assert!(!secret.is_empty());
    }

    #[test]
    fn test_secret_string_debug() {
        let secret = SecretString::from_str("my-secret-key");
        let debug_output = format!("{:?}", secret);
        assert_eq!(debug_output, "[REDACTED]");
        assert!(!debug_output.contains("my-secret-key"));
    }

    #[test]
    fn test_secret_string_display() {
        let secret = SecretString::from_str("my-secret-key");
        let display_output = format!("{}", secret);
        assert_eq!(display_output, "[REDACTED]");
        assert!(!display_output.contains("my-secret-key"));
    }

    #[test]
    fn test_secret_string_from_env_missing() {
        // Test that a non-existent env var returns None (no env loading)
        let result = std::env::var("OPENCRABS_TEST_NONEXISTENT_KEY_12345");
        assert!(result.is_err());
    }

    #[test]
    fn test_secret_string_serialize() {
        let secret = SecretString::from_str("my-secret-key");
        let serialized = serde_json::to_string(&secret).unwrap();
        assert_eq!(serialized, "\"[REDACTED]\"");
        assert!(!serialized.contains("my-secret-key"));
    }
}
