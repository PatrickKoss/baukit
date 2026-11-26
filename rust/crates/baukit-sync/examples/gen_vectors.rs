//! Writes `fixtures/sync/hlc-generated-v1.json`: HLC step traces from a fixed
//! seed, shared by the Rust and TypeScript clocks.

use std::{error::Error, fs, path::PathBuf};

use baukit_sync::hlc::{
    COUNTERS_PER_MILLISECOND, DEFAULT_MAX_DRIFT_MS, HlcError, HlcStep, MAX_ENCODED_TIMESTAMP,
    encode, now_step, observe_step,
};
use serde_json::{Value, json};

const SEED: u64 = 0x0b0b_5eed_2026_1125;
const RANDOM_TRACES: usize = 24;
const STEPS: usize = 16;
const BASE_MS: i64 = 1_767_225_600_000;
const MAX_WALL_MS: i64 = MAX_ENCODED_TIMESTAMP / COUNTERS_PER_MILLISECOND;

struct Random(u64);

impl Random {
    fn below(&mut self, bound: u64) -> i64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        i64::try_from(self.0 % bound).unwrap_or(0)
    }
}

const fn code(error: HlcError) -> &'static str {
    match error {
        HlcError::InvalidComponent => "invalid_component",
        HlcError::InvalidTimestamp => "invalid_timestamp",
        HlcError::ExceedsSafeInteger => "exceeds_safe_integer",
        HlcError::InvalidPhysicalClock => "invalid_physical_clock",
        HlcError::EmptyDeviceId => "empty_device_id",
        HlcError::RemoteTooFarAhead => "remote_too_far_ahead",
        _ => "invalid_max_drift",
    }
}

enum Operation {
    Now { physical: i64 },
    Observe { physical: i64, remote: i64 },
}

fn trace(name: &str, start: HlcStep, operations: &[Operation]) -> Value {
    let mut state = start;
    let steps: Vec<Value> = operations
        .iter()
        .map(|operation| {
            let (op, physical, remote, result) = match *operation {
                Operation::Now { physical } => ("now", physical, None, now_step(state, physical)),
                Operation::Observe { physical, remote } => (
                    "observe",
                    physical,
                    Some(remote),
                    observe_step(state, physical, remote, DEFAULT_MAX_DRIFT_MS),
                ),
            };
            let outcome = result.and_then(|next| Ok((next, encode(next.0, next.1)?)));
            let expect = match outcome {
                Ok((next, timestamp)) => {
                    state = next;
                    json!({ "state": [next.0, next.1], "timestamp": timestamp })
                }
                Err(error) => json!({ "error": code(error) }),
            };
            json!({ "op": op, "physical": physical, "remote": remote, "expect": expect })
        })
        .collect();
    json!({ "name": name, "start": [start.0, start.1], "steps": steps })
}

fn random_trace(random: &mut Random, index: usize) -> Value {
    let mut physical = BASE_MS + random.below(1_000);
    let operations: Vec<Operation> = (0..STEPS)
        .map(|_| {
            physical += random.below(5) - 2;
            if random.below(2) == 0 {
                return Operation::Now { physical };
            }
            let wall = physical + random.below(20) - 10;
            let remote = wall * COUNTERS_PER_MILLISECOND + random.below(4) + 1;
            Operation::Observe { physical, remote }
        })
        .collect();
    trace(&format!("random-{index}"), (BASE_MS, 0), &operations)
}

fn edge_traces() -> Vec<Value> {
    let last_counter = COUNTERS_PER_MILLISECOND - 1;
    vec![
        trace(
            "counter-rollover",
            (BASE_MS, last_counter - 1),
            &[
                Operation::Now { physical: BASE_MS },
                Operation::Now { physical: BASE_MS },
                Operation::Now { physical: BASE_MS },
            ],
        ),
        trace(
            "exhaustion",
            (
                MAX_WALL_MS,
                MAX_ENCODED_TIMESTAMP % COUNTERS_PER_MILLISECOND - 2,
            ),
            &[
                Operation::Now {
                    physical: MAX_WALL_MS,
                },
                Operation::Now {
                    physical: MAX_WALL_MS,
                },
                Operation::Now {
                    physical: MAX_WALL_MS,
                },
            ],
        ),
        trace(
            "drift-rejection",
            (BASE_MS, 0),
            &[
                Operation::Observe {
                    physical: BASE_MS,
                    remote: (BASE_MS + DEFAULT_MAX_DRIFT_MS) * COUNTERS_PER_MILLISECOND + 1,
                },
                Operation::Observe {
                    physical: BASE_MS,
                    remote: (BASE_MS + DEFAULT_MAX_DRIFT_MS + 1) * COUNTERS_PER_MILLISECOND + 1,
                },
                Operation::Now { physical: BASE_MS },
            ],
        ),
    ]
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut random = Random(SEED);
    let mut traces: Vec<Value> = (0..RANDOM_TRACES)
        .map(|index| random_trace(&mut random, index))
        .collect();
    traces.extend(edge_traces());
    let document = json!({
        "version": 1,
        "seed": SEED.to_string(),
        "maxDriftMs": DEFAULT_MAX_DRIFT_MS,
        "traces": traces,
    });
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/sync/hlc-generated-v1.json");
    fs::create_dir_all(path.parent().ok_or("fixture path has no parent")?)?;
    fs::write(
        &path,
        format!("{}\n", serde_json::to_string_pretty(&document)?),
    )?;
    Ok(())
}
