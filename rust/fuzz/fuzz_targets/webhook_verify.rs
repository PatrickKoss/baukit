#![no_main]

use baukit_core::webhook_signature::{sign_webhook_hmac_sha256, verify_webhook_hmac_sha256};
use libfuzzer_sys::fuzz_target;

const TIMESTAMP_BYTES: usize = 8;

struct Input<'a> {
    secret: &'a [u8],
    timestamp: i64,
    id: String,
    body: &'a [u8],
    signature: String,
}

/// Layout: secret length, secret, timestamp, then id, body and signature split on 0xff.
fn parse(data: &[u8]) -> Option<Input<'_>> {
    let (&secret_length, rest) = data.split_first()?;
    let (secret, rest) = rest.split_at_checked(usize::from(secret_length))?;
    let (timestamp, rest) = rest.split_at_checked(TIMESTAMP_BYTES)?;
    let mut parts = rest.splitn(3, |byte| *byte == 0xff);
    let id = String::from_utf8(parts.next()?.to_vec()).ok()?;
    let body = parts.next()?;
    let signature = String::from_utf8(parts.next().unwrap_or_default().to_vec()).ok()?;
    Some(Input {
        secret,
        timestamp: i64::from_le_bytes(timestamp.try_into().ok()?),
        id,
        body,
        signature,
    })
}

/// A `~` prefix keeps the first `n` canonical bytes and appends the rest, so
/// the fuzzer explores edits close to a valid signature.
fn near_canonical(canonical: &str, supplied: &str) -> String {
    let Some(rest) = supplied.strip_prefix('~') else {
        return supplied.to_owned();
    };
    let mut chars = rest.chars();
    let keep = chars
        .next()
        .map_or(0, |c| c as usize % (canonical.len() + 1));
    format!("{}{}", &canonical[..keep], chars.as_str())
}

fuzz_target!(|data: &[u8]| {
    let Some(input) = parse(data) else {
        return;
    };
    let canonical = sign_webhook_hmac_sha256(input.secret, input.timestamp, &input.id, input.body);
    assert!(verify_webhook_hmac_sha256(
        [input.secret],
        input.timestamp,
        &input.id,
        input.body,
        &canonical,
    ));
    let signature = near_canonical(&canonical, &input.signature);
    let accepted = verify_webhook_hmac_sha256(
        [input.secret],
        input.timestamp,
        &input.id,
        input.body,
        &signature,
    );
    assert_eq!(accepted, signature == canonical);
});
