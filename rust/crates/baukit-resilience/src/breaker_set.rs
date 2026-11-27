//! One breaker per key under a shared policy.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::hash::Hash;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use crate::breaker::{Admission, BreakerMachine, BreakerPolicy, BreakerState, Outcome, Permit};
use crate::clock::{Clock, SystemClock};
use crate::jitter::{JitterSource, SplitMixJitter};

/// Keys a [`BreakerSet`] tracks before it evicts one.
pub const DEFAULT_MAX_KEYS: usize = 256;

struct Tracked {
    machine: BreakerMachine,
    last_used: Instant,
}

const GENERATION_SPACING_BITS: u32 = 32;

struct Entries<K> {
    breakers: HashMap<K, Tracked>,
    next_epoch: u64,
    jitter: Box<dyn JitterSource>,
}

/// Breakers keyed by destination. Clones share state.
///
/// When a new key arrives at capacity, the longest-idle closed breaker is
/// evicted. If none is closed, the longest-idle breaker of any state goes.
pub struct BreakerSet<K> {
    policy: BreakerPolicy,
    max_keys: usize,
    clock: Arc<dyn Clock>,
    entries: Arc<Mutex<Entries<K>>>,
}

impl<K> Clone for BreakerSet<K> {
    fn clone(&self) -> Self {
        Self {
            policy: self.policy,
            max_keys: self.max_keys,
            clock: Arc::clone(&self.clock),
            entries: Arc::clone(&self.entries),
        }
    }
}

impl<K> std::fmt::Debug for BreakerSet<K> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BreakerSet")
            .field("policy", &self.policy)
            .field("max_keys", &self.max_keys)
            .finish_non_exhaustive()
    }
}

impl<K: Hash + Eq + Clone> BreakerSet<K> {
    /// A set on the system clock with entropy-seeded jitter and [`DEFAULT_MAX_KEYS`].
    #[must_use]
    pub fn new(policy: BreakerPolicy) -> Self {
        Self::with_parts(
            policy,
            Arc::new(SystemClock),
            Box::new(SplitMixJitter::from_entropy()),
        )
    }

    /// A set on the given clock and jitter source.
    #[must_use]
    pub fn with_parts(
        policy: BreakerPolicy,
        clock: Arc<dyn Clock>,
        jitter: Box<dyn JitterSource>,
    ) -> Self {
        Self {
            policy,
            max_keys: DEFAULT_MAX_KEYS,
            clock,
            entries: Arc::new(Mutex::new(Entries {
                breakers: HashMap::new(),
                next_epoch: 0,
                jitter,
            })),
        }
    }

    /// Replaces the key cap (at least one).
    #[must_use]
    pub fn with_max_keys(mut self, max_keys: usize) -> Self {
        self.max_keys = max_keys.max(1);
        self
    }

    /// The shared policy.
    #[must_use]
    pub const fn policy(&self) -> &BreakerPolicy {
        &self.policy
    }

    /// Decides whether a call to `key` may go ahead now.
    #[must_use]
    pub fn admit(&self, key: &K) -> Admission {
        let now = self.clock.now();
        let mut entries = self.lock();
        let Entries {
            breakers,
            next_epoch,
            ..
        } = &mut *entries;
        if !breakers.contains_key(key) {
            evict_if_full(breakers, self.max_keys);
        }
        let tracked = match breakers.entry(key.clone()) {
            Entry::Occupied(occupied) => occupied.into_mut(),
            Entry::Vacant(vacant) => {
                let start = *next_epoch << GENERATION_SPACING_BITS;
                *next_epoch = next_epoch.wrapping_add(1);
                vacant.insert(Tracked {
                    machine: BreakerMachine::starting_at(start),
                    last_used: now,
                })
            }
        };
        tracked.last_used = now;
        tracked.machine.admit(&self.policy, now)
    }

    /// Records the outcome of the call to `key` admitted with `permit`.
    /// Unknown keys (never admitted or evicted since) are ignored.
    pub fn record(&self, key: &K, permit: Permit, outcome: Outcome) {
        let now = self.clock.now();
        let mut entries = self.lock();
        let fraction = entries.jitter.next_fraction();
        let Some(tracked) = entries.breakers.get_mut(key) else {
            return;
        };
        tracked.last_used = now;
        tracked
            .machine
            .record(&self.policy, permit, outcome, now, fraction);
    }

    /// The state of `key`'s breaker; untracked keys are closed.
    #[must_use]
    pub fn state(&self, key: &K) -> BreakerState {
        self.lock()
            .breakers
            .get(key)
            .map_or(BreakerState::Closed, |tracked| tracked.machine.state())
    }

    /// The least healthy state across all keys, closed when empty.
    #[must_use]
    pub fn worst_state(&self) -> BreakerState {
        self.lock()
            .breakers
            .values()
            .map(|tracked| tracked.machine.state())
            .max()
            .unwrap_or(BreakerState::Closed)
    }

    /// Number of tracked keys.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().breakers.len()
    }

    /// Whether no key is tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> MutexGuard<'_, Entries<K>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn evict_if_full<K: Hash + Eq + Clone>(breakers: &mut HashMap<K, Tracked>, max_keys: usize) {
    if breakers.len() < max_keys {
        return;
    }
    let idlest = |closed_only: bool| {
        breakers
            .iter()
            .filter(|(_, tracked)| !closed_only || tracked.machine.state() == BreakerState::Closed)
            .min_by_key(|(_, tracked)| tracked.last_used)
            .map(|(key, _)| key.clone())
    };
    if let Some(key) = idlest(true).or_else(|| idlest(false)) {
        breakers.remove(&key);
    }
}
