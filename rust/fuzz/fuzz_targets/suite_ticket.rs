#![no_main]

use baukit_suite::platform::{TicketJwks, verify_ticket};
use libfuzzer_sys::fuzz_target;
use std::sync::LazyLock;

const FIXTURE: &str = include_str!("../../../fixtures/entitlement-tickets/v1/vectors.json");

struct Setup {
    jwks: TicketJwks,
    issuer: String,
    audience: String,
    now: i64,
}

static SETUP: LazyLock<Setup> = LazyLock::new(|| {
    let fixture: serde_json::Value = serde_json::from_str(FIXTURE).expect("fixture");
    let vector = &fixture["vectors"][0];
    Setup {
        jwks: serde_json::from_value(fixture["jwks"].clone()).expect("jwks"),
        issuer: fixture["issuer"].as_str().expect("issuer").to_owned(),
        audience: vector["audience"].as_str().expect("audience").to_owned(),
        now: vector["now"].as_i64().expect("now"),
    }
});

fuzz_target!(|data: &[u8]| {
    let Ok(ticket) = std::str::from_utf8(data) else {
        return;
    };
    let setup = &*SETUP;
    let Ok(claims) = verify_ticket(ticket, &setup.jwks, &setup.issuer, &setup.audience, setup.now)
    else {
        return;
    };
    let reencoded = serde_json::to_vec(&claims).expect("serialize claims");
    let reparsed: baukit_suite::platform::TicketClaims =
        serde_json::from_slice(&reencoded).expect("claims round trip");
    assert_eq!(reparsed, claims);
    reparsed
        .verify_claims(&setup.issuer, &setup.audience, setup.now)
        .expect("verified claims stay valid");
});
