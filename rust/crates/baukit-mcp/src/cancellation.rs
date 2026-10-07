use std::{
    collections::{HashMap, hash_map::Entry},
    sync::{Mutex, MutexGuard, PoisonError},
};

use rmcp::{ErrorData, model::RequestId};

use crate::{CancellationToken, Principal};

#[derive(Hash, PartialEq, Eq)]
struct RequestKey {
    issuer: Option<String>,
    subject: String,
    client: Option<String>,
    id: RequestId,
}

impl RequestKey {
    fn new(principal: &Principal, id: &RequestId) -> Self {
        Self {
            issuer: principal.issuer().map(str::to_owned),
            subject: principal.subject().to_owned(),
            client: principal.client_id().map(str::to_owned),
            id: id.clone(),
        }
    }
}

#[derive(Default)]
pub(crate) struct ActiveRequests(Mutex<HashMap<RequestKey, CancellationToken>>);

impl ActiveRequests {
    fn lock(&self) -> MutexGuard<'_, HashMap<RequestKey, CancellationToken>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn register(
        &self,
        principal: &Principal,
        id: &RequestId,
        token: CancellationToken,
    ) -> Result<RequestGuard<'_>, ErrorData> {
        let key = RequestKey::new(principal, id);
        match self.lock().entry(RequestKey::new(principal, id)) {
            Entry::Occupied(_) => Err(ErrorData::invalid_request(
                "Request id already active",
                None,
            )),
            Entry::Vacant(entry) => {
                entry.insert(token);
                Ok(RequestGuard {
                    requests: self,
                    key,
                })
            }
        }
    }

    pub fn cancel(&self, principal: &Principal, id: &RequestId) {
        if let Some(token) = self.lock().get(&RequestKey::new(principal, id)) {
            token.cancel();
        }
    }
}

pub(crate) struct RequestGuard<'a> {
    requests: &'a ActiveRequests,
    key: RequestKey,
}

impl Drop for RequestGuard<'_> {
    fn drop(&mut self) {
        self.requests.lock().remove(&self.key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_requests_release_ids_and_unknown_or_foreign_ids_are_ignored() {
        let requests = ActiveRequests::default();
        let alice = Principal::new("alice");
        let bob = Principal::new("bob");
        let id = RequestId::Number(7);
        let token = CancellationToken::new();
        let guard = requests
            .register(&alice, &id, token.clone())
            .expect("request");
        assert!(
            requests
                .register(&alice, &id, CancellationToken::new())
                .is_err()
        );
        requests.cancel(&bob, &id);
        requests.cancel(&alice, &RequestId::Number(8));
        assert!(!token.is_cancelled());
        requests.cancel(&alice, &id);
        assert!(token.is_cancelled());
        drop(guard);
        assert!(requests.lock().is_empty());
        let next = CancellationToken::new();
        let _guard = requests
            .register(&alice, &id, next.clone())
            .expect("reused id");
        assert!(!next.is_cancelled());
    }
}
