use std::fmt;

use zeroize::Zeroize;

use super::ApiToken;

/// A newly issued token together with the only copy of its secret.
///
/// The secret exists in memory once. Return it to the caller in the creation
/// response and drop it. Nothing else can recover it from the store. Debug
/// output redacts the secret, and the type has no `PartialEq` so the secret is
/// never compared in variable time.
#[derive(Clone)]
pub struct IssuedApiToken {
    /// The stored token metadata.
    pub token: ApiToken,
    /// The full presented token, including its marker.
    pub secret: String,
}

/// A token secret generated without a store, with its hash and display prefix.
///
/// Show the secret once in the creation response. Store only
/// [`secret_hash()`](Self::secret_hash) and optionally
/// [`display_prefix()`](Self::display_prefix). Look up presented credentials by
/// `hash_api_token(presented)`, as [`ApiTokenVerifier`](crate::ApiTokenVerifier) does.
///
/// Debug output redacts the secret and hash. Dropping this value zeroizes the
/// secret unless [`into_secret()`](Self::into_secret) transfers it to the caller.
pub struct GeneratedApiToken {
    pub(super) secret: String,
    pub(super) secret_hash: Vec<u8>,
    pub(super) display_prefix: String,
}

impl GeneratedApiToken {
    /// Returns the full secret, including its marker, to show once.
    #[must_use]
    pub fn secret(&self) -> &str {
        &self.secret
    }

    /// Returns the [`hash_api_token`](crate::hash_api_token) digest to store for later lookups.
    #[must_use]
    pub fn secret_hash(&self) -> &[u8] {
        &self.secret_hash
    }

    /// Returns the leading characters safe to store and display.
    #[must_use]
    pub fn display_prefix(&self) -> &str {
        &self.display_prefix
    }

    /// Transfers the secret to the caller without copying or zeroizing it.
    #[must_use]
    pub fn into_secret(mut self) -> String {
        std::mem::take(&mut self.secret)
    }
}

impl fmt::Debug for IssuedApiToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("IssuedApiToken")
            .field("token", &self.token)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

impl fmt::Debug for GeneratedApiToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GeneratedApiToken")
            .field("secret", &"[REDACTED]")
            .field("secret_hash", &"[REDACTED]")
            .field("display_prefix", &self.display_prefix)
            .finish()
    }
}

impl Drop for GeneratedApiToken {
    fn drop(&mut self) {
        self.secret.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use chrono::Utc;
    use uuid::Uuid;

    use super::{ApiToken, IssuedApiToken};

    #[test]
    fn issued_api_token_debug_redacts_secret() {
        let issued = IssuedApiToken {
            token: ApiToken {
                id: Uuid::nil(),
                owner_id: Uuid::nil(),
                name: "ci".to_owned(),
                display_prefix: "bk_live_abcd".to_owned(),
                grants: BTreeSet::new(),
                created_at: Utc::now(),
                expires_at: None,
                last_used_at: None,
                revoked_at: None,
            },
            secret: "bk_live_abcd_super-secret-value".to_owned(),
        };

        let debug = format!("{issued:?}");

        assert!(!debug.contains("super-secret-value"));
        assert!(debug.contains("secret: \"[REDACTED]\""));
        assert!(debug.contains("bk_live_abcd"));
    }
}
