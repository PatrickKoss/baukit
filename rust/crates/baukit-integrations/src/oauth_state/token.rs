//! Opaque random tokens, their wire form and their hashes.

use std::fmt;

use aws_lc_rs::{
    digest::{SHA256, digest},
    rand::{SecureRandom as _, SystemRandom},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use subtle::ConstantTimeEq as _;

use super::OAuthStateError;

/// Random bytes behind every state, binding and PKCE verifier (256 bits).
pub const TOKEN_BYTES: usize = 32;
/// Length of a token in unpadded URL-safe base64.
pub const TOKEN_CHARS: usize = 43;

/// SHA-256 of a token. The store keeps only these for state and binding.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TokenHash(pub [u8; 32]);

impl TokenHash {
    fn of(token: &str) -> Self {
        let mut bytes = [0_u8; 32];
        bytes.copy_from_slice(digest(&SHA256, token.as_bytes()).as_ref());
        Self(bytes)
    }

    /// Compares two hashes in constant time.
    #[must_use]
    pub fn matches(&self, other: &Self) -> bool {
        self.0.ct_eq(&other.0).into()
    }
}

impl fmt::Debug for TokenHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TokenHash(..)")
    }
}

pub(crate) fn random_token(random: &SystemRandom) -> Result<String, OAuthStateError> {
    let mut bytes = [0_u8; TOKEN_BYTES];
    random
        .fill(&mut bytes)
        .map_err(|_| OAuthStateError::Random)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn parse_token(raw: &str) -> Result<String, OAuthStateError> {
    if raw.len() != TOKEN_CHARS {
        return Err(OAuthStateError::Malformed);
    }
    let decoded = URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|_| OAuthStateError::Malformed)?;
    if decoded.len() != TOKEN_BYTES {
        return Err(OAuthStateError::Malformed);
    }
    Ok(raw.to_owned())
}

macro_rules! secret_token {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, PartialEq, Eq)]
        pub struct $name(String);

        impl $name {
            /// Parses the wire form: 43 characters of unpadded URL-safe base64.
            ///
            /// # Errors
            ///
            /// [`OAuthStateError::Malformed`] for any other input.
            pub fn parse(raw: &str) -> Result<Self, OAuthStateError> {
                parse_token(raw).map(Self)
            }

            /// The wire form, for the URL or the cookie.
            #[must_use]
            pub fn expose(&self) -> &str {
                &self.0
            }

            /// SHA-256 of the wire form.
            #[must_use]
            pub fn hash(&self) -> TokenHash {
                TokenHash::of(&self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($name), "(..)"))
            }
        }
    };
}

secret_token!(
    /// The opaque `state` query parameter.
    OAuthState
);
secret_token!(
    /// The browser binding carried in the `__Host-` cookie.
    BrowserBinding
);

impl OAuthState {
    pub(crate) fn generate(random: &SystemRandom) -> Result<Self, OAuthStateError> {
        random_token(random).map(Self)
    }
}

impl BrowserBinding {
    pub(crate) fn generate(random: &SystemRandom) -> Result<Self, OAuthStateError> {
        random_token(random).map(Self)
    }
}

/// PKCE code verifier (RFC 7636). Sent only to the provider's token endpoint.
#[derive(Clone, PartialEq, Eq)]
pub struct PkceVerifier(String);

impl PkceVerifier {
    pub(crate) fn generate(random: &SystemRandom) -> Result<Self, OAuthStateError> {
        random_token(random).map(Self)
    }

    /// Wraps a verifier read back from a store.
    #[must_use]
    pub fn from_stored(value: String) -> Self {
        Self(value)
    }

    /// The verifier for the token request.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// The `S256` challenge for the authorize URL.
    #[must_use]
    pub fn challenge(&self) -> String {
        URL_SAFE_NO_PAD.encode(digest(&SHA256, self.0.as_bytes()))
    }
}

impl fmt::Debug for PkceVerifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PkceVerifier(..)")
    }
}

/// Name of the binding cookie. `__Host-` pins it to this origin, `Path=/` and `Secure`.
pub const OAUTH_BINDING_COOKIE: &str = "__Host-baukit-oauth";

/// Finds the binding cookie's raw value in one `Cookie` header line.
#[must_use]
pub fn binding_from_cookie_header(header: &str) -> Option<&str> {
    header.split(';').find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        (name == OAUTH_BINDING_COOKIE).then_some(value)
    })
}

#[cfg(kani)]
mod proofs {
    use super::*;

    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    fn sextet(byte: u8) -> Option<usize> {
        ALPHABET.iter().position(|candidate| *candidate == byte)
    }

    /// K-18: for any 43 ASCII bytes, and for any other length up to 44,
    /// `parse_token` accepts iff the length is 43, every byte is in the
    /// URL-safe alphabet, and the last sextet has its two unused bits clear,
    /// which is exactly a canonical base64url round trip. Unwind 65.
    #[kani::proof]
    #[kani::unwind(65)]
    fn parse_token_accepts_only_canonical_tokens() {
        let bytes: [u8; TOKEN_CHARS + 1] = kani::any();
        let len: usize = kani::any();
        kani::assume(len == TOKEN_CHARS || len == TOKEN_CHARS + 1 || len == TOKEN_CHARS - 1);
        let Some(input) = bytes.get(..len) else {
            return;
        };
        kani::assume(input.is_ascii());
        let raw: String = input.iter().copied().map(char::from).collect();
        let alphabet = input.iter().all(|byte| sextet(*byte).is_some());
        let canonical_tail = input
            .last()
            .and_then(|byte| sextet(*byte))
            .is_some_and(|value| value % 4 == 0);
        let expected = len == TOKEN_CHARS && alphabet && canonical_tail;
        assert_eq!(parse_token(&raw).is_ok(), expected);
    }
}
