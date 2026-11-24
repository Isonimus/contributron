//! Resolving a GitHub token per ADR-0013.
//!
//! One resolver, shared by the read path (`watch`, `list`) and the write path (reactor). The
//! read path treats a missing token as an acceptable state for public repositories, so it
//! calls [`resolve_optional`]; the write path must have a credential and calls [`resolve`].
//!
//! Precedence, per ADR-0013: `--token-path <file>` first, then the `CONTRIBUTRON_TOKEN`
//! environment variable. `git credential fill` is a convenience fallback, kept because this
//! was the original shape and a user on a machine that has already pushed to github.com
//! already has a credential there; it is documented in the ADR-0013 amendment as an operator
//! convenience, not a credential source the write path depends on.

use crate::secret::SecretString;
use anyhow::{Context as _, Result, anyhow};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The environment variable checked after `--token-path`. Named by ADR-0013.
pub const TOKEN_VARIABLE: &str = "CONTRIBUTRON_TOKEN";

const CREDENTIAL_REQUEST: &str = "protocol=https\nhost=github.com\n\n";

/// Where the resolver was told to look.
#[derive(Debug, Clone, Default)]
pub struct Source {
    /// A path passed as `--token-path <file>`. Takes precedence over the env var.
    pub path: Option<PathBuf>,
}

impl Source {
    /// Builds a source from the CLI flag. `None` means "no `--token-path` was given".
    #[must_use]
    pub fn from_flag(path: Option<PathBuf>) -> Self {
        Self { path }
    }
}

/// Resolves a token, failing loud if none is available.
///
/// # Errors
///
/// Returns an error naming the places that were tried. The write path uses this; a missing
/// credential there is a failure the operator must fix, not a state to work around (ADR-0013
/// consequence 3 of the token-handling block).
pub fn resolve(source: &Source) -> Result<SecretString> {
    resolve_optional(source)?.ok_or_else(|| {
        anyhow!(
            "no GitHub token found. Pass --token-path <file>, set {TOKEN_VARIABLE}, or store \
             a github.com credential with `git credential approve` (ADR-0013)."
        )
    })
}

/// Resolves a token if one is available, returning `None` otherwise.
///
/// # Errors
///
/// Returns an error only when a source was *given* and could not be read: a `--token-path`
/// pointing at a missing or unreadable file, or an env var whose value turned out to be only
/// whitespace. "No source given" is not an error for this function; it returns `Ok(None)`,
/// because the read path is allowed to proceed anonymously against public repositories
/// (ADR-0013 consequence 3).
pub fn resolve_optional(source: &Source) -> Result<Option<SecretString>> {
    if let Some(path) = source.path.as_deref() {
        return read_token_file(path).map(Some);
    }

    if let Ok(token) = std::env::var(TOKEN_VARIABLE) {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            return Err(anyhow!(
                "{TOKEN_VARIABLE} is set but holds only whitespace. Unset it or pass \
                 --token-path to override."
            ));
        }
        return Ok(Some(SecretString::new(trimmed.to_owned())));
    }

    Ok(from_git_credential().map(SecretString::new))
}

/// Reads a token from a file. Trailing whitespace is trimmed; an empty file is refused.
fn read_token_file(path: &Path) -> Result<SecretString> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading the token file at {}", path.display()))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!(
            "the token file at {} is empty or whitespace-only",
            path.display()
        ));
    }
    Ok(SecretString::new(trimmed.to_owned()))
}

/// Asks git's configured credential helper for a github.com password.
///
/// Returns `None` for every failure, because every failure means the same thing to the
/// caller: there is no credential here. Kept as the operator-convenience fallback documented
/// in the ADR-0013 amendment; not relied on by the write path, which fails loud through
/// [`resolve`] if the fallback also found nothing.
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
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{parse_password, read_token_file};
    use std::io::Write as _;

    #[test]
    fn a_credential_reply_yields_its_password() {
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
        assert_eq!(parse_password("password=\n"), None);
    }

    #[test]
    fn a_username_is_not_mistaken_for_a_password() {
        let reply = "username=password-manager\npassword=ghp_example\n";
        assert_eq!(parse_password(reply).as_deref(), Some("ghp_example"));
    }

    #[test]
    fn reading_a_token_file_trims_trailing_newline() {
        let mut file = tempfile::NamedTempFile::new().expect("tempfile");
        writeln!(file, "ghp_example").expect("write");
        let s = read_token_file(file.path()).expect("resolved");
        assert_eq!(s.expose(), "ghp_example");
    }

    #[test]
    fn an_empty_token_file_is_refused() {
        let file = tempfile::NamedTempFile::new().expect("tempfile");
        let err = read_token_file(file.path()).expect_err("empty");
        assert!(err.to_string().contains("empty"), "was: {err}");
    }

    #[test]
    fn a_missing_token_file_is_refused_naming_the_path() {
        let err = read_token_file(std::path::Path::new("/nope/nope/nope.txt"))
            .expect_err("missing");
        assert!(err.to_string().contains("nope.txt"), "was: {err}");
    }

    // Env-variable precedence and the "no credential at all" failure mode are exercised in
    // process-level integration tests (`crates/cli/tests/`), because asserting them here
    // would need `std::env::set_var`, which is `unsafe` in Rust 2024 and forbidden by the
    // workspace lint `unsafe_code = "forbid"` (ADR-0002, invariant 3).
}
