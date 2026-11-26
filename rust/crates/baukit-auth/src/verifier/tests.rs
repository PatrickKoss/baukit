use std::time::Duration;

use super::*;

#[test]
fn infrastructure_failures_are_retryable_and_bad_credentials_are_not() {
    use super::VerificationError as E;
    for error in [
        E::Unavailable,
        E::JwksTimeout,
        E::JwksStatus(reqwest::StatusCode::SERVICE_UNAVAILABLE),
        E::JwksStatus(reqwest::StatusCode::TOO_MANY_REQUESTS),
        E::JwksStatus(reqwest::StatusCode::REQUEST_TIMEOUT),
        E::DiscoveryTimeout,
        E::DiscoveryStatus(reqwest::StatusCode::SERVICE_UNAVAILABLE),
        E::DiscoveryStatus(reqwest::StatusCode::TOO_MANY_REQUESTS),
        E::DiscoveryStatus(reqwest::StatusCode::REQUEST_TIMEOUT),
    ] {
        assert!(error.is_unavailable(), "{error:?}");
        assert!(!error.is_misconfigured(), "{error:?}");
    }
    for error in [
        E::DiscoveryIssuerMismatch,
        E::InvalidJwksUri,
        E::JwksStatus(reqwest::StatusCode::NOT_FOUND),
        E::DiscoveryStatus(reqwest::StatusCode::UNAUTHORIZED),
        E::Clock,
    ] {
        assert!(error.is_misconfigured(), "{error:?}");
        assert!(!error.is_unavailable(), "{error:?}");
    }
    for error in [
        E::Expired,
        E::MalformedToken,
        E::InvalidSignature,
        E::UnknownKeyId,
        E::WrongAudience,
    ] {
        assert!(!error.is_unavailable(), "{error:?}");
        assert!(!error.is_misconfigured(), "{error:?}");
    }
}

#[tokio::test]
async fn interrupted_document_bodies_are_retryable_but_invalid_json_is_not() {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    for truncated in [true, false] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let issuer = format!("http://{}", listener.local_addr().expect("address"));
        let server = tokio::spawn(async move {
            let body = "{";
            let length = body.len() + usize::from(truncated);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n{body}"
            );
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.expect("connection");
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    assert_ne!(stream.read_buf(&mut request).await.expect("request"), 0);
                }
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("response");
                stream.shutdown().await.expect("shutdown");
            }
        });
        let config = OidcConfig::new(&issuer, "api")
            .and_then(|config| config.with_startup_budget(Duration::from_nanos(1)))
            .expect("config");
        let discovery_error = match OidcVerifier::discover(config.clone()).await {
            Ok(_) => panic!("invalid discovery response"),
            Err(VerificationError::DiscoveryBudgetExhausted(source)) if truncated => *source,
            Err(error) => error,
        };
        let verifier =
            OidcVerifier::from_jwks_uri(config, format!("{issuer}/jwks")).expect("verifier");
        let jwks_error = verifier
            .fetch_jwks()
            .await
            .expect_err("invalid JWKS response");
        if truncated {
            assert!(matches!(
                discovery_error,
                VerificationError::DiscoveryRequest(_)
            ));
            assert!(matches!(jwks_error, VerificationError::JwksRequest(_)));
        } else {
            assert!(matches!(
                discovery_error,
                VerificationError::InvalidDiscoveryDocument(_)
            ));
            assert!(matches!(
                jwks_error,
                VerificationError::InvalidJwksDocument(_)
            ));
        }
        for error in [discovery_error, jwks_error] {
            assert_eq!(error.is_unavailable(), truncated, "{error:?}");
            assert_eq!(error.is_misconfigured(), !truncated, "{error:?}");
        }
        server.await.expect("server");
    }
}

#[tokio::test]
async fn http_error_displays_do_not_expose_urls_or_credentials() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new().fallback(|| async { "invalid JSON" }),
        )
        .await
        .expect("server");
    });
    let secret = "private-token-material";
    let issuer = format!("http://{address}/{secret}");
    let jwks = format!("http://{address}/jwks?token={secret}");
    let config = OidcConfig::new(&issuer, "api").expect("config");
    let blocked = config
        .clone()
        .with_egress_policy(baukit_egress::AddressPolicy::PublicOnly);
    let errors = [
        OidcVerifier::discover(config.clone())
            .await
            .map(|_| ())
            .expect_err("invalid discovery document"),
        OidcVerifier::discover(blocked.clone())
            .await
            .map(|_| ())
            .expect_err("blocked discovery"),
        OidcVerifier::from_jwks_uri(config, &jwks)
            .expect("verifier")
            .fetch_jwks()
            .await
            .map(|_| ())
            .expect_err("invalid JWKS document"),
        OidcVerifier::from_jwks_uri(blocked, &jwks)
            .expect("verifier")
            .fetch_jwks()
            .await
            .map(|_| ())
            .expect_err("blocked JWKS"),
    ];
    assert!(matches!(
        errors,
        [
            VerificationError::InvalidDiscoveryDocument(_),
            VerificationError::DiscoveryRequest(_),
            VerificationError::InvalidJwksDocument(_),
            VerificationError::JwksRequest(_),
        ]
    ));
    for error in errors {
        let mut source: Option<&dyn std::error::Error> = Some(&error);
        while let Some(current) = source {
            assert!(!current.to_string().contains(secret), "{current}");
            assert!(!current.to_string().contains(&address.to_string()));
            source = current.source();
        }
        assert!(error.is_misconfigured(), "{error:?}");
        assert!(!error.is_unavailable(), "{error:?}");
    }
    server.abort();
}

#[test]
fn unsigned_and_malformed_tokens_are_rejected_before_key_lookup() {
    assert!(matches!(
        ParsedToken::parse("abc.def"),
        Err(VerificationError::MalformedToken)
    ));
    assert!(matches!(
        ParsedToken::parse("eyJhbGciOiJub25lIn0.e30."),
        Err(VerificationError::InvalidSignature)
    ));
}

#[test]
fn principal_exposes_only_normalized_fields() {
    let principal = Principal::new("subject")
        .with_organization("org")
        .with_tenant("tenant");
    assert_eq!(principal.subject(), "subject");
    assert_eq!(principal.issuer(), None);
    assert_eq!(principal.authenticated_at(), None);
    assert_eq!(principal.organization(), Some("org"));
    assert_eq!(principal.tenant(), Some("tenant"));
    assert_eq!(principal.client_id(), None);
    assert!(principal.scopes().is_empty());
    assert_eq!(principal.grants(), None);
    assert!(principal.profile_claims().is_empty());
    assert_eq!(principal.api_token(), None);
}

#[test]
fn authentication_time_validates_integer_shape_and_clock_skew() {
    let now = 1_700_000_000;
    let skew = 60;
    assert_eq!(
        authentication_time(None, now, skew).expect("valid timestamp"),
        None
    );
    for seconds in [0, now - 1, now, now + skew] {
        assert_eq!(
            authentication_time(Some(&Value::from(seconds)), now, skew).expect("valid timestamp"),
            jiff::Timestamp::from_second(i64::try_from(seconds).expect("valid timestamp")).ok()
        );
    }
    for invalid in [
        Value::Null,
        Value::from(-1),
        Value::from(1.5),
        Value::from(1.0),
        Value::from("1700000000"),
        Value::from(true),
        serde_json::json!([]),
        serde_json::json!({}),
        Value::from(u64::MAX),
    ] {
        assert!(
            matches!(
                authentication_time(Some(&invalid), now, skew),
                Err(VerificationError::MalformedToken)
            ),
            "{invalid}"
        );
    }
    assert!(matches!(
        authentication_time(Some(&Value::from(now + skew + 1)), now, skew),
        Err(VerificationError::NotYetValid)
    ));
    let timestamp = jiff::Timestamp::from_second(i64::try_from(now).expect("valid timestamp"))
        .expect("valid timestamp");
    assert_eq!(
        Principal::new("subject")
            .with_authenticated_at(timestamp)
            .authenticated_at(),
        Some(timestamp)
    );
}

#[test]
fn scope_claims_accept_strings_and_string_arrays() {
    let expected = BTreeSet::from(["read".to_owned(), "write".to_owned()]);
    assert_eq!(
        scope_claim(Some(&Value::from(" read  write read "))).ok(),
        Some(expected.clone())
    );
    assert_eq!(
        scope_claim(Some(&serde_json::json!(["read", "write"]))).ok(),
        Some(expected)
    );
    assert_eq!(scope_claim(None).ok(), Some(BTreeSet::new()));
    assert_eq!(scope_claim(Some(&Value::Null)).ok(), Some(BTreeSet::new()));
    for invalid in [
        serde_json::json!(42),
        serde_json::json!(["read", 1]),
        serde_json::json!([""]),
        serde_json::json!({"read": true}),
    ] {
        assert!(matches!(
            scope_claim(Some(&invalid)),
            Err(VerificationError::InvalidPrincipalContext)
        ));
    }
}

#[test]
fn profile_claims_copy_only_selected_strings_and_booleans() {
    let claims = BTreeMap::from([
        ("email".to_owned(), Value::from("ada@example.com")),
        ("email_verified".to_owned(), Value::from(true)),
        ("preferred_username".to_owned(), Value::from(42)),
        ("phone_number".to_owned(), Value::from("+100")),
    ]);
    let selected = BTreeSet::from([
        "email".to_owned(),
        "email_verified".to_owned(),
        "preferred_username".to_owned(),
        "name".to_owned(),
    ]);
    let profile = selected_profile_claims(&claims, &selected);
    assert_eq!(
        profile,
        BTreeMap::from([
            (
                "email".to_owned(),
                ProfileClaim::String("ada@example.com".to_owned())
            ),
            ("email_verified".to_owned(), ProfileClaim::Bool(true)),
        ])
    );
    assert_eq!(profile["email"].as_str(), Some("ada@example.com"));
    assert_eq!(profile["email_verified"].as_bool(), Some(true));
}

mod stale_keys {
    use std::{sync::Arc, time::Duration};

    use baukit_resilience::ManualClock;
    use baukit_test::MockOidcServer;

    use super::*;

    const CONCURRENT_VERIFICATIONS: usize = 100;
    const FRESH_FOR: Duration = Duration::from_secs(300);
    const STALE_FOR: Duration = Duration::from_secs(3600);
    const HUNG_JWKS: Duration = Duration::from_secs(30);
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
    const NO_WAIT: Duration = Duration::from_millis(500);

    async fn wait_until(condition: impl Fn() -> bool) {
        for _ in 0..200 {
            if condition() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("condition not reached");
    }

    #[tokio::test]
    async fn stale_keys_serve_known_kids_while_the_jwks_endpoint_is_down() {
        let server = MockOidcServer::start().await.expect("mock issuer");
        let clock = Arc::new(ManualClock::new());
        let config = OidcConfig::new(server.issuer(), "api")
            .and_then(|config| config.with_request_timeout(REQUEST_TIMEOUT))
            .expect("config");
        let verifier = Arc::new(
            OidcVerifier::from_jwks_uri_with_clock(
                config,
                server.jwks_url(),
                Arc::clone(&clock) as Arc<dyn baukit_resilience::Clock>,
            )
            .expect("verifier"),
        );
        let claims = server
            .claims("user-123", "api", Duration::from_secs(7200))
            .expect("claims");
        let token = server.mint(&claims).expect("token");
        let unknown = server
            .mint_with_key_id(&claims, "rotated-away")
            .expect("unknown kid token");
        verifier.verify(&token).await.expect("first verification");
        assert_eq!(server.jwks_request_count(), 1);

        server.set_jwks_delay(HUNG_JWKS);
        clock.advance(FRESH_FOR + Duration::from_secs(1));
        let tasks: Vec<_> = (0..CONCURRENT_VERIFICATIONS)
            .map(|_| {
                let verifier = Arc::clone(&verifier);
                let token = token.clone();
                tokio::spawn(async move { verifier.verify(&token).await })
            })
            .collect();
        let all = async {
            for task in tasks {
                task.await.expect("task").expect("stale key verifies");
            }
        };
        tokio::time::timeout(NO_WAIT, all)
            .await
            .expect("no verification waits on the hung JWKS fetch");
        wait_until(|| server.jwks_request_count() == 2).await;
        assert_eq!(server.jwks_request_count(), 2, "one refresh attempt");

        let started = std::time::Instant::now();
        assert!(matches!(
            verifier.verify(&unknown).await,
            Err(VerificationError::UnknownKeyId)
        ));
        assert!(started.elapsed() < NO_WAIT, "unknown kid fails fast");

        wait_until(|| !verifier.inner.keys.is_refreshing()).await;
        assert!(matches!(
            verifier.verify(&unknown).await,
            Err(VerificationError::UnknownKeyId)
        ));
        verifier
            .verify(&token)
            .await
            .expect("still stale and usable");
        assert_eq!(server.jwks_request_count(), 2, "cooldown blocks refetching");

        drop(server);
        clock.advance(STALE_FOR);
        for _ in 0..3 {
            let error = verifier
                .verify(&token)
                .await
                .map(|_| ())
                .expect_err("keys expired");
            assert!(error.is_unavailable(), "{error:?}");
            assert!(!error.is_misconfigured(), "{error:?}");
        }
        assert!(matches!(
            verifier.verify(&token).await,
            Err(VerificationError::JwksUnavailable)
        ));
    }
}

mod discovery_retry {
    use std::time::{Duration, Instant};

    use axum::{
        Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::get,
    };
    use serde_json::json;

    use super::*;

    const IDP_COMES_UP_AFTER: Duration = Duration::from_secs(3);

    async fn late_issuer(comes_up_after: Duration) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let issuer = format!("http://{}", listener.local_addr().expect("address"));
        let state = (Instant::now(), issuer.clone());
        let router = Router::new()
            .route(
                "/.well-known/openid-configuration",
                get(
                    move |State((started, issuer)): State<(Instant, String)>| async move {
                        if started.elapsed() < comes_up_after {
                            return StatusCode::SERVICE_UNAVAILABLE.into_response();
                        }
                        Json(json!({"issuer": issuer, "jwks_uri": format!("{issuer}/certs")}))
                            .into_response()
                    },
                ),
            )
            .with_state(state);
        tokio::spawn(async move { axum::serve(listener, router).await });
        issuer
    }

    #[tokio::test]
    async fn discovery_waits_for_an_issuer_that_comes_up_late() {
        let issuer = late_issuer(IDP_COMES_UP_AFTER).await;
        let started = Instant::now();
        let verifier = OidcVerifier::discover(OidcConfig::new(&issuer, "api").expect("config"))
            .await
            .expect("discovery succeeds once the issuer is up");
        assert!(started.elapsed() >= IDP_COMES_UP_AFTER);
        assert_eq!(verifier.issuer(), issuer);
    }

    #[tokio::test]
    async fn discovery_fails_clearly_after_the_startup_budget() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listener");
        let issuer = format!("http://{}", listener.local_addr().expect("address"));
        drop(listener);
        let budget = Duration::from_secs(1);
        let config = OidcConfig::new(&issuer, "api")
            .and_then(|config| config.with_startup_budget(budget))
            .expect("config");
        let started = Instant::now();
        let error = OidcVerifier::discover(config)
            .await
            .map(|_| ())
            .expect_err("issuer never comes up");
        assert!(started.elapsed() >= budget);
        assert!(
            matches!(&error, VerificationError::DiscoveryBudgetExhausted(source) if matches!(**source, VerificationError::DiscoveryRequest(_))),
            "{error:?}"
        );
        assert_eq!(
            error.to_string(),
            "OIDC discovery did not succeed within the startup budget"
        );
    }

    #[tokio::test]
    async fn a_static_jwks_uri_skips_discovery() {
        let config = OidcConfig::new("http://127.0.0.1:9/realms/test", "api")
            .and_then(|config| config.with_jwks_uri("http://127.0.0.1:9/realms/test/certs"))
            .expect("config");
        let verifier = tokio::time::timeout(Duration::from_secs(1), OidcVerifier::discover(config))
            .await
            .expect("no discovery request")
            .expect("verifier");
        assert_eq!(verifier.issuer(), "http://127.0.0.1:9/realms/test");
    }

    #[tokio::test]
    async fn issuer_mismatch_is_not_retried() {
        let issuer = late_issuer(Duration::ZERO).await;
        let config = OidcConfig::new(format!("{issuer}/"), "api").expect("config");
        let started = Instant::now();
        let error = OidcVerifier::discover(config)
            .await
            .map(|_| ())
            .expect_err("mismatch");
        assert!(matches!(error, VerificationError::DiscoveryIssuerMismatch));
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
