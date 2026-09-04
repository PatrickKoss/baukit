use std::{
    collections::HashMap,
    future::ready,
    sync::{Arc, Mutex},
};

use baukit_auth::{
    ApiToken, ApiTokenRecord, ApiTokenService, ApiTokenStore, ApiTokenStoreError,
    ApiTokenStoreFuture, StoredApiToken,
};
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// In-memory [`ApiTokenStore`] for tests that need the real token service.
///
/// The store keeps the same invariants a product adapter must keep: it holds
/// only the SHA-256 digest, returns revoked and expired tokens so the service
/// decides the policy, and scopes revocation and listing to one owner.
///
/// ```
/// use baukit_auth::{ApiTokenService, NewApiToken};
/// use baukit_test::InMemoryApiTokenStore;
/// use std::sync::Arc;
/// use uuid::Uuid;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let store = Arc::new(InMemoryApiTokenStore::new());
/// let service = ApiTokenService::new(store.clone());
/// let owner_id = Uuid::now_v7();
///
/// let issued = service.issue(owner_id, NewApiToken::new("CI")).await?;
/// assert_eq!(service.verify(&issued.secret).await?.owner_id, owner_id);
/// assert!(!store.stored_hashes().contains(&issued.secret.into_bytes()));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Default)]
pub struct InMemoryApiTokenStore {
    state: Mutex<StoreState>,
}

#[derive(Debug, Default)]
struct StoreState {
    tokens: HashMap<Uuid, ApiToken>,
    hashes: HashMap<Vec<u8>, Uuid>,
    failure: Option<ApiTokenStoreError>,
}

impl InMemoryApiTokenStore {
    /// Creates an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a store and the service issuing into it.
    #[must_use]
    pub fn with_service() -> (Arc<Self>, ApiTokenService) {
        let store = Arc::new(Self::new());
        let service = ApiTokenService::new(store.clone());
        (store, service)
    }

    /// Makes every later operation fail with a typed store error.
    ///
    /// Use it to script either an internal failure or a safe policy rejection.
    pub fn fail_with(&self, error: ApiTokenStoreError) {
        self.lock().failure = Some(error);
    }

    /// Returns how many tokens the store currently holds.
    #[must_use]
    pub fn token_count(&self) -> usize {
        self.lock().tokens.len()
    }

    /// Returns every digest the store holds, so a test can assert that no
    /// stored byte sequence is the plaintext secret.
    #[must_use]
    pub fn stored_hashes(&self) -> Vec<Vec<u8>> {
        self.lock().hashes.keys().cloned().collect()
    }

    /// Returns one stored token by id.
    #[must_use]
    pub fn token(&self, token_id: Uuid) -> Option<ApiToken> {
        self.lock().tokens.get(&token_id).cloned()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, StoreState> {
        self.state.lock().unwrap_or_else(|poisoned| {
            self.state.clear_poison();
            poisoned.into_inner()
        })
    }
}

impl ApiTokenStore for InMemoryApiTokenStore {
    fn create(
        &self,
        record: ApiTokenRecord,
    ) -> ApiTokenStoreFuture<'_, Result<ApiToken, ApiTokenStoreError>> {
        let mut state = self.lock();
        if let Some(failure) = state.failure.clone() {
            return Box::pin(ready(Err(failure)));
        }
        let token = ApiToken {
            id: record.id,
            owner_id: record.owner_id,
            name: record.name,
            display_prefix: record.display_prefix,
            created_at: record.created_at,
            expires_at: record.expires_at,
            last_used_at: None,
            revoked_at: None,
        };
        state.hashes.insert(record.secret_hash, token.id);
        state.tokens.insert(token.id, token.clone());
        Box::pin(ready(Ok(token)))
    }

    fn find_by_hash<'a>(
        &'a self,
        secret_hash: &'a [u8],
    ) -> ApiTokenStoreFuture<'a, Result<Option<StoredApiToken>, ApiTokenStoreError>> {
        let state = self.lock();
        if let Some(failure) = state.failure.clone() {
            return Box::pin(ready(Err(failure)));
        }
        let found = state
            .hashes
            .get(secret_hash)
            .and_then(|token_id| state.tokens.get(token_id))
            .cloned()
            .map(|token| StoredApiToken {
                token,
                secret_hash: secret_hash.to_vec(),
            });
        Box::pin(ready(Ok(found)))
    }

    fn touch_last_used(
        &self,
        token_id: Uuid,
        used_at: DateTime<Utc>,
    ) -> ApiTokenStoreFuture<'_, Result<(), ApiTokenStoreError>> {
        let mut state = self.lock();
        if let Some(failure) = state.failure.clone() {
            return Box::pin(ready(Err(failure)));
        }
        if let Some(token) = state.tokens.get_mut(&token_id) {
            token.last_used_at = Some(used_at);
        }
        Box::pin(ready(Ok(())))
    }

    fn revoke(
        &self,
        owner_id: Uuid,
        token_id: Uuid,
        revoked_at: DateTime<Utc>,
    ) -> ApiTokenStoreFuture<'_, Result<bool, ApiTokenStoreError>> {
        let mut state = self.lock();
        if let Some(failure) = state.failure.clone() {
            return Box::pin(ready(Err(failure)));
        }
        let revoked = state
            .tokens
            .get_mut(&token_id)
            .filter(|token| token.owner_id == owner_id && token.revoked_at.is_none())
            .is_some_and(|token| {
                token.revoked_at = Some(revoked_at);
                true
            });
        Box::pin(ready(Ok(revoked)))
    }

    fn list_for_owner(
        &self,
        owner_id: Uuid,
    ) -> ApiTokenStoreFuture<'_, Result<Vec<ApiToken>, ApiTokenStoreError>> {
        let state = self.lock();
        if let Some(failure) = state.failure.clone() {
            return Box::pin(ready(Err(failure)));
        }
        let mut owned: Vec<ApiToken> = state
            .tokens
            .values()
            .filter(|token| token.owner_id == owner_id)
            .cloned()
            .collect();
        owned.sort_by_key(|token| std::cmp::Reverse(token.created_at));
        Box::pin(ready(Ok(owned)))
    }
}

#[cfg(test)]
mod tests {
    use baukit_auth::{ApiTokenError, ApiTokenPolicyRejection, NewApiToken};

    use super::*;

    #[tokio::test]
    async fn store_scripts_internal_failures() {
        let (store, service) = InMemoryApiTokenStore::with_service();
        store.fail_with(ApiTokenStoreError::internal(
            "SELECT token_hash FROM api_tokens failed",
        ));

        let error = service
            .list_for_owner(Uuid::now_v7())
            .await
            .expect_err("scripted operation must fail");

        assert_eq!(error.to_string(), "API token storage failed");
        assert!(matches!(error, ApiTokenError::Storage(_)));
    }

    #[tokio::test]
    async fn store_scripts_policy_rejections() {
        let (store, service) = InMemoryApiTokenStore::with_service();
        let rejection = ApiTokenPolicyRejection::new("api_tokens_active_limit_exceeded")
            .expect("valid code")
            .with_detail("maximum", 10)
            .expect("valid detail");
        store.fail_with(ApiTokenStoreError::PolicyRejected(rejection.clone()));

        let error = service
            .issue(Uuid::now_v7(), NewApiToken::new("Over limit"))
            .await
            .expect_err("scripted operation must fail");

        assert!(matches!(
            error,
            ApiTokenError::PolicyRejected(actual) if actual == rejection
        ));
    }
}
