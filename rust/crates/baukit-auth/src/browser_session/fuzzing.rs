//! Cookie parsing hooks for the fuzz targets. Not a public API.

use axum::http::{HeaderMap, HeaderValue, header::COOKIE};

use super::{cookie::CookieSettings, crypto::SessionCredential};

/// Reads the session cookie `name` from raw `Cookie` header lines and returns
/// the re-encoded credential.
#[must_use]
pub fn read_session_cookie(name: &str, cookie_headers: &[&[u8]]) -> Option<String> {
    let mut headers = HeaderMap::new();
    for line in cookie_headers {
        headers.append(COOKIE, HeaderValue::from_bytes(line).ok()?);
    }
    CookieSettings::new(name.to_owned(), true)
        .read(&headers)
        .map(|credential| credential.encode().to_string())
}

/// Parses one cookie value and returns the re-encoded credential.
#[must_use]
pub fn parse_session_credential(value: &str) -> Option<String> {
    SessionCredential::parse(value).map(|credential| credential.encode().to_string())
}
