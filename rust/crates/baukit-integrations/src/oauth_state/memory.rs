use std::{
    collections::HashMap,
    sync::{Mutex, PoisonError},
};

use chrono::{DateTime, Utc};

use super::{
    ConsumeOutcome, OAuthStateError, OAuthStateFuture, OAuthStateStore, PendingOAuthFlow, TokenHash,
};

#[derive(Default)]
struct Records {
    pending: HashMap<TokenHash, PendingOAuthFlow>,
    used: HashMap<TokenHash, DateTime<Utc>>,
}

/// Process-local [`OAuthStateStore`] for tests and single-instance products.
#[derive(Default)]
pub struct InMemoryOAuthStateStore {
    records: Mutex<Records>,
}

impl InMemoryOAuthStateStore {
    fn with_records<T>(&self, f: impl FnOnce(&mut Records) -> T) -> T {
        let mut records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut records)
    }
}

impl std::fmt::Debug for InMemoryOAuthStateStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InMemoryOAuthStateStore")
            .finish_non_exhaustive()
    }
}

impl OAuthStateStore for InMemoryOAuthStateStore {
    fn insert(&self, flow: PendingOAuthFlow) -> OAuthStateFuture<'_, Result<(), OAuthStateError>> {
        let result = self.with_records(|records| {
            if records.pending.contains_key(&flow.state_hash)
                || records.used.contains_key(&flow.state_hash)
            {
                return Err(OAuthStateError::Store("duplicate oauth state".into()));
            }
            records.pending.insert(flow.state_hash, flow);
            Ok(())
        });
        Box::pin(async move { result })
    }

    fn consume<'a>(
        &'a self,
        state_hash: &'a TokenHash,
    ) -> OAuthStateFuture<'a, Result<ConsumeOutcome, OAuthStateError>> {
        let outcome = self.with_records(|records| {
            if let Some(flow) = records.pending.remove(state_hash) {
                records.used.insert(*state_hash, flow.expires_at);
                return ConsumeOutcome::Consumed(flow);
            }
            if records.used.contains_key(state_hash) {
                return ConsumeOutcome::AlreadyUsed;
            }
            ConsumeOutcome::Unknown
        });
        Box::pin(async move { Ok(outcome) })
    }

    fn purge_expired(
        &self,
        now: DateTime<Utc>,
    ) -> OAuthStateFuture<'_, Result<u64, OAuthStateError>> {
        let removed = self.with_records(|records| {
            let before = records.pending.len() + records.used.len();
            records.pending.retain(|_, flow| flow.expires_at > now);
            records.used.retain(|_, expires_at| *expires_at > now);
            before - (records.pending.len() + records.used.len())
        });
        let removed = u64::try_from(removed)
            .map_err(|_| OAuthStateError::Store("purge count exceeds u64".into()));
        Box::pin(async move { removed })
    }
}
