//! Replays `fixtures/resilience/*.json`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use baukit_resilience::{
    Admission, Backoff, BackoffStrategy, BreakerMachine, BreakerPolicy, BreakerState, Outcome,
    Permit,
};
use serde::Deserialize;

fn fixture(name: &str) -> std::io::Result<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/resilience")
        .join(name);
    std::fs::read_to_string(path)
}

#[derive(Deserialize)]
struct BackoffFile {
    version: u32,
    cases: Vec<BackoffCase>,
}

#[derive(Deserialize)]
struct BackoffCase {
    strategy: String,
    initial_ms: u64,
    max_ms: u64,
    attempt: u32,
    fraction: f64,
    expected_delay_ms: u64,
}

#[test]
fn backoff_vectors() {
    let file: BackoffFile = serde_json::from_str(&fixture("backoff-v1.json").unwrap()).unwrap();
    assert_eq!(file.version, 1);
    assert!(!file.cases.is_empty());
    for case in file.cases {
        let strategy = match case.strategy.as_str() {
            "full_jitter" => BackoffStrategy::FullJitter,
            "decorrelated_jitter" => BackoffStrategy::DecorrelatedJitter,
            other => panic!("unknown strategy {other}"),
        };
        let backoff = Backoff::new(
            Duration::from_millis(case.initial_ms),
            Duration::from_millis(case.max_ms),
        )
        .with_strategy(strategy);
        assert_eq!(
            backoff.delay(case.attempt, case.fraction),
            Duration::from_millis(case.expected_delay_ms),
            "{} initial={} max={} attempt={} fraction={}",
            case.strategy,
            case.initial_ms,
            case.max_ms,
            case.attempt,
            case.fraction
        );
    }
}

#[derive(Deserialize)]
struct BreakerFile {
    version: u32,
    policy: PolicyVector,
    scenarios: Vec<Scenario>,
}

#[derive(Deserialize)]
struct PolicyVector {
    window: u32,
    min_calls: u32,
    failure_rate_percent: u32,
    consecutive_timeouts: u32,
    open_base_ms: u64,
    open_max_ms: u64,
    jitter_percent: u32,
    probe_timeout_ms: u64,
}

impl From<PolicyVector> for BreakerPolicy {
    fn from(vector: PolicyVector) -> Self {
        Self {
            window: vector.window,
            min_calls: vector.min_calls,
            failure_rate_percent: vector.failure_rate_percent,
            consecutive_timeouts: vector.consecutive_timeouts,
            open_base: Duration::from_millis(vector.open_base_ms),
            open_max: Duration::from_millis(vector.open_max_ms),
            jitter_percent: vector.jitter_percent,
            probe_timeout: Duration::from_millis(vector.probe_timeout_ms),
        }
    }
}

#[derive(Deserialize)]
struct Scenario {
    name: String,
    events: Vec<Event>,
}

#[derive(Deserialize)]
struct Event {
    at_ms: u64,
    op: String,
    expect_state: String,
    expect_admission: Option<String>,
    expect_retry_after_ms: Option<u64>,
    permit: Option<String>,
    outcome: Option<String>,
    fraction: Option<f64>,
    hold_for_ms: Option<u64>,
}

fn outcome(event: &Event) -> Option<Outcome> {
    match event.outcome.as_deref()? {
        "success" => Some(Outcome::Success),
        "failure" => Some(Outcome::Failure),
        "timeout" => Some(Outcome::Timeout),
        "ignored" => Some(Outcome::Ignored),
        "hold_for" => event
            .hold_for_ms
            .map(|ms| Outcome::HoldFor(Duration::from_millis(ms))),
        _ => None,
    }
}

fn admission_matches(admission: Admission, event: &Event) -> bool {
    match (admission, event.expect_admission.as_deref()) {
        (Admission::Allowed(_), Some("allowed")) | (Admission::Probe(_), Some("probe")) => true,
        (Admission::Rejected { retry_after }, Some("rejected")) => {
            event.expect_retry_after_ms.map(Duration::from_millis) == Some(retry_after)
        }
        _ => false,
    }
}

#[test]
fn breaker_vectors() {
    let file: BreakerFile = serde_json::from_str(&fixture("breaker-v1.json").unwrap()).unwrap();
    assert_eq!(file.version, 1);
    let policy = BreakerPolicy::from(file.policy);
    assert_eq!(
        policy,
        BreakerPolicy::default(),
        "the vectors pin the default policy"
    );
    let start = Instant::now();
    for scenario in file.scenarios {
        let mut machine = BreakerMachine::new();
        let mut permits: HashMap<String, Permit> = HashMap::new();
        for (index, event) in scenario.events.iter().enumerate() {
            let now = start + Duration::from_millis(event.at_ms);
            let context = format!("{} event {index}", scenario.name);
            match event.op.as_str() {
                "admit" => {
                    let admission = machine.admit(&policy, now);
                    assert!(
                        admission_matches(admission, event),
                        "{context}: got {admission:?}"
                    );
                    if let (Some(label), Some(permit)) = (&event.permit, admission.permit()) {
                        permits.insert(label.clone(), permit);
                    }
                }
                "record" => {
                    let outcome =
                        outcome(event).unwrap_or_else(|| panic!("{context}: bad outcome"));
                    let label = event
                        .permit
                        .as_deref()
                        .unwrap_or_else(|| panic!("{context}: no permit"));
                    machine.record(
                        &policy,
                        permits[label],
                        outcome,
                        now,
                        event.fraction.unwrap(),
                    );
                }
                other => panic!("unknown op {other}"),
            }
            assert_eq!(machine.state().as_str(), event.expect_state, "{context}");
        }
    }
}

#[test]
fn state_labels_are_stable() {
    assert_eq!(BreakerState::Closed.as_str(), "closed");
    assert_eq!(BreakerState::HalfOpen.as_str(), "half_open");
    assert_eq!(BreakerState::Open.as_str(), "open");
}
