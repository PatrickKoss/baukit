#![no_main]

use std::sync::LazyLock;

use baukit_core::media_grant::{
    MAX_CLOCK_SKEW_SECONDS, MAX_GRANT_LIFETIME_SECONDS, MediaGrantKey, MediaGrantKeyRing,
    MediaGrantRequest,
};
use libfuzzer_sys::fuzz_target;

const CURRENT_ID: &str = "current_2026_09";
const CURRENT_SECRET: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY";
const PREVIOUS_ID: &str = "previous_2026_08";
const PREVIOUS_SECRET: &str = "cHJldmlvdXMta2V5LW1hdGVyaWFsLTMyLWJ5dGVzISE";
const PATH: &str = "/media/6ca2eeeb-b0bf-55e7-8793-1b9a52b154a0/v1/clip.mp4";
const NOW: u64 = 2_000_000_000;
const METHODS: [&str; 6] = ["GET", "HEAD", "POST", "get", "GET\n", ""];

fn key(id: &str, secret: &str) -> MediaGrantKey {
    MediaGrantKey::from_base64url(id, secret).expect("vector key")
}

static RING: LazyLock<MediaGrantKeyRing> = LazyLock::new(|| {
    MediaGrantKeyRing::new(
        key(CURRENT_ID, CURRENT_SECRET),
        Some(key(PREVIOUS_ID, PREVIOUS_SECRET)),
    )
    .expect("key ring")
});

// Layout: method selector, clock offset, then `path?query` (path defaults to a valid one).
fuzz_target!(|data: &[u8]| {
    let Some((&selector, rest)) = data.split_first() else {
        return;
    };
    let Ok(text) = std::str::from_utf8(rest) else {
        return;
    };
    let method = METHODS[usize::from(selector) % METHODS.len()];
    let now = NOW + u64::from(selector >> 4) * 300;
    let (path, query) = text.split_once('?').unwrap_or((PATH, text));
    let request = MediaGrantRequest {
        method,
        path,
        query,
        now,
    };
    let Ok(verified) = RING.verify(request) else {
        return;
    };
    assert!(matches!(method, "GET" | "HEAD"));
    assert!(now < verified.expires());
    assert!(verified.expires() - now <= MAX_GRANT_LIFETIME_SECONDS + MAX_CLOCK_SKEW_SECONDS);
    let signer = if verified.key_id() == CURRENT_ID {
        key(CURRENT_ID, CURRENT_SECRET)
    } else {
        assert_eq!(verified.key_id(), PREVIOUS_ID);
        key(PREVIOUS_ID, PREVIOUS_SECRET)
    };
    let expected = signer
        .sign(path, verified.expires(), verified.expires() - 1)
        .expect("re-sign verified grant")
        .query();
    assert_eq!(
        query, expected,
        "verified query differs from the signed form"
    );
});
