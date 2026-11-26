#![no_main]

use baukit_suite::platform::{PlatformEventDecoder, PlatformEventKind};
use jiff::Timestamp;
use libfuzzer_sys::fuzz_target;

const SOURCE: &str = "schluesselbund";
const NOW: &str = "2026-10-18T12:00:00Z";

fuzz_target!(|data: &[u8]| {
    let decoder = PlatformEventDecoder::new(SOURCE.into(), None).expect("decoder");
    let now: Timestamp = NOW.parse().expect("fixed timestamp");
    let Ok(event) = decoder.decode(data, now) else {
        return;
    };
    let envelope = event.envelope();
    assert_eq!(envelope.source_app, SOURCE);
    if !matches!(event.kind(), PlatformEventKind::Unknown) {
        let owner_named = envelope
            .payload
            .values()
            .any(|value| value.as_str() == Some(envelope.user_id.as_str()));
        assert!(owner_named, "a known event must name the envelope user");
    }
});
