use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use baukit_http::RetryClass;
use baukit_resilience::{Admission, BreakerPolicy, BreakerSet, Outcome, Permit};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use url::Url;

use crate::{
    EgressError,
    metrics::{Destination, record_state, record_transition},
};

const MAX_BULKHEAD_KEYS: usize = 256;

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
pub(crate) struct DestinationKey {
    scheme: String,
    host: String,
    port: Option<u16>,
}

impl DestinationKey {
    fn of(url: &Url) -> Self {
        Self {
            scheme: url.scheme().to_owned(),
            host: url.host_str().unwrap_or_default().to_owned(),
            port: url.port_or_known_default(),
        }
    }
}

/// The breaker and bulkhead state of one client, shared by its clones.
#[derive(Clone)]
pub(crate) struct Guard {
    destination: Destination,
    breakers: Option<BreakerSet<DestinationKey>>,
    bulkheads: Arc<Mutex<HashMap<DestinationKey, Arc<Semaphore>>>>,
    max_in_flight: usize,
}

/// A call that passed the bulkhead and the breaker.
pub(crate) struct Admitted {
    key: DestinationKey,
    permit: Option<Permit>,
    slot: OwnedSemaphorePermit,
}

impl Guard {
    pub(crate) fn new(
        destination: Destination,
        policy: Option<BreakerPolicy>,
        max_in_flight: usize,
    ) -> Self {
        let breakers = policy.map(BreakerSet::new);
        if breakers.is_some() {
            record_state(&destination, baukit_resilience::BreakerState::Closed);
        }
        Self {
            destination,
            breakers,
            bulkheads: Arc::new(Mutex::new(HashMap::new())),
            max_in_flight,
        }
    }

    pub(crate) const fn destination(&self) -> &Destination {
        &self.destination
    }

    pub(crate) fn admit(&self, url: &Url) -> Result<Admitted, EgressError> {
        let key = DestinationKey::of(url);
        let slot = self
            .bulkhead(&key)
            .try_acquire_owned()
            .map_err(|_| EgressError::Saturated)?;
        let Some(breakers) = &self.breakers else {
            return Ok(Admitted {
                key,
                permit: None,
                slot,
            });
        };
        let before = breakers.state(&key);
        let admission = breakers.admit(&key);
        self.note_transition(breakers, &key, before);
        match admission {
            Admission::Allowed(permit) | Admission::Probe(permit) => Ok(Admitted {
                key,
                permit: Some(permit),
                slot,
            }),
            Admission::Rejected { retry_after } => Err(EgressError::CircuitOpen { retry_after }),
        }
    }

    /// Records the outcome and hands back the bulkhead slot.
    pub(crate) fn finish(&self, admitted: Admitted, outcome: Outcome) -> OwnedSemaphorePermit {
        let Admitted { key, permit, slot } = admitted;
        if let (Some(breakers), Some(permit)) = (&self.breakers, permit) {
            let before = breakers.state(&key);
            breakers.record(&key, permit, outcome);
            self.note_transition(breakers, &key, before);
        }
        slot
    }

    fn note_transition(
        &self,
        breakers: &BreakerSet<DestinationKey>,
        key: &DestinationKey,
        before: baukit_resilience::BreakerState,
    ) {
        let after = breakers.state(key);
        if after == before {
            return;
        }
        record_transition(&self.destination, after);
        record_state(&self.destination, breakers.worst_state());
    }

    fn bulkhead(&self, key: &DestinationKey) -> Arc<Semaphore> {
        let mut bulkheads = self
            .bulkheads
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(semaphore) = bulkheads.get(key) {
            return Arc::clone(semaphore);
        }
        if bulkheads.len() >= MAX_BULKHEAD_KEYS {
            bulkheads.retain(|_, semaphore| Arc::strong_count(semaphore) > 1);
        }
        let semaphore = Arc::new(Semaphore::new(self.max_in_flight));
        bulkheads.insert(key.clone(), Arc::clone(&semaphore));
        semaphore
    }
}

/// How a status the client turned into [`EgressError::Status`] counts (D4).
pub(crate) const fn status_outcome(class: RetryClass) -> Outcome {
    match class {
        RetryClass::RetryAfter(delay) => Outcome::HoldFor(delay),
        RetryClass::Unavailable => Outcome::Failure,
        RetryClass::Timeout => Outcome::Timeout,
        RetryClass::RateLimited | RetryClass::Revoked | RetryClass::Permanent => Outcome::Ignored,
    }
}

/// How a call that ended without a status counts (D4).
///
/// A timeout counts only when the client's own timeout fired. One cut short by
/// the caller's deadline says nothing about the destination.
pub(crate) const fn error_outcome(error: &EgressError, deadline_capped: bool) -> Outcome {
    match error {
        EgressError::Timeout if deadline_capped => Outcome::Ignored,
        EgressError::Timeout => Outcome::Timeout,
        EgressError::Resolve | EgressError::Transport(_) => Outcome::Failure,
        _ => Outcome::Ignored,
    }
}

/// Delay a caller should wait after a full bulkhead.
pub(crate) const SATURATED_RETRY_AFTER: Duration = Duration::from_secs(1);
