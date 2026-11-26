//! Invariants of the breaker over generated event sequences.

use std::time::{Duration, Instant};

use baukit_resilience::{Admission, BreakerMachine, BreakerPolicy, BreakerState, Outcome, Permit};
use proptest::prelude::*;

#[derive(Debug, Clone)]
enum Step {
    Admit {
        advance_ms: u64,
    },
    Finish {
        advance_ms: u64,
        pick: prop::sample::Index,
        outcome: Outcome,
        fraction: f64,
    },
}

fn outcome() -> impl Strategy<Value = Outcome> {
    prop_oneof![
        Just(Outcome::Success),
        Just(Outcome::Failure),
        Just(Outcome::Timeout),
        Just(Outcome::Ignored),
        (0_u64..1_000_000).prop_map(|ms| Outcome::HoldFor(Duration::from_millis(ms))),
    ]
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        (0_u64..120_000).prop_map(|advance_ms| Step::Admit { advance_ms }),
        (
            0_u64..5_000,
            any::<prop::sample::Index>(),
            outcome(),
            0.0_f64..1.0
        )
            .prop_map(|(advance_ms, pick, outcome, fraction)| Step::Finish {
                advance_ms,
                pick,
                outcome,
                fraction,
            }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Outcomes are recorded with any permit issued so far, in any order, so
    /// late closed-phase calls and replaced probes report at random times.
    #[test]
    fn probes_and_late_outcomes_respect_the_ticket(steps in prop::collection::vec(step(), 1..400)) {
        let policy = BreakerPolicy::default();
        let mut machine = BreakerMachine::new();
        let mut now = Instant::now();
        let mut issued: Vec<Permit> = Vec::new();
        let mut live_probe: Option<(Permit, Instant)> = None;
        for step in steps {
            match step {
                Step::Admit { advance_ms } => {
                    now += Duration::from_millis(advance_ms);
                    let admission = machine.admit(&policy, now);
                    match admission {
                        Admission::Probe(permit) => {
                            let still_live = live_probe
                                .is_some_and(|(_, since)| now.duration_since(since) < policy.probe_timeout);
                            prop_assert!(!still_live, "a second probe while one is live");
                            live_probe = Some((permit, now));
                        }
                        Admission::Allowed(_) => prop_assert_eq!(machine.state(), BreakerState::Closed),
                        Admission::Rejected { retry_after } => {
                            prop_assert!(retry_after <= policy.open_max);
                            prop_assert!(retry_after > Duration::ZERO);
                        }
                    }
                    issued.extend(admission.permit());
                }
                Step::Finish { advance_ms, pick, outcome, fraction } => {
                    if issued.is_empty() {
                        continue;
                    }
                    now += Duration::from_millis(advance_ms);
                    let permit = issued[pick.index(issued.len())];
                    let before = machine.state();
                    machine.record(&policy, permit, outcome, now, fraction);
                    let after = machine.state();
                    let current_probe = live_probe.map(|(probe, _)| probe) == Some(permit);
                    if before != BreakerState::Closed && after == BreakerState::Closed {
                        prop_assert!(current_probe && outcome == Outcome::Success, "closed without a current probe success");
                    }
                    if before == BreakerState::HalfOpen && after != BreakerState::HalfOpen {
                        prop_assert!(current_probe, "half-open decided by a permit other than the current probe");
                    }
                    if current_probe || after != BreakerState::HalfOpen {
                        live_probe = None;
                    }
                }
            }
        }
    }

    #[test]
    fn open_duration_never_exceeds_cap(opens in 0_u32..u32::MAX, fraction in -1.0_f64..2.0) {
        let policy = BreakerPolicy::default();
        let duration = policy.open_duration(opens, fraction);
        prop_assert!(duration <= policy.open_max);
        prop_assert!(duration >= policy.open_base.min(policy.open_max).mul_f64(0.8));
    }
}

#[test]
fn open_duration_doubles_until_the_cap() {
    let policy = BreakerPolicy::default();
    let seconds: Vec<u64> = (1..=6)
        .map(|opens| policy.open_duration(opens, 0.0).as_secs())
        .collect();
    assert_eq!(seconds, [30, 60, 120, 240, 300, 300]);
}
