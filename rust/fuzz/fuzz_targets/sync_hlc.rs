#![no_main]

use std::{cell::Cell, cmp::Ordering, rc::Rc};

use baukit_sync::hlc::{self, HybridLogicalClock, HybridLogicalClockState};
use libfuzzer_sys::fuzz_target;

const WORD_BYTES: usize = 8;
const DEVICE: &str = "device-fuzz";
const MAX_DRIFT_MS: i64 = 60_000;
const OPERATIONS: i64 = 4;
const MAX_CLOCK_STEP_MS: i64 = 3_600_000;

type Clock = HybridLogicalClock<Box<dyn FnMut() -> i64>>;

fn words(data: &[u8]) -> impl Iterator<Item = i64> + '_ {
    data.as_chunks::<WORD_BYTES>()
        .0
        .iter()
        .map(|chunk| i64::from_le_bytes(*chunk))
}

fn open(physical: &Rc<Cell<i64>>, persisted: Option<HybridLogicalClockState>) -> Option<Clock> {
    let reading = Rc::clone(physical);
    let clock: Box<dyn FnMut() -> i64> = Box::new(move || reading.get());
    HybridLogicalClock::open(DEVICE, clock, persisted)
        .ok()?
        .with_max_drift_ms(MAX_DRIFT_MS)
        .ok()
}

fuzz_target!(|data: &[u8]| {
    let mut values = words(data);
    let (Some(wall), Some(counter), Some(start)) = (values.next(), values.next(), values.next())
    else {
        return;
    };
    if let Ok(encoded) = hlc::encode(wall, counter) {
        assert_eq!(hlc::decode(encoded), Ok((wall, counter)));
    }
    let physical = Rc::new(Cell::new(start));
    let persisted = HybridLogicalClockState {
        wall_time_ms: wall,
        counter,
        device_id: DEVICE.to_owned(),
    };
    let Some(mut clock) = open(&physical, Some(persisted)) else {
        return;
    };
    let mut last: Option<i64> = None;
    while let (Some(operation), Some(argument)) = (values.next(), values.next()) {
        let issued = match operation.rem_euclid(OPERATIONS) {
            0 => clock.now(),
            1 => {
                let result = clock.observe(argument);
                if let (Ok(next), Ok((remote_wall, _))) = (result, hlc::decode(argument)) {
                    assert_eq!(hlc::compare(next, argument), Some(Ordering::Greater));
                    assert!(remote_wall <= physical.get().saturating_add(MAX_DRIFT_MS));
                }
                result
            }
            2 => {
                physical.set(physical.get().wrapping_add(argument % MAX_CLOCK_STEP_MS));
                continue;
            }
            _ => {
                let Some(reopened) = open(&physical, Some(clock.snapshot())) else {
                    return;
                };
                clock = reopened;
                continue;
            }
        };
        let Ok(next) = issued else { continue };
        if let Some(previous) = last {
            assert_eq!(hlc::compare(next, previous), Some(Ordering::Greater));
        }
        last = Some(next);
    }
});
