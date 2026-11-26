//! OIDC JWKS adapter for [`KeySetCache`].

use std::{sync::Arc, time::Duration};

use baukit_egress::GuardedClient;
use baukit_resilience::{Clock, StalePolicy, SystemClock};
use reqwest::Url;

use super::{Jwk, JwkSet, VerificationError};
use crate::key_set_cache::{KeyLookupError, KeySet, KeySetCache, KeySource, KeySourceFuture};
use crate::oidc_http::{self, Document};

const FAILURE_COOLDOWN: Duration = Duration::from_secs(10);
const OIDC_SOURCE: &str = "oidc";

impl KeySet for JwkSet {
    type Key = Jwk;

    fn key(&self, key_id: &str) -> Option<&Jwk> {
        self.find(key_id)
    }
}

/// Fetches an issuer's JWKS through the guarded client.
pub(super) struct JwksSource {
    client: GuardedClient,
    jwks_uri: Url,
}

impl KeySource for JwksSource {
    type Set = JwkSet;
    type Error = VerificationError;

    fn fetch(&self) -> KeySourceFuture<'_, JwkSet, VerificationError> {
        Box::pin(oidc_http::get_json(
            &self.client,
            self.jwks_uri.clone(),
            Document::Jwks,
        ))
    }
}

pub(super) type KeyCache = KeySetCache<JwksSource>;

pub(super) fn key_cache(
    client: GuardedClient,
    jwks_uri: Url,
    fresh_for: Duration,
    stale_for: Duration,
) -> KeyCache {
    key_cache_with_clock(
        client,
        jwks_uri,
        fresh_for,
        stale_for,
        Arc::new(SystemClock),
    )
}

pub(super) fn key_cache_with_clock(
    client: GuardedClient,
    jwks_uri: Url,
    fresh_for: Duration,
    stale_for: Duration,
    clock: Arc<dyn Clock>,
) -> KeyCache {
    let policy = StalePolicy::new(fresh_for, stale_for, FAILURE_COOLDOWN);
    KeySetCache::with_clock(JwksSource { client, jwks_uri }, policy, OIDC_SOURCE, clock)
}

/// A cache with the same endpoint and clock but no cached keys.
pub(super) fn emptied(cache: &KeyCache, fresh_for: Duration, stale_for: Duration) -> KeyCache {
    let source = cache.source();
    key_cache_with_clock(
        source.client.clone(),
        source.jwks_uri.clone(),
        fresh_for,
        stale_for,
        cache.clock(),
    )
}

pub(super) fn jwks_uri(cache: &KeyCache) -> &Url {
    &cache.source().jwks_uri
}

pub(super) async fn key_for(cache: &KeyCache, key_id: &str) -> Result<Jwk, VerificationError> {
    cache.key_for(key_id).await.map_err(|error| match error {
        KeyLookupError::UnknownKeyId => VerificationError::UnknownKeyId,
        KeyLookupError::Fetch(error) => error,
        KeyLookupError::Unavailable { .. } => VerificationError::JwksUnavailable,
    })
}

#[cfg(test)]
pub(super) async fn fetch(cache: &KeyCache) -> Result<JwkSet, VerificationError> {
    cache.source().fetch().await
}
