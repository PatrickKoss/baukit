use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, HeaderValue, Method, Request, StatusCode},
    routing::post,
};
use serde_json::Value;
use tower::ServiceExt;

use super::{
    IDEMPOTENCY_KEY, IdempotencyError, IdempotencyKeyRule, InvalidIdempotencyKey,
    InvalidIdempotencyKeyRule, MAX_IDEMPOTENCY_KEY_BYTES,
};
use crate::{ApiError, HttpOptions, finalize};

const MAX_BODY_BYTES: usize = 64 * 1024;
const RULE: IdempotencyKeyRule = IdempotencyKeyRule::new(8, 64);
const UUID_KEY: &str = "0b3f5d1e-8c2a-4f6b-9d7e-1a2b3c4d5e6f";

fn headers(values: &[&[u8]]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for value in values {
        headers.append(
            IDEMPOTENCY_KEY,
            HeaderValue::from_bytes(value).expect("header value bytes"),
        );
    }
    headers
}

fn required(values: &[&[u8]]) -> Result<String, IdempotencyError> {
    let headers = headers(values);
    RULE.required(&headers).map(|key| key.as_str().to_owned())
}

#[test]
fn accepts_keys_inside_the_bounds() {
    for key in [
        UUID_KEY,
        "abcdefgh",
        "\"quoted\"",
        "!#$%&'()*+,./:;<=>?@[]^_`{|}~",
        &"k".repeat(64),
    ] {
        assert_eq!(required(&[key.as_bytes()]), Ok(key.to_owned()), "{key}");
    }
}

#[test]
fn rejects_keys_outside_the_grammar() {
    for (value, reason) in [
        (&b""[..], InvalidIdempotencyKey::Empty),
        (b"short", InvalidIdempotencyKey::TooShort),
        ("k".repeat(65).as_bytes(), InvalidIdempotencyKey::TooLong),
        (b"has space", InvalidIdempotencyKey::InvalidCharacter),
        (b"has\ttab1", InvalidIdempotencyKey::InvalidCharacter),
        (
            "umlaut-\u{e4}-key".as_bytes(),
            InvalidIdempotencyKey::InvalidCharacter,
        ),
    ] {
        assert_eq!(
            required(&[value]),
            Err(IdempotencyError::Invalid(reason)),
            "{reason:?}"
        );
    }
}

#[test]
fn a_repeated_header_is_rejected_even_when_both_values_match() {
    let key = UUID_KEY.as_bytes();
    assert_eq!(
        required(&[key, key]),
        Err(IdempotencyError::Invalid(
            InvalidIdempotencyKey::RepeatedHeader
        ))
    );
}

#[test]
fn a_missing_header_is_required_or_absent() {
    let empty = HeaderMap::new();
    assert_eq!(RULE.required(&empty), Err(IdempotencyError::Required));
    assert_eq!(RULE.optional(&empty), Ok(None));
}

#[test]
fn bounds_are_validated() {
    assert_eq!(
        IdempotencyKeyRule::try_new(1, MAX_IDEMPOTENCY_KEY_BYTES).map(|rule| rule.max_bytes()),
        Ok(MAX_IDEMPOTENCY_KEY_BYTES)
    );
    for (min, max) in [(0, 8), (9, 8), (1, MAX_IDEMPOTENCY_KEY_BYTES + 1)] {
        assert_eq!(
            IdempotencyKeyRule::try_new(min, max),
            Err(InvalidIdempotencyKeyRule)
        );
    }
}

#[test]
#[should_panic(expected = "invalid idempotency key bounds")]
fn invalid_fixed_bounds_panic() {
    let _ = IdempotencyKeyRule::new(0, 8);
}

#[test]
fn debug_output_hides_the_key() {
    let headers = headers(&[UUID_KEY.as_bytes()]);
    let key = RULE.required(&headers).expect("valid key");
    assert_eq!(format!("{key:?}"), "IdempotencyKey(..)");
}

#[test]
fn every_invalid_reason_is_distinct_snake_case() {
    let reasons = [
        InvalidIdempotencyKey::RepeatedHeader,
        InvalidIdempotencyKey::Empty,
        InvalidIdempotencyKey::InvalidCharacter,
        InvalidIdempotencyKey::TooShort,
        InvalidIdempotencyKey::TooLong,
    ]
    .map(InvalidIdempotencyKey::reason);
    let unique: std::collections::BTreeSet<_> = reasons.iter().collect();
    assert_eq!(unique.len(), reasons.len());
    assert!(
        reasons
            .iter()
            .all(|reason| crate::error::is_valid_error_code(reason))
    );
}

#[test]
fn storage_outcomes_map_to_conflicts() {
    for (error, code) in [
        (IdempotencyError::Reused, "idempotency_key_reused"),
        (IdempotencyError::InProgress, "idempotency_key_in_progress"),
    ] {
        assert_eq!(error.status(), StatusCode::CONFLICT);
        assert_eq!(error.code(), code);
    }
}

async fn create_item(headers: HeaderMap) -> Result<StatusCode, ApiError> {
    RULE.required(&headers)?;
    Ok(StatusCode::CREATED)
}

async fn send(values: &[&str]) -> (StatusCode, Value) {
    let app = finalize(
        Router::new().route("/items", post(create_item)),
        HttpOptions::default(),
    );
    let mut request = Request::builder().method(Method::POST).uri("/items");
    for value in values {
        request = request.header(IDEMPOTENCY_KEY, *value);
    }
    let response = app
        .oneshot(request.body(Body::empty()).expect("request"))
        .await
        .expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), MAX_BODY_BYTES)
        .await
        .expect("body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn a_valid_key_reaches_the_handler() {
    let (status, _) = send(&[UUID_KEY]).await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn a_missing_key_is_400_idempotency_key_required() {
    let (status, body) = send(&[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "idempotency_key_required");
    let details = body["error"].get("details").and_then(Value::as_object);
    assert!(details.is_none_or(serde_json::Map::is_empty), "{body}");
}

#[tokio::test]
async fn a_malformed_key_is_400_with_a_reason() {
    for (values, reason) in [
        (&["short"][..], "too_short"),
        (&[UUID_KEY, UUID_KEY][..], "repeated_header"),
    ] {
        let (status, body) = send(values).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{reason}");
        assert_eq!(body["error"]["code"], "invalid_idempotency_key", "{reason}");
        assert_eq!(body["error"]["details"]["reason"], reason);
    }
}
