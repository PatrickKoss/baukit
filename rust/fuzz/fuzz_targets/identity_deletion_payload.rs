#![no_main]

use baukit_erasure::internals::parse_payload;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return;
    };
    if let Ok(payload) = parse_payload(&value, "clerk") {
        assert_eq!(payload.provider_id, "clerk");
        assert!(baukit_erasure::kernel::is_well_formed_subject(
            &payload.subject
        ));
    }
});
