#![no_main]

use baukit_core::media_grant::{MediaGrantKey, MediaGrantKeyRing, MediaGrantRequest};
use libfuzzer_sys::fuzz_target;

const KEY_ID: &str = "current_2026_09";
const SECRET: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY";
const PATH: &str = "/media/6ca2eeeb-b0bf-55e7-8793-1b9a52b154a0/v1/clip.mp4";
const NOW: u64 = 2_000_000_000;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let key = MediaGrantKey::from_base64url(KEY_ID, SECRET).expect("vector key");
    let ring = MediaGrantKeyRing::new(key, None).expect("key ring");
    let (path, query) = text.split_once('?').unwrap_or((PATH, text));
    let request = MediaGrantRequest {
        method: "GET",
        path,
        query,
        now: NOW,
    };
    if let Ok(verified) = ring.verify(request) {
        let expected = ring
            .sign(path, verified.expires(), verified.expires() - 1)
            .expect("re-sign verified grant")
            .query();
        assert_eq!(
            query, expected,
            "verified query differs from the signed form"
        );
    }
});
