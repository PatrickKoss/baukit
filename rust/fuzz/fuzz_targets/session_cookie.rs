#![no_main]

//! Z-09: raw `Cookie` headers into the BFF session cookie reader.
//!
//! The reader must pick the first pair whose name matches exactly, and a
//! parsed credential must re-encode to the bytes it was read from.

use baukit_auth::browser_session::fuzzing::{parse_session_credential, read_session_cookie};
use libfuzzer_sys::fuzz_target;

const NAME: &str = "acme-session";

fn reference(lines: &[&str]) -> Option<String> {
    let value = lines
        .iter()
        .flat_map(|line| line.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == NAME)
        .map(|(_, value)| value)?;
    parse_session_credential(value)
}

fuzz_target!(|data: &[u8]| {
    let lines = data.split(|byte| *byte == b'\n').collect::<Vec<_>>();
    let read = read_session_cookie(NAME, &lines);
    let text_lines = lines
        .iter()
        .map(|line| std::str::from_utf8(line).ok())
        .collect::<Option<Vec<_>>>();
    let valid_headers = lines.iter().all(|line| {
        axum::http::HeaderValue::from_bytes(line).is_ok_and(|value| value.to_str().is_ok())
    });
    if let (Some(text_lines), true) = (text_lines, valid_headers) {
        assert_eq!(read, reference(&text_lines));
    }
    if let Some(encoded) = read {
        assert!(lines.iter().any(|line| {
            line.windows(encoded.len())
                .any(|window| window == encoded.as_bytes())
        }));
        assert_eq!(parse_session_credential(&encoded), Some(encoded));
    }
    if let Ok(text) = std::str::from_utf8(data)
        && let Some(encoded) = parse_session_credential(text)
    {
        assert_eq!(encoded, text);
    }
});
