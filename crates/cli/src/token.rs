//! Finding a GitHub token without inventing a place to keep one.
//!
//! This slice is read-only, so a read token is all it can need. Nothing is stored: the token
//! comes from the environment, or from `git credential`, which is the mechanism git itself
//! uses and which already holds a github.com credential on any machine that has pushed to
//! one. A configuration file holding a secret would be a store to secure, migrate and
//! eventually leak, and the ledger carries the item that replaces all of this with an
//! authenticated identity before anything writes.

use anyhow::{Result, anyhow};
use std::io::Write as _;
use std::process::{Command, Stdio};

/// The environment variable checked first.
pub const TOKEN_VARIABLE: &str = "CONTRIBUTRON_GITHUB_TOKEN";

const CREDENTIAL_REQUEST: &str = "protocol=https\nhost=github.com\n\n";

/// Resolves a GitHub token, from the environment or from git's credential helper.
///
/// # Errors
///
/// Returns an error naming both places it looked. A missing credential is reported rather
/// than worked around: an unauthenticated request would fail later with a message about rate
/// limits, which sends the reader looking in the wrong place.
pub fn resolve() -> Result<String> {
    if let Ok(token) = std::env::var(TOKEN_VARIABLE)
        && !token.trim().is_empty()
    {
        return Ok(token);
    }

    from_git_credential().ok_or_else(|| {
        anyhow!(
            "no GitHub token found. Set {TOKEN_VARIABLE}, or store a github.com credential \
             with `git credential approve`, which is where this looked second."
        )
    })
}

/// Asks git's configured credential helper for a github.com password.
///
/// Returns `None` for every failure, because every failure means the same thing to the
/// caller: there is no credential here. The distinction between "no helper configured" and
/// "helper declined" would change nothing a caller could do about it.
fn from_git_credential() -> Option<String> {
    let mut child = Command::new("git")
        .args(["credential", "fill"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    child
        .stdin
        .as_mut()?
        .write_all(CREDENTIAL_REQUEST.as_bytes())
        .ok()?;

    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }

    parse_password(&String::from_utf8_lossy(&output.stdout))
}

/// Reads the `password=` line out of a credential helper's reply.
fn parse_password(reply: &str) -> Option<String> {
    reply
        .lines()
        .find_map(|line| line.strip_prefix("password="))
        .map(str::to_owned)
        .filter(|password| !password.is_empty())
}

#[cfg(test)]
mod tests {
    use super::parse_password;

    #[test]
    fn a_credential_reply_yields_its_password() {
        // The shape `git credential fill` writes: one key=value per line, order not promised.
        let reply = "protocol=https\nhost=github.com\nusername=Isonimus\npassword=ghp_example\n";
        assert_eq!(parse_password(reply).as_deref(), Some("ghp_example"));
    }

    #[test]
    fn a_reply_without_a_password_yields_nothing() {
        let reply = "protocol=https\nhost=github.com\nusername=Isonimus\n";
        assert_eq!(parse_password(reply), None);
    }

    #[test]
    fn an_empty_password_is_not_a_password() {
        // A helper that declines can still answer with an empty value, and an empty token
        // would be sent as a bearer credential and fail as a rate limit error.
        assert_eq!(parse_password("password=\n"), None);
    }

    #[test]
    fn a_username_is_not_mistaken_for_a_password() {
        let reply = "username=password-manager\npassword=ghp_example\n";
        assert_eq!(parse_password(reply).as_deref(), Some("ghp_example"));
    }
}
