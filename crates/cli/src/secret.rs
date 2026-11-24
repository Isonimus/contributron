//! A string that redacts itself in `Debug` and `Display`.
//!
//! Standing invariant 10 (ADR-0013) says a secret is held in memory, never logged. `String`
//! implements both traits and prints its contents; putting a token in one is one `tracing`
//! call away from leaking it. `SecretString` makes the leak a compile error at the obvious
//! sites: a `format!("{token}")` or a `{:?}` in a log event prints `<redacted>` instead of
//! the value. Reaching the inner string takes [`SecretString::expose`], which is greppable.
//!
//! The wrapper does not defend against a caller who exposes the string and logs the result
//! themselves; that is what `scripts/lint-secrets.mjs` watches for. It defends against the
//! common accident, and names the uncommon one so a reviewer can see it.

use std::fmt;

/// A secret, redacted when formatted.
#[derive(Clone)]
pub struct SecretString(String);

impl SecretString {
    /// Wraps a secret.
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// Returns the inner string. Named so a reviewer can grep for every site that unwraps.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString(<redacted>)")
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::SecretString;

    #[test]
    fn debug_does_not_print_the_value() {
        let s = SecretString::new("ghp_supersecret".into());
        let printed = format!("{s:?}");
        assert!(!printed.contains("ghp_supersecret"), "was: {printed}");
        assert!(printed.contains("redacted"), "was: {printed}");
    }

    #[test]
    fn display_does_not_print_the_value() {
        let s = SecretString::new("ghp_supersecret".into());
        let printed = format!("{s}");
        assert!(!printed.contains("ghp_supersecret"), "was: {printed}");
    }

    #[test]
    fn expose_returns_the_value() {
        let s = SecretString::new("ghp_supersecret".into());
        assert_eq!(s.expose(), "ghp_supersecret");
    }
}
