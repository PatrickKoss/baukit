//! Cookie glue for axum routes.

use std::time::Duration;

use axum::http::{HeaderMap, HeaderValue, header::COOKIE};

use super::{BrowserBinding, OAUTH_BINDING_COOKIE, OAuthStateError, binding_from_cookie_header};

/// `Set-Cookie` value for the binding. `SameSite=Lax` because the provider
/// redirect back is a top-level cross-site GET.
///
/// # Errors
///
/// [`OAuthStateError::Malformed`] if the value is not a valid header (cannot
/// happen for a generated binding).
pub fn binding_set_cookie(
    binding: &BrowserBinding,
    max_age: Duration,
) -> Result<HeaderValue, OAuthStateError> {
    HeaderValue::try_from(format!(
        "{OAUTH_BINDING_COOKIE}={}; Max-Age={}; Path=/; Secure; HttpOnly; SameSite=Lax",
        binding.expose(),
        max_age.as_secs()
    ))
    .map_err(|_| OAuthStateError::Malformed)
}

/// `Set-Cookie` value that removes the binding after the callback.
#[must_use]
pub fn binding_clear_cookie() -> HeaderValue {
    HeaderValue::from_static(
        "__Host-baukit-oauth=; Max-Age=0; Path=/; Secure; HttpOnly; SameSite=Lax",
    )
}

/// Reads the binding from the request's `Cookie` headers.
///
/// # Errors
///
/// [`OAuthStateError::BrowserMismatch`] when the cookie is missing,
/// [`OAuthStateError::Malformed`] when it is not a token.
pub fn binding_from_headers(headers: &HeaderMap) -> Result<BrowserBinding, OAuthStateError> {
    let value = headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|header| header.to_str().ok())
        .find_map(binding_from_cookie_header)
        .ok_or(OAuthStateError::BrowserMismatch)?;
    BrowserBinding::parse(value)
}
