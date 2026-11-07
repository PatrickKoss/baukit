//! Single-use, browser-bound OAuth `state` for provider-linking flows.
//!
//! [`OAuthStateService::start`] mints an opaque state, a browser binding for a
//! `__Host-` cookie and, when the provider supports it, a PKCE verifier. The
//! store keeps a hash of the state and of the binding. [`OAuthStateService::complete`]
//! consumes the record atomically and checks the cookie in constant time, so a
//! victim who opens an attacker-started authorize URL fails with
//! [`OAuthStateError::BrowserMismatch`].

mod memory;
mod token;

#[cfg(feature = "axum")]
pub mod axum;
#[cfg(feature = "postgres")]
pub mod postgres;

use std::{fmt, future::Future, pin::Pin, sync::Arc, time::Duration};

use aws_lc_rs::rand::SystemRandom;
use chrono::{DateTime, Utc};
use thiserror::Error;

pub use memory::InMemoryOAuthStateStore;
pub use token::{
    BrowserBinding, OAUTH_BINDING_COOKIE, OAuthState, PkceVerifier, TOKEN_CHARS, TokenHash,
    binding_from_cookie_header,
};

/// Lifetime of a pending flow and of the binding cookie.
pub const DEFAULT_OAUTH_STATE_TTL: Duration = Duration::from_secs(600);

/// Boxed future returned by [`OAuthStateStore`].
pub type OAuthStateFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Why a flow could not start or complete. Each callback failure is its own variant.
#[derive(Debug, Error)]
pub enum OAuthStateError {
    /// The state or cookie is not a well-formed token.
    #[error("oauth state is malformed")]
    Malformed,
    /// No pending flow has this state.
    #[error("oauth state is unknown")]
    Unknown,
    /// The flow existed but its TTL passed. It is consumed now.
    #[error("oauth state expired")]
    Expired,
    /// The flow was already completed once.
    #[error("oauth state was already used")]
    AlreadyUsed,
    /// The callback came from a browser that did not start the flow.
    #[error("oauth state belongs to another browser")]
    BrowserMismatch,
    /// The system random source failed.
    #[error("random source failed")]
    Random,
    /// The TTL does not fit the clock.
    #[error("oauth state ttl is out of range")]
    InvalidTtl,
    /// The store failed.
    #[error("oauth state store failed: {0}")]
    Store(String),
}

/// What [`OAuthStateService::start`] stores. Owner-agnostic: the state hash is the key.
#[derive(Clone)]
pub struct PendingOAuthFlow {
    /// SHA-256 of the state.
    pub state_hash: TokenHash,
    /// SHA-256 of the browser binding.
    pub binding_hash: TokenHash,
    /// Product user that started the flow.
    pub user: String,
    /// Provider id, such as `github`.
    pub provider: String,
    /// Where the product sends the browser after the callback.
    pub redirect: String,
    /// PKCE verifier, when the provider supports PKCE.
    pub pkce_verifier: Option<PkceVerifier>,
    /// End of the flow's lifetime.
    pub expires_at: DateTime<Utc>,
}

impl fmt::Debug for PendingOAuthFlow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PendingOAuthFlow")
            .field("user", &self.user)
            .field("provider", &self.provider)
            .field("redirect", &self.redirect)
            .field("pkce_verifier", &self.pkce_verifier)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// Result of an atomic consume.
#[derive(Debug)]
pub enum ConsumeOutcome {
    /// The record was pending and is gone now.
    Consumed(PendingOAuthFlow),
    /// A previous consume took it.
    AlreadyUsed,
    /// No record, live or used, has this hash.
    Unknown,
}

/// Port for pending flows.
pub trait OAuthStateStore: Send + Sync {
    /// Stores a new pending flow.
    ///
    /// # Errors
    ///
    /// [`OAuthStateError::Store`] when the backend fails.
    fn insert(&self, flow: PendingOAuthFlow) -> OAuthStateFuture<'_, Result<(), OAuthStateError>>;

    /// Removes the flow and returns it. Exactly one concurrent caller gets
    /// [`ConsumeOutcome::Consumed`]; later callers see [`ConsumeOutcome::AlreadyUsed`]
    /// until the record's TTL passes.
    ///
    /// # Errors
    ///
    /// [`OAuthStateError::Store`] when the backend fails.
    fn consume<'a>(
        &'a self,
        state_hash: &'a TokenHash,
    ) -> OAuthStateFuture<'a, Result<ConsumeOutcome, OAuthStateError>>;

    /// Drops pending flows and used markers that expired before `now`.
    ///
    /// # Errors
    ///
    /// [`OAuthStateError::Store`] when the backend fails.
    fn purge_expired(
        &self,
        now: DateTime<Utc>,
    ) -> OAuthStateFuture<'_, Result<u64, OAuthStateError>>;
}

/// Whether the provider takes a PKCE challenge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pkce {
    /// Generate an `S256` verifier and challenge.
    S256,
    /// The provider does not support PKCE.
    Unsupported,
}

/// Input to [`OAuthStateService::start`].
#[derive(Clone, Debug)]
pub struct StartOAuthFlow {
    /// Product user starting the flow.
    pub user: String,
    /// Provider id.
    pub provider: String,
    /// Product redirect after the callback.
    pub redirect: String,
    /// Whether to generate PKCE.
    pub pkce: Pkce,
}

/// What the product puts in the authorize URL and the cookie.
pub struct StartedOAuthFlow {
    /// `state` query parameter.
    pub state: OAuthState,
    /// `code_challenge` (method `S256`), when PKCE applies.
    pub pkce_challenge: Option<String>,
    /// Value for the binding cookie.
    pub binding: BrowserBinding,
    /// Cookie max-age.
    pub max_age: Duration,
}

impl fmt::Debug for StartedOAuthFlow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StartedOAuthFlow")
            .field("state", &self.state)
            .field("binding", &self.binding)
            .field("max_age", &self.max_age)
            .finish_non_exhaustive()
    }
}

/// What a successful callback yields.
#[derive(Debug)]
pub struct CompletedOAuthFlow {
    /// User that started the flow. Link the provider account to this user.
    pub user: String,
    /// Provider id.
    pub provider: String,
    /// Product redirect.
    pub redirect: String,
    /// Verifier for the token request.
    pub pkce_verifier: Option<PkceVerifier>,
}

/// Starts and completes flows against an [`OAuthStateStore`].
#[derive(Clone)]
pub struct OAuthStateService {
    store: Arc<dyn OAuthStateStore>,
    random: SystemRandom,
    ttl: Duration,
}

impl OAuthStateService {
    /// Service with [`DEFAULT_OAUTH_STATE_TTL`].
    #[must_use]
    pub fn new(store: Arc<dyn OAuthStateStore>) -> Self {
        Self {
            store,
            random: SystemRandom::new(),
            ttl: DEFAULT_OAUTH_STATE_TTL,
        }
    }

    /// Replaces the TTL.
    #[must_use]
    pub fn with_ttl(self, ttl: Duration) -> Self {
        Self { ttl, ..self }
    }

    /// Starts a flow now.
    ///
    /// # Errors
    ///
    /// [`OAuthStateError::Random`], [`OAuthStateError::InvalidTtl`] or a store error.
    pub async fn start(&self, input: StartOAuthFlow) -> Result<StartedOAuthFlow, OAuthStateError> {
        self.start_at(input, Utc::now()).await
    }

    /// Starts a flow at `now`.
    ///
    /// # Errors
    ///
    /// [`OAuthStateError::Random`], [`OAuthStateError::InvalidTtl`] or a store error.
    pub async fn start_at(
        &self,
        input: StartOAuthFlow,
        now: DateTime<Utc>,
    ) -> Result<StartedOAuthFlow, OAuthStateError> {
        let ttl = chrono::Duration::from_std(self.ttl).map_err(|_| OAuthStateError::InvalidTtl)?;
        let expires_at = now
            .checked_add_signed(ttl)
            .ok_or(OAuthStateError::InvalidTtl)?;
        let state = OAuthState::generate(&self.random)?;
        let binding = BrowserBinding::generate(&self.random)?;
        let pkce_verifier = match input.pkce {
            Pkce::S256 => Some(PkceVerifier::generate(&self.random)?),
            Pkce::Unsupported => None,
        };
        let pkce_challenge = pkce_verifier.as_ref().map(PkceVerifier::challenge);
        self.store
            .insert(PendingOAuthFlow {
                state_hash: state.hash(),
                binding_hash: binding.hash(),
                user: input.user,
                provider: input.provider,
                redirect: input.redirect,
                pkce_verifier,
                expires_at,
            })
            .await?;
        Ok(StartedOAuthFlow {
            state,
            pkce_challenge,
            binding,
            max_age: self.ttl,
        })
    }

    /// Completes a flow now.
    ///
    /// # Errors
    ///
    /// See [`Self::complete_at`].
    pub async fn complete(
        &self,
        state: &OAuthState,
        binding: &BrowserBinding,
    ) -> Result<CompletedOAuthFlow, OAuthStateError> {
        self.complete_at(state, binding, Utc::now()).await
    }

    /// Consumes the flow, then checks expiry and the browser binding. A failed
    /// check still burns the state.
    ///
    /// # Errors
    ///
    /// [`OAuthStateError::Unknown`], [`OAuthStateError::AlreadyUsed`],
    /// [`OAuthStateError::Expired`], [`OAuthStateError::BrowserMismatch`] or a store error.
    pub async fn complete_at(
        &self,
        state: &OAuthState,
        binding: &BrowserBinding,
        now: DateTime<Utc>,
    ) -> Result<CompletedOAuthFlow, OAuthStateError> {
        let flow = match self.store.consume(&state.hash()).await? {
            ConsumeOutcome::Consumed(flow) => flow,
            ConsumeOutcome::AlreadyUsed => return Err(OAuthStateError::AlreadyUsed),
            ConsumeOutcome::Unknown => return Err(OAuthStateError::Unknown),
        };
        if flow.expires_at <= now {
            return Err(OAuthStateError::Expired);
        }
        if !flow.binding_hash.matches(&binding.hash()) {
            return Err(OAuthStateError::BrowserMismatch);
        }
        Ok(CompletedOAuthFlow {
            user: flow.user,
            provider: flow.provider,
            redirect: flow.redirect,
            pkce_verifier: flow.pkce_verifier,
        })
    }
}

impl fmt::Debug for OAuthStateService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OAuthStateService")
            .field("ttl", &self.ttl)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
