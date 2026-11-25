//! Slice 0020: `AuthorLogin::new` stores the trimmed value. Fixes the ledgered bug where
//! `" alice"` passed validation and was sent to the provider with its leading space.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use contributron_domain::pull_request::AuthorLogin;

#[test]
fn a_padded_login_is_stored_trimmed() {
    let login = AuthorLogin::new(" alice ").expect("padded is accepted");
    assert_eq!(login.as_str(), "alice");
}

#[test]
fn a_padded_login_compares_equal_to_its_unpadded_form() {
    let padded = AuthorLogin::new("  alice\n").expect("padded is accepted");
    let plain = AuthorLogin::new("alice").expect("plain is accepted");
    assert_eq!(padded, plain);
}

#[test]
fn an_empty_login_is_still_rejected() {
    AuthorLogin::new("").expect_err("empty remains invalid");
    AuthorLogin::new("   ").expect_err("whitespace only remains invalid");
}
