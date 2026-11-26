//! Invariants of the breaker over generated event sequences.

use std::time::{Duration, Instant};

use baukit_resilience::{Admission, BreakerMachine, BreakerPolicy, BreakerState, Outcome};
use proptest::prelude::*;

#[derive(Debug, Clone)]
enum Step {
    Admit {
        advance_ms: u64,
    },
    Finish {
        advance_ms: u64,
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
        (0_u64..5_000, outcome(), 0.0_f64..1.0).prop_map(|(advance_ms, outcome, fraction)| {
            Step::Finish {
                advance_ms,
                outcome,
                fraction,
            }
        }),
    ]
}

struct Probe {
    admitted_at: Instant,
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn at_most_one_live_probe_and_open_time_within_cap(steps in prop::collection::vec(step(), 1..400)) {
        let policy = BreakerPolicy::default();
        let mut machine = BreakerMachine::new();
        let mut now = Instant::now();
        let mut probes: Vec<Probe> = Vec::new();
        for step in steps {
            match step {
                Step::Admit { advance_ms } => {
                    now += Duration::from_millis(advance_ms);
                    match machine.admit(&policy, now) {
                        Admission::Probe => {
                            probes.retain(|probe| now.duration_since(probe.admitted_at) < policy.probe_timeout);
                            prop_assert!(probes.is_empty(), "a second probe while one is live");
                            probes.push(Probe { admitted_at: now });
                        }
                        Admission::Allowed => {
                            prop_assert_eq!(machine.state(), BreakerState::Closed);
                        }
                        Admission::Rejected { retry_after } => {
                            prop_assert!(retry_after <= policy.open_max);
                            prop_assert!(retry_after > Duration::ZERO);
                        }
                    }
                }
                Step::Finish { advance_ms, outcome, fraction } => {
                    now += Duration::from_millis(advance_ms);
                    probes.clear();
                    machine.record(&policy, outcome, now, fraction);
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
