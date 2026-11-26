//! `BreakerSet` and `CircuitBreaker` behaviour on a manual clock.

use std::sync::Arc;
use std::time::Duration;

use baukit_resilience::{
    Admission, BreakerPolicy, BreakerSet, BreakerState, CircuitBreaker, DEFAULT_MAX_KEYS,
    JitterSource, ManualClock, Outcome, Permit, SplitMixJitter, SystemClock,
};

fn trip(admit: impl Fn() -> Admission, record: impl Fn(Permit, Outcome)) {
    for _ in 0..BreakerPolicy::default().consecutive_timeouts {
        if let Some(permit) = admit().permit() {
            record(permit, Outcome::Timeout);
        }
    }
}

fn set(clock: &Arc<ManualClock>) -> BreakerSet<String> {
    BreakerSet::with_parts(
        BreakerPolicy::default(),
        clock.clone(),
        Box::new(SplitMixJitter::with_seed(1)),
    )
}

#[test]
fn keys_are_independent_and_worst_state_reports_the_open_one() {
    let clock = Arc::new(ManualClock::new());
    let breakers = set(&clock);
    let dead = "dead".to_owned();
    let alive = "alive".to_owned();
    assert!(matches!(breakers.admit(&dead), Admission::Allowed(_)));
    assert!(matches!(breakers.admit(&alive), Admission::Allowed(_)));
    trip(
        || breakers.admit(&dead),
        |permit, outcome| breakers.record(&dead, permit, outcome),
    );
    assert_eq!(breakers.state(&dead), BreakerState::Open);
    assert!(!breakers.admit(&dead).is_admitted());
    assert!(matches!(breakers.admit(&alive), Admission::Allowed(_)));
    assert_eq!(breakers.worst_state(), BreakerState::Open);
}

#[test]
fn clones_share_state() {
    let clock = Arc::new(ManualClock::new());
    let breakers = set(&clock);
    let other = breakers.clone();
    let key = "peer".to_owned();
    let _ = breakers.admit(&key);
    trip(
        || other.admit(&key),
        |permit, outcome| other.record(&key, permit, outcome),
    );
    assert_eq!(breakers.state(&key), BreakerState::Open);
}

#[test]
fn eviction_drops_the_longest_idle_closed_key_and_keeps_open_ones() {
    let clock = Arc::new(ManualClock::new());
    let breakers = set(&clock).with_max_keys(3);
    let open = "open".to_owned();
    let _ = breakers.admit(&open);
    trip(
        || breakers.admit(&open),
        |permit, outcome| breakers.record(&open, permit, outcome),
    );
    clock.advance(Duration::from_millis(1));
    let _ = breakers.admit(&"idle".to_owned());
    clock.advance(Duration::from_millis(1));
    let _ = breakers.admit(&"busy".to_owned());
    clock.advance(Duration::from_millis(1));
    let _ = breakers.admit(&"new".to_owned());
    assert_eq!(breakers.len(), 3);
    assert_eq!(breakers.state(&open), BreakerState::Open);
    let _ = breakers.admit(&"newer".to_owned());
    assert_eq!(breakers.state(&open), BreakerState::Open);
    assert_eq!(breakers.len(), 3);
}

#[test]
fn default_cap_is_256() {
    let clock = Arc::new(ManualClock::new());
    let breakers = set(&clock);
    for index in 0..(DEFAULT_MAX_KEYS + 10) {
        let _ = breakers.admit(&index.to_string());
    }
    assert_eq!(breakers.len(), DEFAULT_MAX_KEYS);
}

#[test]
fn record_for_unknown_key_is_ignored() {
    let clock = Arc::new(ManualClock::new());
    let breakers = set(&clock);
    let permit = set(&clock).admit(&"elsewhere".to_owned()).permit().unwrap();
    breakers.record(&"never".to_owned(), permit, Outcome::Failure);
    assert!(breakers.is_empty());
}

#[test]
fn circuit_breaker_recovers_through_a_probe() {
    let clock = Arc::new(ManualClock::new());
    let breaker = CircuitBreaker::with_parts(
        BreakerPolicy::default(),
        clock.clone(),
        Box::new(SplitMixJitter::with_seed(2)),
    );
    trip(
        || breaker.admit(),
        |permit, outcome| breaker.record(permit, outcome),
    );
    let Admission::Rejected { retry_after } = breaker.admit() else {
        panic!("breaker should be open");
    };
    assert!(retry_after <= Duration::from_secs(30));
    assert!(retry_after >= Duration::from_secs(24));
    clock.advance(retry_after);
    let Admission::Probe(probe) = breaker.admit() else {
        panic!("expected the probe");
    };
    assert!(!breaker.admit().is_admitted());
    breaker.record(probe, Outcome::Success);
    assert_eq!(breaker.state(), BreakerState::Closed);
    assert!(matches!(breaker.admit(), Admission::Allowed(_)));
}

struct FnJitter(Box<dyn FnMut() -> f64 + Send>);

impl JitterSource for FnJitter {
    fn next_fraction(&mut self) -> f64 {
        (self.0)()
    }
}

#[test]
fn a_poisoned_lock_does_not_panic_later_callers() {
    let mut armed = true;
    let breaker = CircuitBreaker::with_parts(
        BreakerPolicy::default(),
        Arc::new(SystemClock),
        Box::new(FnJitter(Box::new(move || {
            assert!(!std::mem::replace(&mut armed, false), "poison the lock");
            0.0
        }))),
    );
    let clone = breaker.clone();
    let permit = breaker.admit().permit().unwrap();
    let poisoned = std::thread::spawn(move || clone.record(permit, Outcome::Failure)).join();
    assert!(poisoned.is_err());
    assert!(matches!(breaker.admit(), Admission::Allowed(_)));
    trip(
        || breaker.admit(),
        |permit, outcome| breaker.record(permit, outcome),
    );
    assert_eq!(breaker.state(), BreakerState::Open);
}

#[test]
fn permits_from_an_evicted_breaker_do_not_match_its_replacement() {
    let clock = Arc::new(ManualClock::new());
    let breakers = set(&clock).with_max_keys(1);
    let key = "peer".to_owned();
    let old = breakers.admit(&key).permit().unwrap();
    let _ = breakers.admit(&"other".to_owned());
    let _ = breakers.admit(&key);
    for _ in 0..BreakerPolicy::default().consecutive_timeouts {
        breakers.record(&key, old, Outcome::Timeout);
    }
    assert_eq!(breakers.state(&key), BreakerState::Closed);
}
