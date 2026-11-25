#![no_main]

use std::net::SocketAddr;

use axum::http::{HeaderMap, HeaderValue};
use baukit_ratelimit::resolve_client_ip;
use libfuzzer_sys::fuzz_target;

const XFF: &str = "x-forwarded-for";
const MAX_HOPS: u8 = 8;

fn entries(headers: &HeaderMap) -> Vec<Vec<u8>> {
    headers
        .get_all(XFF)
        .iter()
        .flat_map(|value| value.as_bytes().split(|byte| *byte == b','))
        .map(<[u8]>::to_vec)
        .collect()
}

fn header_map(values: &[&[u8]]) -> Option<HeaderMap> {
    let mut headers = HeaderMap::new();
    for value in values {
        headers.append(XFF, HeaderValue::from_bytes(value).ok()?);
    }
    Some(headers)
}

// Input: hops byte, peer flag byte, then the original header values and an
// attacker prefix, separated by 0xff. The last segment is the prefix.
fuzz_target!(|data: &[u8]| {
    let [hops, peer_flag, rest @ ..] = data else {
        return;
    };
    let hops = usize::from(hops % MAX_HOPS);
    let peer = (peer_flag % 2 == 1).then(|| SocketAddr::from(([10, 0, 0, 5], 443)));
    let mut segments: Vec<&[u8]> = rest.split(|byte| *byte == 0xff).collect();
    let Some(prefix) = segments.pop() else {
        return;
    };
    let Some(original) = header_map(&segments) else {
        return;
    };
    let original_entries = entries(&original);
    if original_entries.len() < hops {
        return;
    }
    let suffix = original_entries[original_entries.len() - hops..].join(&b',');
    let mut rewritten = prefix.to_vec();
    if hops > 0 {
        rewritten.push(b',');
        rewritten.extend_from_slice(&suffix);
    }
    let Some(forged) = header_map(&[&rewritten]) else {
        return;
    };
    assert_eq!(
        resolve_client_ip(&original, peer, hops),
        resolve_client_ip(&forged, peer, hops)
    );
});
