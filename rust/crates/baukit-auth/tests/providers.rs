use baukit_auth::{ClerkVerifier, IdentityVerifier, VerificationError, WorkOsVerifier};
use baukit_test::MockOidcServer;
use std::{error::Error, time::Duration};

#[tokio::test]
async fn provider_sessions_check_issuer_audience_expiry_and_identity_context()
-> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    let clerk =
        ClerkVerifier::from_jwks_uri(server.issuer(), ["https://app.example"], server.jwks_url())?
            .with_audiences(["api"])?;
    let workos = WorkOsVerifier::from_jwks_uri(server.issuer(), "client_app", server.jwks_url())?
        .with_audiences(["api"])?;
    let base = server.claims("user_123", "api", Duration::from_secs(300))?;
    for (verifier, claims) in [
        (
            &clerk as &dyn IdentityVerifier,
            base.clone()
                .claim("azp", "https://app.example")
                .claim("o", serde_json::json!({"id":"org_123"})),
        ),
        (
            &workos as &dyn IdentityVerifier,
            base.clone()
                .claim("client_id", "client_app")
                .claim("org_id", "org_123"),
        ),
    ] {
        let identity = verifier.verify(&server.mint(&claims)?).await?;
        assert_eq!(identity.subject(), "user_123");
        assert_eq!(identity.organization(), Some("org_123"));
        assert_eq!(identity.issuer(), Some(server.issuer()));
        for (claims, expected) in [
            (
                claims.clone().issuer("https://wrong.example"),
                VerificationError::WrongIssuer,
            ),
            (
                claims.clone().audience("other"),
                VerificationError::WrongAudience,
            ),
            (claims.clone().expires_at(0), VerificationError::Expired),
        ] {
            let error = verifier
                .verify(&server.mint(&claims)?)
                .await
                .expect_err("invalid token accepted");
            assert_eq!(
                std::mem::discriminant(&error),
                std::mem::discriminant(&expected)
            );
        }
    }
    assert!(matches!(
        clerk
            .verify(&server.mint(&base.clone().claim("azp", "https://evil.example"))?)
            .await,
        Err(VerificationError::WrongAuthorizedParty)
    ));
    assert!(matches!(
        workos
            .verify(&server.mint(&base.claim("client_id", "other"))?)
            .await,
        Err(VerificationError::WrongClientId)
    ));
    Ok(())
}

#[tokio::test]
async fn clerk_oauth_requires_the_dedicated_client_and_rejects_session_tokens()
-> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    let verifier = baukit_auth::ClerkOAuthVerifier::from_jwks_uri(
        server.issuer(),
        "client_mcp",
        server.jwks_url(),
    )?;
    let claims = server
        .claims("user_123", "client_mcp", Duration::from_secs(300))?
        .claim("client_id", "client_mcp")
        .claim("scope", "items:read");
    let principal = verifier.verify(&server.mint(&claims)?).await?;
    assert_eq!(principal.client_id(), Some("client_mcp"));
    assert!(principal.scopes().contains("items:read"));
    assert!(matches!(
        verifier
            .verify(&server.mint(&claims.clone().claim("client_id", "client_another"))?)
            .await,
        Err(VerificationError::WrongClientId)
    ));
    assert!(matches!(
        verifier
            .verify(
                &server.mint(
                    &server
                        .claims("user_123", "client_mcp", Duration::from_secs(300))?
                        .claim("azp", "https://app.example")
                )?
            )
            .await,
        Err(VerificationError::WrongClientId)
    ));
    for invalid in [
        claims.clone().issuer("https://another.example"),
        claims.expires_at(0),
    ] {
        assert!(verifier.verify(&server.mint(&invalid)?).await.is_err());
    }
    Ok(())
}
