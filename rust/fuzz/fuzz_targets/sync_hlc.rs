#![no_main]

use std::cmp::Ordering;

use baukit_sync::hlc::{self, HybridLogicalClock, HybridLogicalClockState};
use libfuzzer_sys::fuzz_target;

const WORD_BYTES: usize = 8;

fn words(data: &[u8]) -> impl Iterator<Item = i64> + '_ {
    data.as_chunks::<WORD_BYTES>()
        .0
        .iter()
        .map(|chunk| i64::from_le_bytes(*chunk))
}

fuzz_target!(|data: &[u8]| {
    let mut values = words(data);
    let (Some(wall), Some(counter), Some(physical)) = (values.next(), values.next(), values.next())
    else {
        return;
    };
    if let Ok(encoded) = hlc::encode(wall, counter) {
        assert_eq!(hlc::decode(encoded), Ok((wall, counter)));
    }
    if let Ok((decoded_wall, decoded_counter)) = hlc::decode(wall) {
        assert_eq!(hlc::encode(decoded_wall, decoded_counter), Ok(wall));
    }
    let persisted = HybridLogicalClockState {
        wall_time_ms: wall,
        counter,
        device_id: "device-fuzz".to_owned(),
    };
    let Ok(mut clock) = HybridLogicalClock::open("device-fuzz", move || physical, Some(persisted))
    else {
        return;
    };
    let mut last = None;
    for remote in values {
        let Ok(next) = clock.observe(remote) else {
            continue;
        };
        if hlc::decode(remote).is_ok() {
            assert_eq!(hlc::compare(next, remote), Some(Ordering::Greater));
        }
        if let Some(previous) = last {
            assert_eq!(hlc::compare(next, previous), Some(Ordering::Greater));
        }
        last = Some(next);
    }
});
