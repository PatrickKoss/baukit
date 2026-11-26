use axum::http::{HeaderMap, HeaderValue};

use super::*;

fn request_id(value: &str) -> RequestId {
    let mut headers = HeaderMap::new();
    headers.insert(
        &X_REQUEST_ID,
        HeaderValue::from_str(value).expect("header value"),
    );
    RequestId::from_headers(&headers)
}

fn is_generated(id: &RequestId) -> bool {
    Uuid::parse_str(id.as_str()).is_ok()
}

#[test]
fn safe_client_request_ids_are_kept() {
    let max = "a".repeat(MAX_REQUEST_ID_BYTES);
    for value in ["abc-123", "A.b_c-9", max.as_str()] {
        assert_eq!(request_id(value).as_str(), value);
    }
}

#[test]
fn unsafe_client_request_ids_are_replaced() {
    let too_long = "a".repeat(MAX_REQUEST_ID_BYTES + 1);
    for value in ["", "a b", "a\"b", "a/b", "id;x=1", "%41", too_long.as_str()] {
        let id = request_id(value);
        assert!(is_generated(&id), "{value:?} kept as {:?}", id.as_str());
    }
}
