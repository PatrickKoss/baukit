use std::collections::BTreeMap;

use baukit_core::media_grant::{MediaGrantKey, MediaGrantKeyRing, MediaGrantRequest};
use proptest::prelude::*;
use serde_json::Value;

const VECTORS: &str = include_str!("../../../../fixtures/media-grants/vectors-v1.json");
const KEY_ID: &str = "current_2026_09";
const SECRET: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY";
const NOW: u64 = 2_000_000_000;
const MAX_LIFETIME: u64 = 3_600;

fn ring() -> MediaGrantKeyRing {
    let key = MediaGrantKey::from_base64url(KEY_ID, SECRET).expect("vector key");
    MediaGrantKeyRing::new(key, None).expect("ring")
}

fn media_path() -> impl Strategy<Value = String> {
    prop::collection::vec("[A-Za-z0-9_-][A-Za-z0-9._-]{0,15}", 1..5)
        .prop_map(|segments| format!("/{}", segments.join("/")))
}

fn mutate(text: &str, index: prop::sample::Index, replacement: u8) -> Option<String> {
    let mut bytes = text.as_bytes().to_vec();
    let at = index.index(bytes.len());
    if bytes[at] == replacement {
        return None;
    }
    bytes[at] = replacement;
    String::from_utf8(bytes).ok()
}

proptest! {
    #[test]
    fn signed_grants_verify(path in media_path(), lifetime in 1..=MAX_LIFETIME) {
        let ring = ring();
        let grant = ring.sign(&path, NOW + lifetime, NOW).expect("sign");
        let query = grant.query();
        let request = MediaGrantRequest { method: "GET", path: &path, query: &query, now: NOW };
        prop_assert_eq!(ring.verify(request).expect("verify").expires(), NOW + lifetime);
    }

    #[test]
    fn single_byte_query_mutation_is_rejected(
        path in media_path(),
        lifetime in 1..=MAX_LIFETIME,
        index in any::<prop::sample::Index>(),
        replacement in any::<u8>(),
    ) {
        let ring = ring();
        let query = ring.sign(&path, NOW + lifetime, NOW).expect("sign").query();
        let Some(mutated) = mutate(&query, index, replacement) else {
            return Ok(());
        };
        let request = MediaGrantRequest { method: "GET", path: &path, query: &mutated, now: NOW };
        prop_assert!(ring.verify(request).is_err(), "accepted mutated query {mutated}");
    }

    #[test]
    fn single_byte_path_mutation_is_rejected(
        path in media_path(),
        lifetime in 1..=MAX_LIFETIME,
        index in any::<prop::sample::Index>(),
        replacement in any::<u8>(),
    ) {
        let ring = ring();
        let query = ring.sign(&path, NOW + lifetime, NOW).expect("sign").query();
        let Some(mutated) = mutate(&path, index, replacement) else {
            return Ok(());
        };
        let request = MediaGrantRequest { method: "GET", path: &mutated, query: &query, now: NOW };
        prop_assert!(ring.verify(request).is_err(), "accepted mutated path {mutated}");
    }
}

fn vectors() -> Value {
    serde_json::from_str(VECTORS).expect("vector file")
}

fn text<'a>(value: &'a Value, field: &str) -> &'a str {
    value[field]
        .as_str()
        .unwrap_or_else(|| panic!("missing {field}"))
}

fn number(value: &Value, field: &str) -> u64 {
    value[field]
        .as_u64()
        .unwrap_or_else(|| panic!("missing {field}"))
}

fn keys(vectors: &Value) -> BTreeMap<String, MediaGrantKey> {
    vectors["keys"]
        .as_array()
        .expect("keys")
        .iter()
        .map(|key| {
            let loaded =
                MediaGrantKey::from_base64url(text(key, "keyId"), text(key, "secretBase64url"))
                    .expect("vector key");
            (text(key, "name").to_owned(), loaded)
        })
        .collect()
}

#[test]
fn rust_signer_matches_shared_sign_vectors() {
    let vectors = vectors();
    let keys = keys(&vectors);
    for case in vectors["signCases"].as_array().expect("signCases") {
        let name = text(case, "name");
        let key = &keys[text(case, "key")];
        let result = key.sign(
            text(case, "path"),
            number(case, "expires"),
            number(case, "now"),
        );
        let expected = &case["expected"];
        match expected["error"].as_str() {
            Some(code) => assert_eq!(result.expect_err(name).code(), code, "{name}"),
            None => assert_eq!(
                result.expect(name).query(),
                text(expected, "query"),
                "{name}"
            ),
        }
    }
}

#[test]
fn rust_signed_grants_pass_shared_verify_vectors() {
    let vectors = vectors();
    let keys = keys(&vectors);
    for case in vectors["verifyCases"].as_array().expect("verifyCases") {
        let name = text(case, "name");
        let current = keys[text(case, "current")].clone();
        let previous = case["previous"]
            .as_str()
            .map(|previous| keys[previous].clone());
        let ring = MediaGrantKeyRing::new(current, previous).expect(name);
        let request = MediaGrantRequest {
            method: text(case, "method"),
            path: text(case, "path"),
            query: text(case, "query"),
            now: number(case, "now"),
        };
        let result = ring.verify(request);
        let expected = &case["expected"];
        match expected["error"].as_str() {
            Some(code) => assert_eq!(result.expect_err(name).code(), code, "{name}"),
            None => {
                let verified = result.expect(name);
                assert_eq!(verified.key_id(), text(expected, "keyId"), "{name}");
                assert_eq!(verified.expires(), number(expected, "expires"), "{name}");
            }
        }
    }
}
