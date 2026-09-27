use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header},
    routing::put,
};
use serde::Deserialize;
use serde_json::Value;
use tower::ServiceExt;

use super::{
    InvalidEtagPrefix, InvalidIfMatch, PreconditionError, Revision, RevisionEtag,
    RevisionOutOfRange, ensure_current_revision,
};
use crate::{ApiError, HttpOptions, finalize};

const VECTORS: &str = include_str!("../../../../../fixtures/etag-preconditions/vectors-v1.json");
const MAX_BODY_BYTES: usize = 64 * 1024;
const ITEM_TAG: RevisionEtag<'static> = RevisionEtag::new("rev-");
const STORED_REVISION: u32 = 7;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Vectors {
    version: u32,
    parse_cases: Vec<ParseCase>,
    format_cases: Vec<FormatCase>,
    prefix_cases: Vec<PrefixCase>,
    stale_cases: Vec<StaleCase>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParseCase {
    name: String,
    prefix: String,
    requirement: Requirement,
    if_match: Vec<String>,
    outcome: Outcome,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Requirement {
    Required,
    Optional,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(untagged)]
enum Outcome {
    Rejected {
        status: u16,
        code: String,
        reason: Option<String>,
        #[serde(rename = "currentRevision")]
        current_revision: Option<u64>,
    },
    Accepted {
        revision: Option<u64>,
    },
}

#[derive(Deserialize)]
struct FormatCase {
    prefix: String,
    revision: u64,
    etag: String,
}

#[derive(Deserialize)]
struct PrefixCase {
    prefix: String,
    valid: bool,
}

#[derive(Deserialize)]
struct StaleCase {
    expected: u64,
    current: u64,
    outcome: Option<Outcome>,
}

fn vectors() -> Vectors {
    let vectors: Vectors = serde_json::from_str(VECTORS).expect("valid vectors");
    assert_eq!(vectors.version, 1);
    vectors
}

fn revision(value: u64) -> Revision {
    Revision::try_from(value).expect("revision in range")
}

fn rejected(error: PreconditionError) -> Outcome {
    let (reason, current_revision) = match error {
        PreconditionError::Required => (None, None),
        PreconditionError::Invalid(invalid) => (Some(invalid.reason().to_owned()), None),
        PreconditionError::Stale { current } => (None, current.map(Revision::get)),
    };
    Outcome::Rejected {
        status: error.status().as_u16(),
        code: error.code().to_owned(),
        reason,
        current_revision,
    }
}

fn parse_outcome(case: &ParseCase) -> Outcome {
    let tag = RevisionEtag::try_new(&case.prefix).expect("valid prefix");
    let mut headers = HeaderMap::new();
    for value in &case.if_match {
        headers.append(
            header::IF_MATCH,
            HeaderValue::from_bytes(value.as_bytes()).expect("header value bytes"),
        );
    }
    let result = match case.requirement {
        Requirement::Required => tag.required_if_match(&headers).map(Some),
        Requirement::Optional => tag.optional_if_match(&headers),
    };
    result.map_or_else(rejected, |revision| Outcome::Accepted {
        revision: revision.map(Revision::get),
    })
}

#[test]
fn parse_vectors_pass() {
    for case in vectors().parse_cases {
        assert_eq!(parse_outcome(&case), case.outcome, "{}", case.name);
    }
}

#[test]
fn format_vectors_pass_and_round_trip() {
    for case in vectors().format_cases {
        let tag = RevisionEtag::try_new(&case.prefix).expect("valid prefix");
        let value = tag.header_value(revision(case.revision));
        assert_eq!(value, case.etag.as_str());
        assert_eq!(tag.format(revision(case.revision)), case.etag);
        assert_eq!(tag.parse(&value), Ok(revision(case.revision)));
    }
}

#[test]
fn prefix_vectors_pass() {
    for case in vectors().prefix_cases {
        assert_eq!(
            RevisionEtag::try_new(&case.prefix).is_ok(),
            case.valid,
            "{:?}",
            case.prefix
        );
    }
}

#[test]
fn stale_vectors_pass() {
    for case in vectors().stale_cases {
        let outcome = ensure_current_revision(revision(case.expected), revision(case.current))
            .err()
            .map(rejected);
        assert_eq!(outcome, case.outcome);
    }
}

#[test]
fn revision_range_matches_bigint() {
    assert_eq!(Revision::MAX.to_i64(), i64::MAX);
    assert_eq!(Revision::try_from(i64::MAX), Ok(Revision::MAX));
    assert_eq!(Revision::try_from(-1_i64), Err(RevisionOutOfRange));
    assert_eq!(
        Revision::try_from(Revision::MAX.get() + 1),
        Err(RevisionOutOfRange)
    );
    assert_eq!(Revision::from(0_u32).get(), 0);
}

#[test]
fn invalid_runtime_prefix_is_a_typed_error() {
    assert_eq!(RevisionEtag::try_new("rev "), Err(InvalidEtagPrefix));
}

#[test]
#[should_panic(expected = "invalid revision ETag prefix")]
fn invalid_fixed_prefix_panics() {
    let _ = RevisionEtag::new("rev\"");
}

#[test]
fn every_invalid_reason_is_distinct_snake_case() {
    let reasons = [
        InvalidIfMatch::RepeatedHeader,
        InvalidIfMatch::Empty,
        InvalidIfMatch::NonAscii,
        InvalidIfMatch::Wildcard,
        InvalidIfMatch::WeakValidator,
        InvalidIfMatch::List,
        InvalidIfMatch::Malformed,
        InvalidIfMatch::PrefixMismatch,
        InvalidIfMatch::InvalidRevision,
        InvalidIfMatch::RevisionOutOfRange,
    ]
    .map(InvalidIfMatch::reason);
    let unique: std::collections::BTreeSet<_> = reasons.iter().collect();
    assert_eq!(unique.len(), reasons.len());
    assert!(
        reasons
            .iter()
            .all(|reason| crate::error::is_valid_error_code(reason))
    );
}

async fn update_item(headers: HeaderMap) -> Result<(HeaderMap, StatusCode), ApiError> {
    let expected = ITEM_TAG.required_if_match(&headers)?;
    let stored = Revision::from(STORED_REVISION);
    ensure_current_revision(expected, stored)?;
    let next = Revision::from(STORED_REVISION + 1);
    let mut response = HeaderMap::new();
    response.insert(header::ETAG, ITEM_TAG.header_value(next));
    Ok((response, StatusCode::OK))
}

async fn send(if_match: &[&str]) -> (StatusCode, HeaderMap, Value) {
    let app = finalize(
        Router::new().route("/items/1", put(update_item)),
        HttpOptions::default(),
    );
    let mut request = Request::builder().method(Method::PUT).uri("/items/1");
    for value in if_match {
        request = request.header(header::IF_MATCH, *value);
    }
    let response = app
        .oneshot(request.body(Body::empty()).expect("request"))
        .await
        .expect("response");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), MAX_BODY_BYTES)
        .await
        .expect("body");
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, headers, body)
}

#[tokio::test]
async fn current_revision_writes_and_returns_the_next_etag() {
    let (status, headers, _) = send(&["\"rev-7\""]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::ETAG], "\"rev-8\"");
}

#[tokio::test]
async fn missing_if_match_is_428() {
    let (status, headers, body) = send(&[]).await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED);
    assert_eq!(body["error"]["code"], "precondition_required");
    assert!(headers.contains_key(crate::X_REQUEST_ID));
}

#[tokio::test]
async fn stale_if_match_is_412_with_the_current_revision() {
    let (status, _, body) = send(&["\"rev-6\""]).await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED);
    assert_eq!(body["error"]["code"], "precondition_failed");
    assert_eq!(body["error"]["details"]["currentRevision"], 7);
}

#[tokio::test]
async fn malformed_if_match_is_400_with_a_reason() {
    for (values, reason) in [
        (&["W/\"rev-7\""][..], "weak_validator"),
        (&["*"][..], "wildcard"),
        (&["\"rev-7\", \"rev-8\""][..], "list"),
        (&["\"rev-7\"", "\"rev-7\""][..], "repeated_header"),
        (&["\"item-7\""][..], "prefix_mismatch"),
    ] {
        let (status, _, body) = send(values).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{reason}");
        assert_eq!(body["error"]["code"], "invalid_if_match", "{reason}");
        assert_eq!(body["error"]["details"]["reason"], reason);
    }
}
