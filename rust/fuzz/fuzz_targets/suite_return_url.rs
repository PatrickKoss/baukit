#![no_main]

use baukit_suite::domain::{
    EncryptedPayload, LinkCallbackQuery, LinkRequest, relay_callback, validate_return_url,
};
use jiff::Timestamp;
use libfuzzer_sys::fuzz_target;
use url::Url;
use uuid::Uuid;

const ORIGIN: &str = "https://alpha.example";
const SCHEME: &str = "alpha";

fuzz_target!(|data: &[u8]| {
    let Ok(value) = std::str::from_utf8(data) else {
        return;
    };
    let allow_dev = data.first().is_some_and(|byte| byte & 1 == 1);
    if validate_return_url(value, ORIGIN, SCHEME, allow_dev).is_err() {
        return;
    }
    let now = Timestamp::UNIX_EPOCH;
    let request = LinkRequest {
        id: Uuid::nil(),
        user_id: Uuid::nil(),
        peer_app: "beta".into(),
        state_hash: [0; 32],
        verifier: EncryptedPayload {
            ciphertext: Vec::new(),
            nonce: Vec::new(),
            key_version: 1,
        },
        exchange: None,
        client_state_nonce: "nonce".into(),
        return_url: value.into(),
        link_id: None,
        expires_at: now,
        consumed_at: None,
        created_at: now,
    };
    let query = LinkCallbackQuery {
        state: None,
        from: None,
        code: None,
        error: None,
    };
    let redirect = relay_callback(Some(&request), &query, now).expect("valid return relays");
    let location = Url::parse(&redirect.location).expect("location parses");
    let web = location.scheme() == "https"
        && location.origin() == Url::parse(ORIGIN).expect("origin").origin()
        && location.path() == "/suite/linked";
    let native = (location.scheme() == SCHEME || allow_dev && location.scheme() == "alpha-dev")
        && location.host_str() == Some("suite")
        && location.path() == "/linked";
    assert!(web || native, "{value} relayed to {location}");
    assert!(location.username().is_empty() && location.password().is_none());
});
