use std::time::Duration;

use super::claims::{authentication_time, scope_claim, selected_profile_claims};
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
    async fn an_open_oidc_circuit_fails_discovery_fast_as_unavailable() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let issuer = format!("http://{}", listener.local_addr().expect("address"));
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = calls.clone();
        let router = Router::new().route(
            "/.well-known/openid-configuration",
            get(move || async move {
                counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    [(axum::http::header::RETRY_AFTER, "60")],
                )
            }),
        );
        tokio::spawn(async move { axum::serve(listener, router).await });
        let budget = Duration::from_secs(1);
        let config = OidcConfig::new(&issuer, "api")
            .and_then(|config| config.with_startup_budget(budget))
            .expect("config");
        let error = OidcVerifier::discover(config)
            .await
            .map(|_| ())
            .expect_err("the issuer stays down");
        assert!(error.is_unavailable(), "{error:?}");
        assert!(
            matches!(
                &error,
                VerificationError::DiscoveryBudgetExhausted(source)
                    if matches!(&**source, VerificationError::DiscoveryRequest(cause)
                        if matches!(**cause, baukit_egress::EgressError::CircuitOpen { .. }))
            ),
            "{error:?}"
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
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

mod signed {
    use aws_lc_rs::{rand::SystemRandom, signature::Ed25519KeyPair, signature::KeyPair as _};
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use serde_json::{Value, json};

    use crate::{OidcConfig, OidcVerifier, SigningAlgorithm, VerificationError};

    const ISSUER: &str = "https://issuer.example";
    const KID: &str = "ed";

    struct Signer(Ed25519KeyPair);

    impl Signer {
        fn new() -> Self {
            let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).expect("key");
            Self(Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("pair"))
        }

        fn verifier(&self, config: OidcConfig) -> OidcVerifier {
            let x = URL_SAFE_NO_PAD.encode(self.0.public_key().as_ref());
            let jwks = json!({"keys": [{"kty": "OKP", "crv": "Ed25519", "kid": KID, "x": x}]});
            let config = config
                .with_allowed_algorithms([SigningAlgorithm::EdDsa])
                .expect("algorithms");
            OidcVerifier::from_jwks_document(config, &jwks.to_string()).expect("verifier")
        }

        fn token(&self, typ: Option<&str>, claims: &Value) -> String {
            let mut header = json!({"alg": "EdDSA", "kid": KID});
            if let Some(typ) = typ {
                header["typ"] = json!(typ);
            }
            let input = format!(
                "{}.{}",
                URL_SAFE_NO_PAD.encode(header.to_string()),
                URL_SAFE_NO_PAD.encode(claims.to_string())
            );
            let signature = URL_SAFE_NO_PAD.encode(self.0.sign(input.as_bytes()).as_ref());
            format!("{input}.{signature}")
        }
    }

    fn claims(audience: &str) -> Value {
        json!({"sub": "user", "iss": ISSUER, "aud": audience, "exp": 4_102_444_800_u64})
    }

    #[tokio::test]
    async fn required_access_token_type_accepts_only_at_jwt() {
        let signer = Signer::new();
        let config = OidcConfig::new(ISSUER, "api").expect("config");
        let strict = signer.verifier(config.clone().requiring_access_token_type());
        for typ in ["at+jwt", "application/at+jwt", "AT+JWT"] {
            let token = signer.token(Some(typ), &claims("api"));
            assert!(strict.verify(&token).await.is_ok(), "{typ}");
        }
        for typ in [Some("JWT"), None, Some("application/jwt")] {
            let token = signer.token(typ, &claims("api"));
            assert!(matches!(
                strict.verify(&token).await,
                Err(VerificationError::WrongTokenType)
            ));
        }
        let lenient = signer.verifier(config);
        assert!(
            lenient
                .verify(&signer.token(Some("JWT"), &claims("api")))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn id_tokens_with_a_nonce_are_rejected_as_access_tokens() {
        let signer = Signer::new();
        let config = OidcConfig::new(ISSUER, "web")
            .expect("config")
            .with_client_id("web")
            .expect("client");
        let verifier = signer.verifier(config);
        let mut id_token = claims("web");
        id_token["nonce"] = json!("login-nonce");
        let token = signer.token(Some("JWT"), &id_token);
        assert!(matches!(
            verifier.verify(&token).await,
            Err(VerificationError::WrongTokenType)
        ));
        assert!(
            verifier
                .verify_id_token(&token, "login-nonce")
                .await
                .is_ok()
        );
    }
}
