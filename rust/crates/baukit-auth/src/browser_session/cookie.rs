use std::time::Duration;

use axum::http::{HeaderMap, HeaderValue, header::COOKIE};

use super::crypto::SessionCredential;

/// Cookie name and attributes for one deployment mode.
#[derive(Clone, Debug)]
pub(crate) struct CookieSettings {
    name: String,
    secure: bool,
}

impl CookieSettings {
    pub(crate) const fn new(name: String, secure: bool) -> Self {
        Self { name, secure }
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn set(
        &self,
        credential: &SessionCredential,
        max_age: Duration,
    ) -> Option<HeaderValue> {
        self.header(&credential.encode(), max_age.as_secs())
    }

    pub(crate) fn clear(&self) -> Option<HeaderValue> {
        self.header("", 0)
    }

    fn header(&self, value: &str, max_age: u64) -> Option<HeaderValue> {
        let secure = if self.secure { "; Secure" } else { "" };
        let header = zeroize::Zeroizing::new(format!(
            "{}={value}; Path=/{secure}; HttpOnly; SameSite=Strict; Max-Age={max_age}",
            self.name
        ));
        let mut header = HeaderValue::from_str(&header).ok()?;
        header.set_sensitive(true);
        Some(header)
    }

    /// Returns the first well-formed session cookie in the request.
    pub(crate) fn read(&self, headers: &HeaderMap) -> Option<SessionCredential> {
        headers
            .get_all(COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(';'))
            .filter_map(|pair| pair.trim().split_once('='))
            .find(|(name, _)| *name == self.name)
            .and_then(|(_, value)| SessionCredential::parse(value))
    }
}
