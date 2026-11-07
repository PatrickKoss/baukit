use std::sync::Arc;

use baukit_auth::{OidcConfig, OidcVerifier, VerificationError};

use super::*;

#[tokio::test]
async fn expired_authorization_codes_are_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let server = MockOidcServer::start().await?;
    let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
    let code = server.issue_authorization_code(
        "user",
        "web",
        "https://app.example/callback",
        challenge,
    )?;
    server
        .state
        .codes
        .write()
        .expect("mock code lock")
        .get_mut(&code)
        .expect("issued code")
        .issued_at -= AUTHORIZATION_CODE_LIFETIME;
    let request = TokenEndpointRequest {
        grant_type: "authorization_code".to_owned(),
        code: Some(code),
        client_id: Some("web".to_owned()),
        redirect_uri: Some("https://app.example/callback".to_owned()),
        code_verifier: Some("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".to_owned()),
        client_secret: None,
        refresh_token: None,
    };
    let response = authorization_code(&server.state, &request);
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await?;
    assert_eq!(
        serde_json::from_slice::<Value>(&body)?["error"],
        "invalid_grant"
    );
    Ok(())
}

#[test]
fn pkce_rejects_invalid_verifier_syntax_even_when_the_hash_matches() {
    for verifier in [
        "a".repeat(MIN_PKCE_VERIFIER_LENGTH - 1),
        "a".repeat(MAX_PKCE_VERIFIER_LENGTH + 1),
        format!("{} ", "a".repeat(MIN_PKCE_VERIFIER_LENGTH)),
        format!("{}é", "a".repeat(MIN_PKCE_VERIFIER_LENGTH)),
    ] {
        let challenge =
            URL_SAFE_NO_PAD.encode(digest::digest(&digest::SHA256, verifier.as_bytes()));
        assert!(!pkce_matches(&verifier, &challenge));
    }
    for length in [MIN_PKCE_VERIFIER_LENGTH, MAX_PKCE_VERIFIER_LENGTH] {
        let verifier = "a".repeat(length);
        let challenge =
            URL_SAFE_NO_PAD.encode(digest::digest(&digest::SHA256, verifier.as_bytes()));
        assert!(pkce_matches(&verifier, &challenge));
    }
}

#[test]
fn creates_hs256_fixture_with_configured_claims() -> Result<(), Box<dyn std::error::Error>> {
    let claims = JwtClaims::new()
        .subject("user-123")
        .issuer("fixture")
        .audience("api")
        .expires_at(4_102_444_800);
    let token = hs256_token(b"fixture-secret", &claims)?;
    assert_eq!(token.split('.').count(), 3);
    assert_eq!(
        authorization_header(&token)?.to_str().expect("header text"),
        format!("Bearer {token}")
    );
    Ok(())
}

#[tokio::test]
async fn mock_server_discovers_verifies_caches_and_rotates()
-> Result<(), Box<dyn std::error::Error>> {
    let server = MockOidcServer::start().await?;
    let verifier = OidcVerifier::discover(OidcConfig::new(server.issuer(), "api")?).await?;
    let claims = server.claims("user-123", "api", Duration::from_secs(60))?;
    assert_eq!(
        verifier.verify(&server.mint(&claims)?).await?.subject(),
        "user-123"
    );
    assert_eq!(
        verifier.verify(&server.mint(&claims)?).await?.subject(),
        "user-123"
    );
    assert_eq!(server.jwks_request_count(), 1);

    server.rotate_signing_key();
    assert_eq!(
        verifier.verify(&server.mint(&claims)?).await?.subject(),
        "user-123"
    );
    assert_eq!(server.jwks_request_count(), 2);
    Ok(())
}

#[tokio::test]
async fn mock_server_exercises_jwks_timeout() -> Result<(), Box<dyn std::error::Error>> {
    let server = MockOidcServer::start().await?;
    // The timeout covers discovery too, and discovery is served without delay,
    // so too tight a budget fails discovery instead of the JWKS fetch under test.
    // Measured discovery is under 30 ms, but the margin has to absorb a clock
    // jump, not just slow I/O, so keep it wide and keep the delay well past it.
    server.set_jwks_delay(Duration::from_secs(10));
    let config =
        OidcConfig::new(server.issuer(), "api")?.with_request_timeout(Duration::from_secs(1))?;
    let verifier = OidcVerifier::discover(config).await?;
    let claims = server.claims("user-123", "api", Duration::from_secs(60))?;
    assert!(matches!(
        verifier.verify(&server.mint(&claims)?).await,
        Err(VerificationError::JwksTimeout)
    ));
    Ok(())
}

#[tokio::test]
async fn unknown_key_refresh_is_single_flight_and_negatively_cached()
-> Result<(), Box<dyn std::error::Error>> {
    let server = MockOidcServer::start().await?;
    server.set_jwks_delay(Duration::from_millis(20));
    let verifier =
        Arc::new(OidcVerifier::discover(OidcConfig::new(server.issuer(), "api")?).await?);
    let claims = server.claims("user-123", "api", Duration::from_secs(60))?;
    let token = server.mint_with_key_id(&claims, "unknown-key")?;
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let verifier = Arc::clone(&verifier);
        let token = token.clone();
        tasks.push(tokio::spawn(async move { verifier.verify(&token).await }));
    }
    for task in tasks {
        assert!(matches!(task.await?, Err(VerificationError::UnknownKeyId)));
    }
    assert_eq!(server.jwks_request_count(), 1);

    assert!(matches!(
        verifier.verify(&token).await,
        Err(VerificationError::UnknownKeyId)
    ));
    assert_eq!(server.jwks_request_count(), 1);
    Ok(())
}

#[tokio::test]
async fn jwks_url_serves_verifiers_without_discovery_and_refreshes_after_ttl()
-> Result<(), Box<dyn std::error::Error>> {
    let server = MockOidcServer::start().await?;
    assert!(server.jwks_url().starts_with(server.issuer()));
    let cache_ttl = Duration::from_millis(100);
    let config = OidcConfig::new(server.issuer(), "api")?.with_jwks_cache_ttl(cache_ttl)?;
    let verifier = OidcVerifier::from_jwks_uri(config, server.jwks_url())?;
    let token = server.mint(&server.claims("user-123", "api", Duration::from_secs(60))?)?;

    verifier.verify(&token).await?;
    verifier.verify(&token).await?;
    assert_eq!(server.jwks_request_count(), 1);

    sleep(cache_ttl * 2).await;
    verifier.verify(&token).await?;
    assert_eq!(server.jwks_request_count(), 2);
    Ok(())
}

#[tokio::test]
async fn mint_with_key_id_signs_with_the_active_key() -> Result<(), Box<dyn std::error::Error>> {
    let server = MockOidcServer::start().await?;
    let claims = server.claims("user-123", "api", Duration::from_secs(60))?;
    let token = server.mint_with_key_id(&claims, KEY_1_ID)?;
    assert_eq!(token, server.mint(&claims)?);
    server.rotate_signing_key();
    let rotated = server.mint_with_key_id(&claims, KEY_2_ID)?;
    assert_eq!(rotated, server.mint(&claims)?);
    Ok(())
}

#[tokio::test]
async fn mock_sessions_cover_short_lived_concurrent_rejected_and_revoked_refresh()
-> Result<(), Box<dyn std::error::Error>> {
    let server = Arc::new(MockOidcServer::start().await?);
    let verifier = OidcVerifier::discover(
        OidcConfig::new(server.issuer(), "api")?.with_clock_skew(Duration::ZERO),
    )
    .await?;
    let session = server.issue_session("user-123", "api", Duration::from_secs(1))?;
    assert_eq!(session.expires_in(), Duration::from_secs(1));
    assert_eq!(
        verifier.verify(session.access_token()).await?.subject(),
        "user-123"
    );

    server.set_refresh_delay(Duration::from_millis(20));
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let server = Arc::clone(&server);
        let refresh_token = session.refresh_token().to_owned();
        tasks.push(tokio::spawn(async move {
            server.refresh_session(&refresh_token).await
        }));
    }
    for task in tasks {
        let refreshed = task.await??;
        assert_eq!(
            verifier.verify(refreshed.access_token()).await?.subject(),
            "user-123"
        );
    }
    assert_eq!(server.refresh_request_count(), 8);

    assert!(server.reject_refresh(session.refresh_token()));
    assert!(matches!(
        server.refresh_session(session.refresh_token()).await,
        Err(JwtFixtureError::RefreshRejected { ref code }) if code == "invalid_grant"
    ));

    let revoked = server.issue_session("user-456", "api", Duration::from_secs(60))?;
    assert!(server.revoke_session(revoked.refresh_token()));
    assert!(matches!(
        server.refresh_session(revoked.refresh_token()).await,
        Err(JwtFixtureError::RefreshRejected { ref code }) if code == "invalid_grant"
    ));
    Ok(())
}
