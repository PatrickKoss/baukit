#![no_main]

use baukit_erasure::internals::user_url;
use libfuzzer_sys::fuzz_target;
use url::Url;

const BASES: [&str; 3] = [
    "https://api.example.com",
    "https://api.example.com/v1/",
    "https://idp.example.com/auth",
];

fuzz_target!(|input: (u8, &str)| {
    let (choice, subject) = input;
    let Ok(base) = Url::parse(BASES[usize::from(choice) % BASES.len()]) else {
        return;
    };
    let parts = ["admin", "users"];
    let Ok(url) = user_url(&base, &parts, subject) else {
        return;
    };
    assert_eq!(url.host_str(), base.host_str());
    assert!(url.query().is_none() && url.fragment().is_none());
    let base_segments = base
        .path_segments()
        .into_iter()
        .flatten()
        .filter(|segment| !segment.is_empty())
        .count();
    let segments = url
        .path_segments()
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    assert_eq!(segments.len(), base_segments + parts.len() + 1);
    assert_eq!(
        &segments[base_segments..base_segments + parts.len()],
        &parts
    );
    let last = segments.last().copied().unwrap_or_default();
    let decoded = percent_encoding::percent_decode_str(last).decode_utf8_lossy();
    assert_eq!(decoded, subject);
});
