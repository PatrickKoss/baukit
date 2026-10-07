use super::*;
use baukit_core::webhook_signature::{
    sign_webhook_hmac_sha256, verify_webhook_hmac_sha256, webhook_signing_input,
};
use serde_json::json;
use std::{collections::BTreeSet, fs, path::PathBuf};
const SUITE_APP_ID: &str = "alpha";
const SUITE_PEERS_JSON: &str = include_str!("../../../../../fixtures/suite-events/v1/peers.json");
fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/suite-events/v1")
}

fn fixture(name: &str) -> Value {
    serde_json::from_slice(&fs::read(fixture_root().join(name)).expect("valid fixture"))
        .expect("valid fixture")
}

fn protocol() -> Value {
    fixture("link-protocol.json")
}

fn test_link() -> SuiteLink {
    let now = "2026-10-06T09:00:00Z".parse().expect("valid fixture");
    SuiteLink {
        id: "0199ba71-6000-7000-8000-000000000001"
            .parse()
            .expect("valid fixture"),
        user_id: "0199ba71-6000-7000-8000-000000000002"
            .parse()
            .expect("valid fixture"),
        peer_app: "beta".into(),
        role: LinkRole::Initiator,
        remote_link_id: "0199ba71-6000-7000-8000-000000000003"
            .parse()
            .expect("valid fixture"),
        remote_subject: "sender-subject".into(),
        remote_display_name: None,
        suite_subject: None,
        status: LinkStatus::Active,
        secret: EncryptedPayload {
            ciphertext: vec![1],
            nonce: vec![2],
            key_version: 1,
        },
        sends: Vec::new(),
        receives: vec!["beta.activity.completed".into()],
        share_xp: true,
        reward_mode: RewardMode::Native,
        delivery_health: DeliveryHealth::Healthy,
        consecutive_failures: 0,
        last_delivery_at: None,
        last_failure_at: None,
        last_failure_code: None,
        last_received_at: None,
        created_at: now,
        updated_at: now,
        revoked_at: None,
    }
}

fn protocol_request(value: &Value) -> Option<LinkRequest> {
    (!value.is_null()).then(|| LinkRequest {
        id: value["id"]
            .as_str()
            .expect("valid fixture")
            .parse()
            .expect("valid fixture"),
        user_id: value["userId"]
            .as_str()
            .expect("valid fixture")
            .parse()
            .expect("valid fixture"),
        peer_app: value["peerApp"].as_str().expect("valid fixture").into(),
        state_hash: Sha256::digest(value["state"].as_str().expect("valid fixture").as_bytes())
            .into(),
        verifier: test_link().secret,
        exchange: None,
        client_state_nonce: value["clientStateNonce"]
            .as_str()
            .expect("valid fixture")
            .into(),
        return_url: value["returnUrl"].as_str().expect("valid fixture").into(),
        link_id: value["linkId"]
            .as_str()
            .map(|id| id.parse().expect("valid fixture")),
        expires_at: value["expiresAt"]
            .as_str()
            .expect("valid fixture")
            .parse()
            .expect("valid fixture"),
        consumed_at: value["consumedAt"]
            .as_str()
            .map(|time| time.parse().expect("valid fixture")),
        created_at: "2026-10-06T08:15:00Z".parse().expect("valid fixture"),
    })
}

fn protocol_code(value: &Value) -> Option<LinkCode> {
    (!value.is_null()).then(|| LinkCode {
        code_hash: Sha256::digest(value["code"].as_str().expect("valid fixture").as_bytes()).into(),
        user_id: value["userId"]
            .as_str()
            .expect("valid fixture")
            .parse()
            .expect("valid fixture"),
        peer_app: value["peerApp"].as_str().expect("valid fixture").into(),
        code_challenge: value["codeChallenge"]
            .as_str()
            .expect("valid fixture")
            .into(),
        suite_subject: value["suiteSubject"].as_str().map(str::to_owned),
        auto_approved: value["autoApproved"].as_bool().expect("valid fixture"),
        link_id: value["linkId"]
            .as_str()
            .map(|id| id.parse().expect("valid fixture")),
        expires_at: value["expiresAt"]
            .as_str()
            .expect("valid fixture")
            .parse()
            .expect("valid fixture"),
        consumed_at: value["consumedAt"]
            .as_str()
            .map(|time| time.parse().expect("valid fixture")),
        created_at: "2026-10-06T08:15:00Z".parse().expect("valid fixture"),
    })
}

fn protocol_link(value: &Value) -> Option<SuiteLink> {
    (!value.is_null()).then(|| SuiteLink {
        id: value["id"]
            .as_str()
            .expect("valid fixture")
            .parse()
            .expect("valid fixture"),
        user_id: value["userId"]
            .as_str()
            .expect("valid fixture")
            .parse()
            .expect("valid fixture"),
        peer_app: value["peerApp"].as_str().expect("valid fixture").into(),
        remote_link_id: value["remoteLinkId"]
            .as_str()
            .expect("valid fixture")
            .parse()
            .expect("valid fixture"),
        role: LinkRole::Authorizer,
        sends: vec![
            "beta.activity.completed".into(),
            "beta.activity.completed".into(),
            "beta.activity.completed".into(),
        ],
        receives: vec!["alpha.activity.completed".into()],
        suite_subject: Some("suite-local|user-17".into()),
        ..test_link()
    })
}

fn protocol_authorizer_registry() -> PeerRegistry {
    PeerRegistry::new(
        "beta",
        SUITE_PEERS_JSON,
        PeerRegistrySettings {
            public_api_url: Some("https://beta.example/api/v1".into()),
            public_web_url: Some("https://beta.example".into()),
            peers: BTreeMap::from([(
                "alpha".into(),
                PeerUrls {
                    api_url: Some("https://alpha.example/api/v1".into()),
                    web_url: Some("https://alpha.example".into()),
                },
            )]),
            allow_loopback: false,
        },
    )
    .expect("valid fixture")
}

fn decode_hex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0);
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).expect("valid fixture"))
        .collect()
}

#[test]
fn event_id_vectors_and_envelope_identity_are_deterministic() {
    let vectors = fixture("event-ids.json");
    assert_eq!(vectors["namespace"], SUITE_EVENT_NAMESPACE.to_string());
    assert_eq!(
        vectors["vectors"].as_array().expect("valid fixture").len(),
        28
    );
    for vector in vectors["vectors"].as_array().expect("valid fixture") {
        let event = SuiteEvent {
            event_type: vector["type"].as_str().expect("valid fixture").into(),
            natural_key: vector["naturalKey"].as_str().expect("valid fixture").into(),
            occurred_at: "2026-10-06T08:15:00Z".parse().expect("valid fixture"),
            payload: Map::new(),
        };
        assert_eq!(event.event_id().to_string(), vector["eventId"]);
        assert_eq!(event.event_id().get_version_num(), 5);
        let envelope = event.envelope("oidc-sub", "alpha");
        assert_eq!(envelope.user_id, "oidc-sub");
        assert_eq!(envelope.source_app, "alpha");
        assert_eq!(envelope.schema_version, 1);
        let wire = serde_json::to_value(envelope).expect("valid fixture");
        assert_eq!(wire["occurredAt"], "2026-10-06T08:15:00Z");
        assert_eq!(wire["eventId"], vector["eventId"]);
    }
}

#[test]
fn pkce_hints_subject_construction_and_subject_matching_use_the_vectors() {
    let protocol = protocol();
    for vector in protocol["pkce"].as_array().expect("valid fixture") {
        assert_eq!(
            pkce_challenge(vector["verifier"].as_str().expect("valid fixture"))
                .expect("valid fixture"),
            vector["challenge"]
        );
    }
    for invalid in [
        "",
        &"A".repeat(42),
        &"A".repeat(129),
        &" ".repeat(43),
        &"ä".repeat(43),
    ] {
        assert_eq!(pkce_challenge(invalid), Err(SuiteDataError::PkceInvalid));
    }
    for vector in protocol["hints"].as_array().expect("valid fixture") {
        assert_eq!(
            hint_for(vector["subject"].as_str().expect("valid fixture")),
            vector["hint"]
        );
    }
    let claims = json!({"suite_sub":"user-17","sub":"product-user"})
        .as_object()
        .expect("valid fixture")
        .clone();
    assert_eq!(
        suite_subject(Some("suite-local"), Some("suite_sub"), &claims).expect("valid fixture"),
        Some("suite-local|user-17".into())
    );
    assert_eq!(
        suite_subject(None, Some("suite_sub"), &claims).expect("valid fixture"),
        None
    );
    assert_eq!(
        suite_subject(Some("suite-local"), None, &claims).expect("valid fixture"),
        None
    );
    assert_eq!(
        suite_subject(Some("suite-local"), Some("missing"), &claims).expect("valid fixture"),
        None
    );
    assert_eq!(
        suite_subject(Some("bad|domain"), Some("sub"), &claims),
        Err(SuiteDataError::SubjectInvalid)
    );
    for vector in protocol["subjects"].as_array().expect("valid fixture") {
        let actual =
            shared_suite_subject(vector["initiator"].as_str(), vector["authorizer"].as_str());
        if vector.get("error").is_some() {
            assert_eq!(actual, Err(SuiteDataError::AccountMismatch));
        } else {
            assert_eq!(
                actual.expect("valid fixture"),
                vector["expected"].as_str().map(str::to_owned)
            );
        }
    }
    assert_eq!(
        shared_suite_subject(None, Some("suite-local|user-17")).expect("valid fixture"),
        None
    );
    assert_eq!(
        shared_suite_subject(Some("bad"), Some("bad")),
        Err(SuiteDataError::SubjectInvalid)
    );
}

#[test]
fn return_url_vectors_match_web_and_native_endpoints() {
    let protocol = protocol();
    let cases = &protocol["returnUrls"];
    for vector in cases["vectors"].as_array().expect("valid fixture") {
        assert_eq!(
            validate_return_url(
                vector["url"].as_str().expect("valid fixture"),
                cases["webOrigin"].as_str().expect("valid fixture"),
                cases["scheme"].as_str().expect("valid fixture")
            )
            .is_ok(),
            vector["expected"].as_bool().expect("valid fixture"),
            "{}",
            vector["url"]
        );
    }
    for url in [
        "//alpha.example/suite/linked",
        "javascript:alert(1)",
        "https://alpha.example/suite/linked/",
        "alpha://suite:123/linked",
        "alpha://user@suite/linked",
    ] {
        assert_eq!(
            validate_return_url(url, "https://alpha.example", "alpha"),
            Err(SuiteDataError::ReturnUrlInvalid)
        );
    }
}

#[test]
fn every_delivery_status_class_and_transport_failure_maps_to_the_required_action() {
    for status in 100..=599 {
        let actual = map_delivery_result(DeliveryResult::Http {
            status,
            retry_after: None,
        });
        let expected = match status {
            200..=299 => DeliveryAction::Delivered,
            401 | 403 => DeliveryAction::Unauthorized,
            410 => DeliveryAction::Revoked,
            408 | 425 | 429 | 500..=599 => DeliveryAction::Retry { after: None },
            _ => DeliveryAction::Rejected,
        };
        assert_eq!(actual, expected, "{status}");
    }
    for result in [
        DeliveryResult::Timeout,
        DeliveryResult::Transport,
        DeliveryResult::Dns,
    ] {
        assert_eq!(
            map_delivery_result(result),
            DeliveryAction::Retry { after: None }
        );
    }
    assert_eq!(
        map_delivery_result(DeliveryResult::Http {
            status: 429,
            retry_after: Some(Duration::from_secs(900))
        }),
        DeliveryAction::Retry {
            after: Some(Duration::from_secs(300))
        }
    );
    assert_eq!(
        map_delivery_result(DeliveryResult::Http {
            status: 503,
            retry_after: Some(Duration::from_secs(17))
        }),
        DeliveryAction::Retry {
            after: Some(Duration::from_secs(17))
        }
    );
    assert_eq!(
        DeliveryAction::Unauthorized.permanent_code(),
        Some("suite_unauthorized")
    );
    assert_eq!(
        DeliveryAction::Revoked.permanent_code(),
        Some("suite_link_revoked")
    );
    assert_eq!(
        DeliveryAction::Rejected.permanent_code(),
        Some("suite_rejected")
    );
    assert_eq!(DeliveryAction::Delivered.outcome(), Some("delivered"));
}

#[test]
fn successful_delivery_resets_failures_and_attention_and_revocation_are_preserved() {
    let state = DeliveryState {
        status: LinkStatus::Active,
        health: DeliveryHealth::Disabled,
        consecutive_failures: 20,
    };
    assert_eq!(after_delivery(state, DeliveryAction::Delivered), state);
    let reset = after_delivery(
        DeliveryState {
            health: DeliveryHealth::Degraded,
            consecutive_failures: 19,
            ..state
        },
        DeliveryAction::Delivered,
    );
    assert_eq!(reset.consecutive_failures, 0);
    assert_eq!(reset.health, DeliveryHealth::Healthy);
    let attention = after_delivery(reset, DeliveryAction::Unauthorized);
    assert_eq!(attention.status, LinkStatus::NeedsAttention);
    assert_eq!(attention.health, DeliveryHealth::NeedsAttention);
    assert_eq!(
        after_delivery(reset, DeliveryAction::Revoked).status,
        LinkStatus::Revoked
    );
    assert_eq!(
        after_delivery(reset, DeliveryAction::Retry { after: None }),
        reset
    );
}

#[test]
fn reward_modes_fallback_and_replay_and_test_exclusions_are_pinned() {
    assert_eq!(RewardMode::default(), RewardMode::Native);
    let cases = [
        (RewardMode::Native, None, RewardSelection::Native),
        (RewardMode::Native, Some(0), RewardSelection::Native),
        (RewardMode::Native, Some(25), RewardSelection::Native),
        (RewardMode::Native, Some(100000), RewardSelection::Native),
        (RewardMode::SourceXp, None, RewardSelection::Native),
        (RewardMode::SourceXp, Some(0), RewardSelection::SourceXp(0)),
        (
            RewardMode::SourceXp,
            Some(25),
            RewardSelection::SourceXp(25),
        ),
        (
            RewardMode::SourceXp,
            Some(100000),
            RewardSelection::SourceXp(100000),
        ),
        (RewardMode::Off, None, RewardSelection::Off),
        (RewardMode::Off, Some(0), RewardSelection::Off),
        (RewardMode::Off, Some(25), RewardSelection::Off),
        (RewardMode::Off, Some(100000), RewardSelection::Off),
    ];
    for (mode, xp, expected) in cases {
        assert_eq!(
            select_reward(mode, xp, "beta.activity.completed", false),
            expected
        );
        assert_eq!(
            select_reward(mode, xp, "beta.activity.completed", true),
            RewardSelection::Off
        );
        assert_eq!(
            select_reward(mode, xp, SUITE_CONNECTION_TEST_TYPE, false),
            RewardSelection::Off
        );
        assert_eq!(
            select_reward(mode, xp, SUITE_CONNECTION_TEST_TYPE, true),
            RewardSelection::Off
        );
    }
    let event = SuiteEvent {
        natural_key: "2026-10-06".into(),
        event_type: "alpha.activity.completed".into(),
        occurred_at: Utc::now(),
        payload: serde_json::from_value(
            json!({"activityId": "0199ba71-6000-7000-8000-000000000001", "minutes": 3, "xp": 30}),
        )
        .expect("payload"),
    };
    for global in [false, true] {
        for link in [false, true] {
            let envelope = event.envelope_for_link("sub", SUITE_APP_ID, global, link);
            assert_eq!(envelope.payload.contains_key("xp"), global && link);
            assert_eq!(envelope.payload["minutes"], 3);
        }
    }
    assert_eq!(event.payload["xp"], 30);
}

#[test]
fn signature_vectors_preserve_baukit_signing_bytes_and_suite_timestamp_policy() {
    let fixture = fixture("signature.json");
    for vector in fixture["signingCases"].as_array().expect("valid fixture") {
        let body = vector["body"]
            .as_str()
            .map(|body| body.as_bytes().to_vec())
            .unwrap_or_else(|| decode_hex(vector["bodyHex"].as_str().expect("valid fixture")));
        let timestamp = vector["timestamp"].as_i64().expect("valid fixture");
        let id = vector["deliveryId"].as_str().expect("valid fixture");
        assert_eq!(
            sign_webhook_hmac_sha256(
                vector["secret"].as_str().expect("valid fixture").as_bytes(),
                timestamp,
                id,
                &body
            ),
            vector["signature"]
        );
        assert_eq!(
            webhook_signing_input(timestamp, id, &body),
            decode_hex(vector["signingInputHex"].as_str().expect("valid fixture"))
        );
    }
    for vector in fixture["verificationCases"]
        .as_array()
        .expect("valid fixture")
    {
        let secrets: Vec<_> = vector["candidateSecrets"]
            .as_array()
            .expect("valid fixture")
            .iter()
            .map(|secret| secret.as_str().expect("valid fixture").as_bytes())
            .collect();
        let body = vector["body"].as_str().expect("valid fixture").as_bytes();
        assert_eq!(
            verify_webhook_hmac_sha256(
                secrets,
                vector["timestamp"].as_i64().expect("valid fixture"),
                vector["deliveryId"].as_str().expect("valid fixture"),
                body,
                vector["signature"].as_str().expect("valid fixture")
            ),
            vector["expected"].as_bool().expect("valid fixture"),
            "{}",
            vector["name"]
        );
    }
    for vector in fixture["suiteCases"].as_array().expect("valid fixture") {
        let timestamp = vector["timestamp"].as_i64().expect("valid fixture");
        let now = vector["now"].as_i64().expect("valid fixture");
        let cryptographic = verify_webhook_hmac_sha256(
            [vector["secret"].as_str().expect("valid fixture").as_bytes()],
            timestamp,
            vector["deliveryId"].as_str().expect("valid fixture"),
            vector["body"].as_str().expect("valid fixture").as_bytes(),
            vector["signature"].as_str().expect("valid fixture"),
        );
        assert_eq!(
            cryptographic,
            vector["cryptographicValid"]
                .as_bool()
                .expect("valid fixture")
        );
        assert_eq!(
            cryptographic && signature_timestamp_valid(timestamp, now),
            vector["expected"].as_bool().expect("valid fixture")
        );
    }
}

#[test]
fn secrets_are_redacted_from_exchange_debug_output() {
    let protocol = protocol();
    let sample = protocol["payloads"]
        .as_array()
        .expect("valid fixture")
        .iter()
        .find(|sample| sample["name"] == "exchange")
        .expect("valid fixture");
    let request: ExchangeRequest =
        serde_json::from_value(sample["request"].clone()).expect("valid fixture");
    let debug = format!("{request:?}");
    assert!(!debug.contains(&request.link_secret));
    assert!(!debug.contains(&request.code_verifier));
    assert!(!debug.contains(&request.code));
    assert!(debug.contains("[redacted]"));
}

#[test]
fn signature_timestamp_window_is_inclusive_and_handles_extreme_integers() {
    for delta in [-300, 0, 300] {
        assert!(signature_timestamp_valid(
            1_800_000_000 + delta,
            1_800_000_000
        ));
    }
    for delta in [-301, 301] {
        assert!(!signature_timestamp_valid(
            1_800_000_000 + delta,
            1_800_000_000
        ));
    }
    assert!(!signature_timestamp_valid(i64::MIN, i64::MAX));
}

#[test]
fn persisted_suite_enum_values_round_trip() {
    for value in [
        LinkStatus::Active,
        LinkStatus::NeedsAttention,
        LinkStatus::Revoked,
    ] {
        assert_eq!(
            value.as_str().parse::<LinkStatus>().expect("valid fixture"),
            value
        );
        assert_eq!(
            serde_json::to_value(value).expect("valid fixture"),
            value.as_str()
        );
    }
    for value in [
        DeliveryHealth::Healthy,
        DeliveryHealth::Degraded,
        DeliveryHealth::NeedsAttention,
        DeliveryHealth::Disabled,
    ] {
        assert_eq!(
            value
                .as_str()
                .parse::<DeliveryHealth>()
                .expect("valid fixture"),
            value
        );
        assert_eq!(
            serde_json::to_value(value).expect("valid fixture"),
            value.as_str()
        );
    }
    for value in [RewardMode::Native, RewardMode::SourceXp, RewardMode::Off] {
        assert_eq!(
            value.as_str().parse::<RewardMode>().expect("valid fixture"),
            value
        );
        assert_eq!(
            serde_json::to_value(value).expect("valid fixture"),
            value.as_str()
        );
    }
    for value in [LinkRole::Initiator, LinkRole::Authorizer] {
        assert_eq!(
            value.as_str().parse::<LinkRole>().expect("valid fixture"),
            value
        );
        assert_eq!(
            serde_json::to_value(value).expect("valid fixture"),
            value.as_str()
        );
    }
    assert!("bad".parse::<LinkStatus>().is_err());
    assert!("bad".parse::<DeliveryHealth>().is_err());
    assert!("bad".parse::<RewardMode>().is_err());
    assert!("bad".parse::<LinkRole>().is_err());
}

#[test]
fn enqueue_gate_and_delivery_preflight_respect_status_and_the_breaker() {
    let cases = [
        (LinkStatus::Active, DeliveryHealth::Healthy, 0, true, None),
        (LinkStatus::Active, DeliveryHealth::Degraded, 19, true, None),
        (
            LinkStatus::Active,
            DeliveryHealth::NeedsAttention,
            19,
            true,
            None,
        ),
        (
            LinkStatus::Active,
            DeliveryHealth::Healthy,
            20,
            false,
            Some(DeliveryAction::Disabled),
        ),
        (
            LinkStatus::Active,
            DeliveryHealth::Healthy,
            u32::MAX,
            false,
            Some(DeliveryAction::Disabled),
        ),
        (
            LinkStatus::Active,
            DeliveryHealth::Disabled,
            0,
            false,
            Some(DeliveryAction::Disabled),
        ),
        (
            LinkStatus::NeedsAttention,
            DeliveryHealth::NeedsAttention,
            0,
            false,
            None,
        ),
        (
            LinkStatus::Revoked,
            DeliveryHealth::Healthy,
            0,
            false,
            Some(DeliveryAction::LinkRevoked),
        ),
        (
            LinkStatus::Revoked,
            DeliveryHealth::Disabled,
            20,
            false,
            Some(DeliveryAction::LinkRevoked),
        ),
    ];
    for (status, delivery_health, consecutive_failures, enqueue, action) in cases {
        let link = SuiteLink {
            status,
            delivery_health,
            consecutive_failures,
            ..test_link()
        };
        assert_eq!(link.can_enqueue(), enqueue);
        assert_eq!(delivery_preflight(Some(&link)), action);
    }
    assert_eq!(delivery_preflight(None), Some(DeliveryAction::LinkMissing));
    let state = DeliveryState {
        status: LinkStatus::Active,
        health: DeliveryHealth::Degraded,
        consecutive_failures: 7,
    };
    for (action, outcome) in [
        (DeliveryAction::LinkMissing, None),
        (DeliveryAction::LinkRevoked, None),
        (DeliveryAction::Disabled, Some("disabled")),
        (DeliveryAction::Delivered, Some("delivered")),
        (DeliveryAction::Retry { after: None }, Some("retry")),
        (DeliveryAction::Rejected, Some("rejected")),
        (DeliveryAction::Unauthorized, Some("unauthorized")),
        (DeliveryAction::Revoked, Some("revoked")),
    ] {
        assert_eq!(action.outcome(), outcome);
    }
    for action in [
        DeliveryAction::LinkMissing,
        DeliveryAction::LinkRevoked,
        DeliveryAction::Disabled,
    ] {
        assert_eq!(
            action.permanent_code(),
            if action == DeliveryAction::Disabled {
                Some("suite_link_disabled")
            } else {
                None
            }
        );
        assert_eq!(after_delivery(state, action), state);
    }
}

#[test]
fn replay_since_is_inclusive_at_365_days_and_rejects_future_dates() {
    let today: NaiveDate = "2026-10-07".parse().expect("valid fixture");
    for (since, expected) in [
        ("2026-10-07", Ok(())),
        ("2026-10-06", Ok(())),
        ("2025-10-07", Ok(())),
        ("2025-10-06", Err(ReplayWindowError::TooOld)),
        ("2026-10-08", Err(ReplayWindowError::Future)),
    ] {
        assert_eq!(
            validate_replay_since(since.parse().expect("valid fixture"), today),
            expected
        );
    }
    assert_eq!(
        validate_replay_since(NaiveDate::MIN, today),
        Err(ReplayWindowError::TooOld)
    );
    assert_eq!(
        validate_replay_since(NaiveDate::MAX, today),
        Err(ReplayWindowError::Future)
    );
}

#[test]
fn callback_fixtures_are_relays_and_do_not_consume_the_request() {
    let corpus = protocol();
    let cases = corpus["payloads"]
        .as_array()
        .expect("valid fixture")
        .iter()
        .chain(corpus["errors"].as_array().expect("valid fixture"));
    let mut names = BTreeSet::new();
    for case in cases.filter(|case| case["operation"] == "callback") {
        names.insert(case["name"].as_str().expect("valid fixture"));
        let request = protocol_request(&case["context"]["storedRequest"]);
        let original = request.clone();
        let query: LinkCallbackQuery =
            serde_json::from_value(case["request"].clone()).expect("valid fixture");
        assert_eq!(
            serde_json::to_value(&query).expect("valid fixture"),
            case["request"]
        );
        let now = case["context"]["now"]
            .as_str()
            .expect("valid fixture")
            .parse()
            .expect("valid fixture");
        let result = relay_callback(request.as_ref(), &query, now);
        if case["expectedStatus"] == 400 {
            assert_eq!(result, Err(CallbackError::CodeInvalid));
            assert_eq!(
                result.expect_err("invalid fixture").code(),
                case["error"].as_str().expect("valid fixture")
            );
        } else {
            assert_eq!(case["expectedStatus"], 303);
            assert_eq!(
                serde_json::to_value(result.expect("valid fixture")).expect("valid fixture"),
                case["response"],
                "{}",
                case["name"]
            );
        }
        assert_eq!(request, original);
    }
    assert_eq!(
        names,
        [
            "callback-relay",
            "callback-denied",
            "callback-mix-up",
            "callback-expired",
            "callback-consumed",
            "callback-wrong-state",
            "callback-missing-from",
            "callback-unknown",
            "callback-both-code-and-error",
            "callback-encoded-query",
            "callback-native"
        ]
        .into_iter()
        .collect()
    );
}

#[test]
fn callback_boundary_and_invalid_return_values_have_literal_results() {
    let corpus = protocol();
    let sample = corpus["payloads"]
        .as_array()
        .expect("valid fixture")
        .iter()
        .find(|case| case["name"] == "callback-relay")
        .expect("valid fixture");
    let mut request = protocol_request(&sample["context"]["storedRequest"]).expect("valid fixture");
    let query: LinkCallbackQuery =
        serde_json::from_value(sample["request"].clone()).expect("valid fixture");
    let failed = "https://alpha.example/suite/linked?state=client-nonce-17&status=failed&code=suite_code_invalid";
    assert_eq!(
        relay_callback(Some(&request), &query, request.expires_at)
            .expect("valid fixture")
            .location,
        failed
    );
    assert_eq!(
        relay_callback(
            Some(&request),
            &query,
            request.expires_at - chrono::TimeDelta::seconds(1)
        )
        .expect("valid fixture")
        .location,
        "https://alpha.example/suite/linked?state=client-nonce-17&request=0199ba71-6000-7000-8000-000000000010&code=AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8"
    );
    let now = sample["context"]["now"]
        .as_str()
        .expect("valid fixture")
        .parse()
        .expect("valid fixture");
    for query in [
        LinkCallbackQuery {
            state: None,
            ..query.clone()
        },
        LinkCallbackQuery {
            state: Some(String::new()),
            ..query.clone()
        },
        LinkCallbackQuery {
            code: None,
            ..query.clone()
        },
        LinkCallbackQuery {
            code: Some(String::new()),
            ..query.clone()
        },
        LinkCallbackQuery {
            code: None,
            error: Some("unexpected-error".into()),
            ..query.clone()
        },
    ] {
        assert_eq!(
            relay_callback(Some(&request), &query, now)
                .expect("valid fixture")
                .location,
            failed
        );
    }
    for url in [
        "invalid",
        "javascript:alert(1)",
        "https://user:password@alpha.example/suite/linked",
    ] {
        request.return_url = url.into();
        assert_eq!(
            relay_callback(Some(&request), &query, now),
            Err(CallbackError::CodeInvalid)
        );
    }
}

#[test]
fn callback_query_encoding_preserves_unrelated_values_and_replaces_protocol_keys() {
    let corpus = protocol();
    let sample = corpus["payloads"]
        .as_array()
        .expect("valid fixture")
        .iter()
        .find(|case| case["name"] == "callback-encoded-query")
        .expect("valid fixture");
    let request = protocol_request(&sample["context"]["storedRequest"]).expect("valid fixture");
    let query: LinkCallbackQuery =
        serde_json::from_value(sample["request"].clone()).expect("valid fixture");
    let now = sample["context"]["now"]
        .as_str()
        .expect("valid fixture")
        .parse()
        .expect("valid fixture");
    let result = relay_callback(Some(&request), &query, now).expect("valid fixture");
    let url = Url::parse(&result.location).expect("valid fixture");
    assert_eq!(
        url.query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect::<Vec<_>>(),
        vec![
            ("view".to_owned(), "apps & links".to_owned()),
            ("state".to_owned(), "nonce +&=ü/?".to_owned()),
            (
                "request".to_owned(),
                "0199ba71-6000-7000-8000-000000000010".to_owned()
            ),
            ("code".to_owned(), "code +&=ü/?".to_owned()),
        ]
    );
    let mut query = query;
    query.code = None;
    query.error = Some("access_denied".into());
    assert_eq!(
        relay_callback(Some(&request), &query, now)
            .expect("valid fixture")
            .location,
        "https://alpha.example/suite/linked?view=apps+%26+links&state=nonce+%2B%26%3D%C3%BC%2F%3F&request=0199ba71-6000-7000-8000-000000000010&status=denied"
    );
    query.from = Some("unknown".into());
    assert_eq!(
        relay_callback(Some(&request), &query, now)
            .expect("valid fixture")
            .location,
        "https://alpha.example/suite/linked?view=apps+%26+links&state=nonce+%2B%26%3D%C3%BC%2F%3F&status=failed&code=suite_code_invalid"
    );
}

#[test]
fn complete_fixtures_enforce_owner_binding_and_return_the_recorded_link_on_retry() {
    let corpus = protocol();
    let cases = corpus["payloads"]
        .as_array()
        .expect("valid fixture")
        .iter()
        .chain(corpus["errors"].as_array().expect("valid fixture"));
    let mut checked = 0;
    for case in cases.filter(|case| {
        case["operation"] == "complete" && case["context"].get("peerFailure").is_none()
    }) {
        checked += 1;
        let request = protocol_request(&case["context"]["storedRequest"]);
        let wire: LinkCompleteRequest =
            serde_json::from_value(case["request"].clone()).expect("valid fixture");
        assert_eq!(
            serde_json::to_value(wire).expect("valid fixture"),
            case["request"]
        );
        let owner = case["context"]["ownerId"]
            .as_str()
            .expect("valid fixture")
            .parse()
            .expect("valid fixture");
        let now = case["context"]["now"]
            .as_str()
            .expect("valid fixture")
            .parse()
            .expect("valid fixture");
        let result = validate_complete(request.as_ref(), owner, now);
        match case["expectedDecision"].as_str() {
            Some("create_link") => {
                assert_eq!(case["expectedStatus"], 201);
                assert_eq!(result, Ok(CompleteDecision::CreateLink));
            }
            Some("existing_link") => {
                assert_eq!(case["expectedStatus"], 201);
                assert_eq!(
                    result,
                    Ok(CompleteDecision::ExistingLink(
                        case["response"]["id"]
                            .as_str()
                            .expect("valid fixture")
                            .parse()
                            .expect("valid fixture")
                    ))
                );
            }
            None => {
                let error = result.expect_err("invalid fixture");
                assert_eq!(error.code(), case["error"].as_str().expect("valid fixture"));
                if case["expectedStatus"] == 404 {
                    assert_eq!(error, LinkProtocolError::NotFound);
                } else {
                    assert_eq!(case["expectedStatus"], 422);
                    assert_eq!(error, LinkProtocolError::CodeInvalid);
                }
            }
            other => panic!("unexpected fixture decision {other:?}"),
        }
    }
    assert_eq!(checked, 7);
}

#[test]
fn completed_requests_still_hide_their_link_from_other_owners() {
    let corpus = protocol();
    let sample = corpus["payloads"]
        .as_array()
        .expect("valid fixture")
        .iter()
        .find(|case| case["name"] == "complete-retry")
        .expect("valid fixture");
    let mut request = protocol_request(&sample["context"]["storedRequest"]).expect("valid fixture");
    let now = sample["context"]["now"]
        .as_str()
        .expect("valid fixture")
        .parse()
        .expect("valid fixture");
    let other_owner = "0199ba71-6000-7000-8000-000000000099"
        .parse()
        .expect("valid fixture");
    assert_eq!(
        validate_complete(Some(&request), other_owner, now),
        Err(LinkProtocolError::NotFound)
    );
    request.consumed_at = None;
    assert_eq!(
        validate_complete(Some(&request), request.user_id, now),
        Err(LinkProtocolError::CodeInvalid)
    );
    request.link_id = None;
    assert_eq!(
        validate_complete(Some(&request), request.user_id, request.expires_at),
        Err(LinkProtocolError::CodeInvalid)
    );
    assert_eq!(
        validate_complete(
            Some(&request),
            request.user_id,
            request.expires_at - chrono::TimeDelta::seconds(1)
        ),
        Ok(CompleteDecision::CreateLink)
    );
}

#[test]
fn exchange_fixtures_pin_verifier_subject_and_idempotent_retry_rules() {
    let corpus = protocol();
    let cases = corpus["payloads"]
        .as_array()
        .expect("valid fixture")
        .iter()
        .chain(corpus["errors"].as_array().expect("valid fixture"));
    let mut checked = 0;
    for case in cases.filter(|case| {
        case["operation"] == "exchange" && case["context"].get("storedCode").is_some()
    }) {
        checked += 1;
        let code = protocol_code(&case["context"]["storedCode"]);
        let link = protocol_link(&case["context"]["existingLink"]);
        let wire: ExchangeRequest =
            serde_json::from_value(case["request"].clone()).expect("valid fixture");
        assert_eq!(
            serde_json::to_value(&wire).expect("valid fixture"),
            case["request"]
        );
        let now = case["context"]["now"]
            .as_str()
            .expect("valid fixture")
            .parse()
            .expect("valid fixture");
        let result = validate_exchange(code.as_ref(), &wire, link.as_ref(), now);
        match case["expectedDecision"].as_str() {
            Some("create_link") => {
                assert_eq!(case["expectedStatus"], 201);
                assert_eq!(
                    result,
                    Ok(ExchangeDecision::CreateLink {
                        suite_subject: case["expectedSuiteSubject"].as_str().map(str::to_owned)
                    })
                );
            }
            Some("existing_link") => {
                assert_eq!(case["expectedStatus"], 201);
                assert_eq!(
                    result,
                    Ok(ExchangeDecision::ExistingLink(
                        case["response"]["linkId"]
                            .as_str()
                            .expect("valid fixture")
                            .parse()
                            .expect("valid fixture")
                    ))
                );
                let original = corpus["payloads"]
                    .as_array()
                    .expect("valid fixture")
                    .iter()
                    .find(|case| case["name"] == "exchange")
                    .expect("valid fixture");
                assert_eq!(case["response"], original["response"]);
            }
            None => {
                let error = result.expect_err("invalid fixture");
                assert_eq!(
                    error.code(),
                    case["error"].as_str().expect("valid fixture"),
                    "{}",
                    case["name"]
                );
                if case["expectedStatus"] == 409 {
                    assert_eq!(error, LinkProtocolError::AccountMismatch);
                } else {
                    assert_eq!(case["expectedStatus"], 422);
                    assert_eq!(error, LinkProtocolError::CodeInvalid);
                }
            }
            other => panic!("unexpected fixture decision {other:?}"),
        }
    }
    assert_eq!(checked, 15);
}

#[test]
fn exchange_retries_cannot_bypass_authentication_subjects_or_the_recorded_link() {
    let corpus = protocol();
    let sample = corpus["payloads"]
        .as_array()
        .expect("valid fixture")
        .iter()
        .find(|case| case["name"] == "exchange-retry")
        .expect("valid fixture");
    let mut code = protocol_code(&sample["context"]["storedCode"]).expect("valid fixture");
    let request: ExchangeRequest =
        serde_json::from_value(sample["request"].clone()).expect("valid fixture");
    let link = protocol_link(&sample["context"]["existingLink"]).expect("valid fixture");
    let now = sample["context"]["now"]
        .as_str()
        .expect("valid fixture")
        .parse()
        .expect("valid fixture");
    assert_eq!(
        validate_exchange(Some(&code), &request, Some(&link), code.expires_at),
        Err(LinkProtocolError::CodeInvalid)
    );
    assert_eq!(
        validate_exchange(
            Some(&code),
            &request,
            Some(&link),
            code.expires_at - chrono::TimeDelta::seconds(1)
        ),
        Ok(ExchangeDecision::ExistingLink(link.id))
    );
    for (request, expected) in [
        (
            ExchangeRequest {
                code: "different-code".into(),
                ..request.clone()
            },
            LinkProtocolError::CodeInvalid,
        ),
        (
            ExchangeRequest {
                client: "different-client".into(),
                ..request.clone()
            },
            LinkProtocolError::CodeInvalid,
        ),
        (
            ExchangeRequest {
                code_verifier: "invalid".into(),
                ..request.clone()
            },
            LinkProtocolError::CodeInvalid,
        ),
        (
            ExchangeRequest {
                initiator_suite_subject: Some("suite-local|other-user".into()),
                ..request.clone()
            },
            LinkProtocolError::AccountMismatch,
        ),
    ] {
        assert_eq!(
            validate_exchange(Some(&code), &request, Some(&link), now),
            Err(expected)
        );
    }
    for link in [
        SuiteLink {
            id: Uuid::nil(),
            ..link.clone()
        },
        SuiteLink {
            user_id: Uuid::nil(),
            ..link.clone()
        },
        SuiteLink {
            peer_app: "different-client".into(),
            ..link.clone()
        },
        SuiteLink {
            remote_link_id: Uuid::nil(),
            ..link.clone()
        },
    ] {
        assert_eq!(
            validate_exchange(Some(&code), &request, Some(&link), now),
            Err(LinkProtocolError::CodeInvalid)
        );
    }
    code.auto_approved = true;
    for subject in [
        None,
        Some("suite-local|other-user".into()),
        Some("clerk-prod|user-17".into()),
    ] {
        let request = ExchangeRequest {
            initiator_suite_subject: subject,
            ..request.clone()
        };
        assert_eq!(
            validate_exchange(Some(&code), &request, Some(&link), now),
            Err(LinkProtocolError::AccountMismatch)
        );
    }
    assert_eq!(
        validate_exchange(Some(&code), &request, Some(&link), now),
        Ok(ExchangeDecision::ExistingLink(link.id))
    );
    code.suite_subject = None;
    assert_eq!(
        validate_exchange(Some(&code), &request, Some(&link), now),
        Err(LinkProtocolError::AccountMismatch)
    );
    code.auto_approved = false;
    code.consumed_at = None;
    assert_eq!(
        validate_exchange(Some(&code), &request, None, now),
        Err(LinkProtocolError::CodeInvalid)
    );
    code.link_id = None;
    assert_eq!(
        validate_exchange(Some(&code), &request, None, now),
        Ok(ExchangeDecision::CreateLink {
            suite_subject: None
        })
    );
    code.suite_subject = Some("invalid-subject".into());
    assert_eq!(
        validate_exchange(Some(&code), &request, None, now),
        Err(LinkProtocolError::CodeInvalid)
    );
}

#[test]
fn preview_fixtures_and_absent_subjects_pin_hint_mismatch() {
    let registry = protocol_authorizer_registry();
    let corpus = protocol();
    let cases = corpus["payloads"]
        .as_array()
        .expect("valid fixture")
        .iter()
        .filter(|case| case["operation"] == "preview");
    let mut checked = 0;
    for case in cases {
        checked += 1;
        let result = preview_authorization(
            &registry,
            case["request"]["client"].as_str().expect("valid fixture"),
            case["context"]["existingLink"]
                .as_bool()
                .expect("valid fixture"),
            case["request"]["hint"].as_str(),
            case["context"]["suiteSubject"].as_str(),
            Some("suite-local"),
        )
        .expect("valid fixture");
        assert_eq!(
            serde_json::to_value(result).expect("valid fixture"),
            case["response"]
        );
        assert_eq!(case["expectedStatus"], 200);
    }
    assert_eq!(checked, 3);
    let peer = registry.active_peer("alpha").expect("valid fixture");
    for (hint, subject, auto_approve, hint_mismatch) in [
        (None, None, false, false),
        (None, Some("suite-local|user-17"), false, false),
        (Some("wrong-hint"), None, false, false),
        (Some(""), Some("suite-local|user-17"), false, true),
        (
            Some("kPetKgWl81rXeCG76KqGdoiCdsjA1KPz5PpHIZRznp4"),
            Some("suite-local|user-17"),
            true,
            false,
        ),
    ] {
        let preview = AuthorizationPreview::new(peer, true, hint, subject, Some("suite-local"));
        assert_eq!(preview.auto_approve, auto_approve);
        assert_eq!(preview.hint_mismatch, hint_mismatch);
        assert!(preview.existing_link);
    }
    assert_eq!(
        preview_authorization(&registry, "unknown", false, None, None, None),
        Err(LinkProtocolError::PeerUnknown)
    );
}

#[test]
fn start_fixtures_validate_urls_nonces_and_peer_availability() {
    let corpus = protocol();
    let sample = corpus["payloads"]
        .as_array()
        .expect("valid fixture")
        .iter()
        .find(|case| case["operation"] == "start")
        .expect("valid fixture");
    let wire: LinkStartRequest =
        serde_json::from_value(sample["request"].clone()).expect("valid fixture");
    assert_eq!(
        serde_json::to_value(&wire).expect("valid fixture"),
        sample["request"]
    );
    let registry = configured_registry();
    assert_eq!(
        validate_link_start(&registry, &wire)
            .expect("valid fixture")
            .metadata
            .id,
        "beta"
    );
    let replacement =
        protocol_request(&sample["context"]["replacementRequest"]).expect("valid fixture");
    assert_eq!(replacement.client_state_nonce, "replacement-nonce");
    assert_eq!(replacement.id.to_string(), sample["response"]["requestId"]);
    let authorization = Url::parse(
        sample["response"]["authorizeUrl"]
            .as_str()
            .expect("valid fixture"),
    )
    .expect("valid fixture");
    assert_eq!(
        authorization.origin().ascii_serialization(),
        "https://beta.example"
    );
    assert_eq!(authorization.path(), "/suite/authorize");
    assert_eq!(
        authorization
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<Vec<_>>(),
        [
            ("client", "alpha"),
            ("state", "YGFiY2RlZmdoaWprbG1ub3BxcnN0dXZ3eHl6e3x9fn8"),
            (
                "code_challenge",
                "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
            ),
            ("hint", "kPetKgWl81rXeCG76KqGdoiCdsjA1KPz5PpHIZRznp4"),
        ]
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
    );
    assert_eq!(
        URL_SAFE_NO_PAD
            .decode(
                sample["context"]["replacementRequest"]["state"]
                    .as_str()
                    .expect("valid fixture")
            )
            .expect("valid fixture"),
        (96..128).collect::<Vec<u8>>()
    );
    assert!(
        sample["expectedSuperseded"]
            .as_bool()
            .expect("valid fixture")
    );
    for (request, expected) in [
        (
            LinkStartRequest {
                peer_app: "unknown".into(),
                ..wire.clone()
            },
            LinkProtocolError::PeerUnknown,
        ),
        (
            LinkStartRequest {
                peer_app: "alpha".into(),
                ..wire.clone()
            },
            LinkProtocolError::PeerUnknown,
        ),
        (
            LinkStartRequest {
                return_url: "https://evil.example/suite/linked".into(),
                ..wire.clone()
            },
            LinkProtocolError::ReturnUrlInvalid,
        ),
        (
            LinkStartRequest {
                state_nonce: String::new(),
                ..wire.clone()
            },
            LinkProtocolError::PayloadInvalid,
        ),
        (
            LinkStartRequest {
                state_nonce: "a\nb".into(),
                ..wire.clone()
            },
            LinkProtocolError::PayloadInvalid,
        ),
    ] {
        assert_eq!(validate_link_start(&registry, &request), Err(expected));
    }
    assert_eq!(
        validate_link_start(
            &PeerRegistry::new(
                SUITE_APP_ID,
                SUITE_PEERS_JSON,
                PeerRegistrySettings::default()
            )
            .expect("valid fixture"),
            &wire
        ),
        Err(LinkProtocolError::Disabled)
    );
    let mut missing_nonce = sample["request"].clone();
    missing_nonce
        .as_object_mut()
        .expect("valid fixture")
        .remove("stateNonce");
    assert!(serde_json::from_value::<LinkStartRequest>(missing_nonce).is_err());
}

#[test]
fn unknown_peer_fixture_pins_the_shared_protocol_status() {
    let corpus = protocol();
    let case = corpus["errors"]
        .as_array()
        .expect("valid fixture")
        .iter()
        .find(|case| case["id"] == "unknown-peer" || case["name"] == "unknown-peer")
        .expect("unknown-peer fixture exists");
    assert_eq!(
        preview_authorization(
            &protocol_authorizer_registry(),
            case["request"]["client"].as_str().expect("valid fixture"),
            false,
            case["request"]["hint"].as_str(),
            None,
            None
        )
        .expect_err("invalid fixture"),
        LinkProtocolError::PeerUnknown
    );
    assert_eq!(case["expectedStatus"], 400);
    assert_eq!(case["error"], LinkProtocolError::PeerUnknown.code());
}

fn configured_registry() -> PeerRegistry {
    PeerRegistry::new(
        "alpha",
        SUITE_PEERS_JSON,
        PeerRegistrySettings {
            public_api_url: Some("https://alpha.example/api/v1".into()),
            public_web_url: Some("https://alpha.example".into()),
            peers: [(
                "beta".into(),
                PeerUrls {
                    api_url: Some("https://beta.example/api/v1".into()),
                    web_url: Some("https://beta.example".into()),
                },
            )]
            .into_iter()
            .collect(),
            allow_loopback: false,
        },
    )
    .expect("valid fixture")
}

#[test]
fn generic_validator_matches_every_payload_vector() {
    let corpus = fixture("payload-validator.json");
    let mut cases = 0;
    for vector in corpus["vectors"].as_array().expect("vectors") {
        let catalog = PayloadCatalog::from_json(
            &json!({
                "schemaVersion": 1,
                "types": [vector["definition"].clone()]
            })
            .to_string(),
        )
        .expect("vector catalog");
        for case in vector["cases"].as_array().expect("cases") {
            let payload = serde_json::from_value(case["payload"].clone()).expect("payload map");
            let result = catalog.validate("alpha.activity.completed", payload);
            let actual = result.as_ref().err().map(PayloadError::kind);
            assert_eq!(
                actual,
                case["error"].as_str(),
                "{} {}",
                vector["name"],
                case["name"]
            );
            cases += 1;
        }
    }
    assert_eq!(cases, 1145);
}

#[test]
fn fixture_checksums_pin_every_json_file() {
    let manifest = fs::read_to_string(fixture_root().join("SHA256SUMS")).expect("checksums");
    assert_eq!(
        baukit_test::suite::fixture("SHA256SUMS").expect("packaged checksums"),
        manifest,
        "packaged checksum manifest differs"
    );
    let mut files = BTreeSet::new();
    for line in manifest.lines() {
        let (digest, name) = line.split_once("  ").expect("checksum line");
        assert!(files.insert(name));
        let bytes = fs::read(fixture_root().join(name)).expect("fixture");
        assert_eq!(
            baukit_test::suite::fixture(name)
                .expect("packaged test-kit fixture")
                .as_bytes(),
            bytes,
            "packaged fixture differs: {name}"
        );
        assert_eq!(
            Sha256::digest(&bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
            digest,
            "{name}"
        );
    }
    let actual: BTreeSet<_> = fs::read_dir(fixture_root())
        .expect("fixtures")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.ends_with(".json"))
        .collect();
    assert_eq!(
        files
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>(),
        actual
    );
}

#[test]
fn catalog_rejects_invalid_contracts_and_exposes_typed_payloads() {
    let catalog = PayloadCatalog::from_json(include_str!(
        "../../../../../fixtures/suite-events/v1/catalog.json"
    ))
    .expect("catalog");
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Activity {
        activity_id: Uuid,
        minutes: u32,
        xp: Option<u32>,
    }
    let id = Uuid::new_v4();
    let payload = catalog
        .validate(
            "alpha.activity.completed",
            serde_json::from_value(json!({"activityId": id, "minutes": 60})).expect("map"),
        )
        .expect("valid");
    let typed: Activity = payload.deserialize().expect("typed");
    assert_eq!(typed.activity_id, id);
    assert_eq!(typed.minutes, 60);
    assert_eq!(typed.xp, None);
    let original: Value = serde_json::from_str(include_str!(
        "../../../../../fixtures/suite-events/v1/catalog.json"
    ))
    .expect("json");
    for (field, value) in [
        ("maximum", json!(9007199254740992_i64)),
        ("minimum", json!(2000)),
    ] {
        let mut invalid = original.clone();
        invalid["types"][0]["fields"][1][field] = value;
        assert!(PayloadCatalog::from_json(&invalid.to_string()).is_err());
    }
}

#[test]
fn registry_requires_both_public_urls_and_a_configured_peer_with_event_flow() {
    let settings = PeerRegistrySettings {
        public_api_url: Some("https://alpha.example/api/v1".into()),
        public_web_url: Some("https://alpha.example".into()),
        peers: BTreeMap::from([(
            "beta".into(),
            PeerUrls {
                api_url: Some("https://beta.example/api/v1".into()),
                web_url: Some("https://beta.example".into()),
            },
        )]),
        allow_loopback: false,
    };
    for public_api in [false, true] {
        for public_web in [false, true] {
            for peer_api in [false, true] {
                for peer_web in [false, true] {
                    let mut candidate = settings.clone();
                    if !public_api {
                        candidate.public_api_url = None;
                    }
                    if !public_web {
                        candidate.public_web_url = None;
                    }
                    let peer = candidate.peers.get_mut("beta").expect("peer");
                    if !peer_api {
                        peer.api_url = None;
                    }
                    if !peer_web {
                        peer.web_url = None;
                    }
                    let registry =
                        PeerRegistry::new("alpha", SUITE_PEERS_JSON, candidate).expect("registry");
                    assert_eq!(
                        registry.standalone(),
                        !(public_api && public_web && peer_api && peer_web)
                    );
                }
            }
        }
    }
    let mut metadata: Value = serde_json::from_str(SUITE_PEERS_JSON).expect("peers");
    for peer in metadata["peers"].as_array_mut().expect("peers") {
        peer["emits"] = json!([SUITE_CONNECTION_TEST_TYPE]);
        peer["accepts"] = json!([SUITE_CONNECTION_TEST_TYPE]);
    }
    assert!(
        PeerRegistry::new("alpha", &metadata.to_string(), settings.clone())
            .expect("registry")
            .standalone()
    );
    for invalid in [
        "http://alpha.example/api/v1",
        "https://user:pass@alpha.example",
        "https://alpha.example?q=x",
        "https://alpha.example#fragment",
    ] {
        let mut candidate = settings.clone();
        candidate.public_api_url = Some(invalid.into());
        assert!(PeerRegistry::new("alpha", SUITE_PEERS_JSON, candidate).is_err());
    }
    for valid in [
        "http://localhost:1234/api/v1",
        "http://127.0.0.1:1234/api/v1",
        "http://[::1]:1234/api/v1",
    ] {
        let mut candidate = settings.clone();
        candidate.public_api_url = Some(valid.into());
        assert!(PeerRegistry::new("alpha", SUITE_PEERS_JSON, candidate.clone()).is_err());
        candidate.allow_loopback = true;
        assert!(PeerRegistry::new("alpha", SUITE_PEERS_JSON, candidate).is_ok());
    }
    for invalid in ["", "Alpha", "alpha-beta", "alpha beta", &"a".repeat(65)] {
        let mut metadata: Value = serde_json::from_str(SUITE_PEERS_JSON).expect("peers");
        metadata["peers"][0]["id"] = json!(invalid);
        assert!(PeerRegistry::new(invalid, &metadata.to_string(), settings.clone()).is_err());
    }
    assert!(PeerRegistry::new("missing", SUITE_PEERS_JSON, settings).is_err());
}

#[test]
fn inbound_checks_every_envelope_rule_during_replay_and_protocol_tests() {
    use baukit_events::{EventValidationCode as Code, MAX_EVENT_AGE_SECONDS};
    let catalog = PayloadCatalog::from_json(include_str!(
        "../../../../../fixtures/suite-events/v1/catalog.json"
    ))
    .expect("catalog");
    let mut link = test_link();
    let envelope = SuiteEvent {
        natural_key: "event".into(),
        event_type: "beta.activity.completed".into(),
        occurred_at: link.created_at,
        payload: json!({"activityId":link.id,"minutes":10})
            .as_object()
            .expect("payload")
            .clone(),
    }
    .envelope("sender-subject", "beta");
    for replay in [false, true] {
        assert!(validate_inbound(&catalog, &envelope, &link, link.created_at, replay).is_ok());
        for (mut invalid, code) in [
            (
                EventEnvelope {
                    schema_version: 2,
                    ..envelope.clone()
                },
                Code::EventSchemaUnsupported,
            ),
            (
                EventEnvelope {
                    event_type: "Beta.activity.completed".into(),
                    ..envelope.clone()
                },
                Code::EventTypeInvalid,
            ),
            (
                EventEnvelope {
                    user_id: "different".into(),
                    ..envelope.clone()
                },
                Code::EventUserMismatch,
            ),
        ] {
            assert_eq!(
                validate_inbound(&catalog, &invalid, &link, link.created_at, replay),
                Err(InboundValidationError::Envelope(code))
            );
            invalid.event_id.clear();
            assert!(validate_inbound(&catalog, &invalid, &link, link.created_at, replay).is_err());
        }
        for id in ["", " padded", &"x".repeat(65)] {
            let invalid = EventEnvelope {
                event_id: id.into(),
                ..envelope.clone()
            };
            assert_eq!(
                validate_inbound(&catalog, &invalid, &link, link.created_at, replay),
                Err(InboundValidationError::Envelope(Code::EventIdInvalid))
            );
        }
    }
    let boundary = envelope.occurred_at + chrono::Duration::seconds(MAX_EVENT_AGE_SECONDS);
    assert!(validate_inbound(&catalog, &envelope, &link, boundary, false).is_ok());
    assert_eq!(
        validate_inbound(
            &catalog,
            &envelope,
            &link,
            boundary + chrono::Duration::seconds(1),
            false
        ),
        Err(InboundValidationError::Envelope(Code::EventTooOld))
    );
    assert!(
        validate_inbound(
            &catalog,
            &envelope,
            &link,
            boundary + chrono::Duration::seconds(1),
            true
        )
        .is_ok()
    );
    let mut test = SuiteEvent::connection_test(Uuid::new_v4(), link.created_at)
        .envelope("sender-subject", "beta");
    link.receives.clear();
    assert_eq!(
        validate_inbound(&catalog, &test, &link, link.created_at, false),
        Ok(ValidatedInbound::ConnectionTest)
    );
    for status in [LinkStatus::NeedsAttention, LinkStatus::Revoked] {
        link.status = status;
        assert_eq!(
            validate_inbound(&catalog, &test, &link, link.created_at, false),
            Err(InboundValidationError::UnsupportedType)
        );
    }
    link.status = LinkStatus::Active;
    test.source_app = "unknown".into();
    assert_eq!(
        validate_inbound(&catalog, &test, &link, link.created_at, true),
        Err(InboundValidationError::SourceAppMismatch)
    );
    test.source_app = "beta".into();
    test.payload.insert("xp".into(), json!(5));
    assert_eq!(
        validate_inbound(&catalog, &test, &link, link.created_at, false),
        Err(InboundValidationError::Payload(PayloadError::UnknownField(
            "xp".into()
        )))
    );
}

#[test]
fn versioned_jobs_and_exchange_values_use_the_exact_wire_shapes() {
    let protocol = protocol();
    let exchange = protocol["payloads"]
        .as_array()
        .expect("payloads")
        .iter()
        .find(|sample| sample["name"] == "exchange")
        .expect("exchange");
    let request: ExchangeRequest =
        serde_json::from_value(exchange["request"].clone()).expect("request");
    let response: ExchangeResponse =
        serde_json::from_value(exchange["response"].clone()).expect("response");
    assert_eq!(
        serde_json::to_value(request).expect("wire"),
        exchange["request"]
    );
    assert_eq!(
        serde_json::to_value(response).expect("wire"),
        exchange["response"]
    );
    let event = SuiteEvent::connection_test(Uuid::now_v7(), Utc::now()).envelope("owner", "alpha");
    let job = SuiteDeliverJobV1::new(Uuid::now_v7(), event, true);
    let wire = serde_json::to_value(&job).expect("wire");
    assert_eq!(wire["schema_version"], 1);
    assert_eq!(wire["replay"], true);
    assert_eq!(
        serde_json::from_value::<SuiteDeliverJobV1>(wire.clone()).expect("job"),
        job
    );
    for (field, value) in [("schema_version", json!(2)), ("unknown", json!(true))] {
        let mut invalid = wire.clone();
        invalid[field] = value;
        assert!(serde_json::from_value::<SuiteDeliverJobV1>(invalid).is_err());
    }
    let revoke = SuiteRevokeJobV1::new(Uuid::now_v7());
    let wire = serde_json::to_value(&revoke).expect("wire");
    assert_eq!(
        serde_json::from_value::<SuiteRevokeJobV1>(wire.clone()).expect("job"),
        revoke
    );
    for (field, value) in [("schema_version", json!(0)), ("unknown", json!(true))] {
        let mut invalid = wire.clone();
        invalid[field] = value;
        assert!(serde_json::from_value::<SuiteRevokeJobV1>(invalid).is_err());
    }
}
