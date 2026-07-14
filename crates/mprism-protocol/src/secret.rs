//! Secret string wrapper that never prints credentials.

use std::fmt;

/// Opaque API credential. `Debug` and `Display` are always redacted.
#[derive(Clone, Default)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Expose the raw secret only for constructing Authorization headers.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl From<String> for SecretString {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for SecretString {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString([REDACTED])")
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_debug_and_display() {
        let secret = SecretString::new("sk-super-secret-key");
        let debug = format!("{secret:?}");
        let display = format!("{secret}");
        assert!(!debug.contains("sk-super-secret-key"));
        assert!(!display.contains("sk-super-secret-key"));
        assert!(debug.contains("REDACTED"));
        assert_eq!(secret.expose_secret(), "sk-super-secret-key");
    }
}
