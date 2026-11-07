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
        let config = OidcConfig::new(&issuer, "api").expect("config");
        let discovery_error = OidcVerifier::discover(config.clone())
            .await
            .map(|_| ())
            .expect_err("invalid discovery response");
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
            DateTime::from_timestamp(i64::try_from(seconds).expect("valid timestamp"), 0)
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
    let timestamp = DateTime::from_timestamp(i64::try_from(now).expect("valid timestamp"), 0)
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

#[test]
fn unknown_key_cache_has_a_hard_entry_bound() {
    let mut cache = JwksCache::default();
    for index in 0..=MAX_UNKNOWN_KEYS {
        cache.remember_unknown_key(&format!("unknown-{index}"));
    }
    assert_eq!(cache.unknown_keys.len(), MAX_UNKNOWN_KEYS);
    assert!(
        cache
            .unknown_keys
            .contains_key(&unknown_key_hash("unknown-128"))
    );
}
