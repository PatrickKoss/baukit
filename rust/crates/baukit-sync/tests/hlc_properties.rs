use std::{cell::Cell, cmp::Ordering, rc::Rc};

use baukit_sync::hlc::{self, HybridLogicalClock, MAX_ENCODED_TIMESTAMP};
use proptest::prelude::*;

const COUNTERS_PER_MILLISECOND: i64 = 1_000;
const MAX_WALL_TIME_MS: i64 = MAX_ENCODED_TIMESTAMP / COUNTERS_PER_MILLISECOND - 1;

fn timestamp() -> impl Strategy<Value = i64> {
    1..=MAX_ENCODED_TIMESTAMP
}

#[derive(Clone, Debug)]
enum Step {
    Now(i64),
    Observe(i64, i64),
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        (0..=MAX_WALL_TIME_MS).prop_map(Step::Now),
        ((0..=MAX_WALL_TIME_MS), timestamp())
            .prop_map(|(physical, remote)| Step::Observe(physical, remote)),
    ]
}

proptest! {
    #[test]
    fn decode_inverts_encode(wall in 0..=MAX_WALL_TIME_MS, counter in 0..COUNTERS_PER_MILLISECOND) {
        let encoded = hlc::encode(wall, counter).expect("encodable");
        prop_assert_eq!(hlc::decode(encoded), Ok((wall, counter)));
    }

    #[test]
    fn encode_inverts_decode(value in timestamp()) {
        let (wall, counter) = hlc::decode(value).expect("decodable");
        prop_assert_eq!(hlc::encode(wall, counter), Ok(value));
    }

    #[test]
    fn invalid_timestamps_never_decode(value in any::<i64>()) {
        prop_assume!(!(1..=MAX_ENCODED_TIMESTAMP).contains(&value));
        prop_assert!(hlc::decode(value).is_err());
        prop_assert_eq!(hlc::compare(value, 1), None);
    }

    #[test]
    fn compare_is_a_total_order_matching_components(left in timestamp(), right in timestamp()) {
        let ordering = hlc::compare(left, right).expect("valid inputs");
        prop_assert_eq!(hlc::compare(right, left), Some(ordering.reverse()));
        let left_parts = hlc::decode(left).expect("left");
        let right_parts = hlc::decode(right).expect("right");
        prop_assert_eq!(ordering, left_parts.cmp(&right_parts));
    }

    #[test]
    fn clock_is_strictly_monotonic_and_after_observed(steps in prop::collection::vec(step(), 1..64)) {
        let physical = Rc::new(Cell::new(0));
        let reading = Rc::clone(&physical);
        let mut clock = HybridLogicalClock::open("device-a", move || reading.get(), None).expect("clock");
        let mut last: Option<i64> = None;
        for step in steps {
            let (next, remote) = match step {
                Step::Now(now) => {
                    physical.set(now);
                    (clock.now(), None)
                }
                Step::Observe(now, remote) => {
                    physical.set(now);
                    (clock.observe(remote), Some(remote))
                }
            };
            let Ok(next) = next else { continue };
            if let Some(previous) = last {
                prop_assert_eq!(hlc::compare(next, previous), Some(Ordering::Greater));
            }
            if let Some(remote) = remote {
                prop_assert_eq!(hlc::compare(next, remote), Some(Ordering::Greater));
            }
            let (wall, _) = hlc::decode(next).expect("valid output");
            prop_assert!(wall >= physical.get());
            last = Some(next);
        }
    }

    #[test]
    fn snapshot_reopen_continues_after_last_timestamp(wall in 0..=MAX_WALL_TIME_MS, counter in 0..COUNTERS_PER_MILLISECOND) {
        let mut clock = HybridLogicalClock::open("device-a", || 0, None).expect("clock");
        let Ok(observed) = clock.observe(hlc::encode(wall, counter).expect("encodable")) else {
            return Ok(());
        };
        let mut reopened = HybridLogicalClock::open("device-a", || 0, Some(clock.snapshot())).expect("reopen");
        prop_assert_eq!(reopened.snapshot(), clock.snapshot());
        if let Ok(next) = reopened.now() {
            prop_assert_eq!(hlc::compare(next, observed), Some(Ordering::Greater));
        }
        let foreign = HybridLogicalClock::open("device-b", || 0, Some(clock.snapshot())).expect("open");
        prop_assert_eq!(foreign.snapshot().wall_time_ms, 0);
        prop_assert_eq!(foreign.snapshot().device_id, "device-b");
    }
}
