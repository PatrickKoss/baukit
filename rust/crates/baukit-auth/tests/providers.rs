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

#[tokio::test]
async fn provider_verifiers_preserve_the_exact_issuer_text() -> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    for issuer in [
        "https://identity.example",
        "https://identity.example/",
        "https://identity.example/tenant/",
    ] {
        let verifiers: [Box<dyn IdentityVerifier>; 4] = [
            Box::new(baukit_auth::OidcVerifier::from_jwks_uri(
                baukit_auth::OidcConfig::new(issuer, "api")?,
                server.jwks_url(),
            )?),
            Box::new(ClerkVerifier::from_jwks_uri(
                issuer,
                ["https://app.example"],
                server.jwks_url(),
            )?),
            Box::new(WorkOsVerifier::from_jwks_uri(
                issuer,
                "client_app",
                server.jwks_url(),
            )?),
            Box::new(baukit_auth::ClerkOAuthVerifier::from_jwks_uri(
                issuer,
                "client_app",
                server.jwks_url(),
            )?),
        ];
        let claims = server
            .claims("user_123", "api", Duration::from_secs(300))?
            .issuer(issuer)
            .claim("client_id", "client_app");
        let alternate = if issuer.ends_with('/') {
            issuer.trim_end_matches('/').to_owned()
        } else {
            format!("{issuer}/")
        };
        for verifier in verifiers {
            let principal = verifier.verify(&server.mint(&claims)?).await?;
            assert_eq!(principal.issuer(), Some(issuer));
            assert!(matches!(
                verifier
                    .verify(&server.mint(&claims.clone().issuer(&alternate))?)
                    .await,
                Err(VerificationError::WrongIssuer)
            ));
        }
    }
    Ok(())
}

struct DiscoveryServer {
    issuer: String,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl DiscoveryServer {
    async fn start(suffix: &str, jwks_uri: &str) -> Result<Self, Box<dyn Error>> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let issuer = format!("http://{}{suffix}", listener.local_addr()?);
        let metadata = serde_json::json!({"issuer": issuer, "jwks_uri": jwks_uri});
        let router = axum::Router::new().route(
            &format!(
                "{}/.well-known/openid-configuration",
                suffix.trim_end_matches('/')
            ),
            axum::routing::get(move || {
                let metadata = metadata.clone();
                async move { axum::Json(metadata) }
            }),
        );
        let task = tokio::spawn(async move { axum::serve(listener, router).await });
        Ok(Self { issuer, task })
    }
}

impl Drop for DiscoveryServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test]
async fn discovery_requires_exact_issuer_text_including_trailing_slashes()
-> Result<(), Box<dyn Error>> {
    let keys = MockOidcServer::start().await?;
    for suffix in ["", "/", "/tenant/"] {
        let server = DiscoveryServer::start(suffix, keys.jwks_url()).await?;
        let config = baukit_auth::OidcConfig::new(&server.issuer, "api")?;
        let verifier = baukit_auth::OidcVerifier::discover(config).await?;
        let claims = keys
            .claims("user_123", "api", Duration::from_secs(300))?
            .issuer(&server.issuer);
        assert_eq!(
            verifier.verify(&keys.mint(&claims)?).await?.issuer(),
            Some(server.issuer.as_str())
        );
        let alternate = if suffix.ends_with('/') {
            server.issuer.trim_end_matches('/').to_owned()
        } else {
            format!("{}/", server.issuer)
        };
        assert!(matches!(
            baukit_auth::OidcVerifier::discover(baukit_auth::OidcConfig::new(alternate, "api")?)
                .await,
            Err(VerificationError::DiscoveryIssuerMismatch)
        ));
    }
    Ok(())
}
