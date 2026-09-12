use std::{error::Error, time::Duration};

use baukit_auth::{ClerkVerifier, VerificationError, WorkOsVerifier};
use baukit_test::{JwtClaims, MockOidcServer};
use serde_json::json;

fn jwks_uri(server: &MockOidcServer) -> String {
    format!(
        "{}/realms/baukit-test/protocol/openid-connect/certs",
        server.base_url()
    )
}

#[tokio::test]
async fn clerk_adapter_checks_authorized_party_and_maps_v2_organization()
-> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    let verifier = ClerkVerifier::from_jwks_uri(
        server.issuer(),
        ["https://app.example.com", "my-app://callback"],
        jwks_uri(&server),
    )?;
    let claims = JwtClaims::new()
        .subject("user_clerk")
        .issuer(server.issuer())
        .expires_at(expiry()?)
        .claim("azp", "https://app.example.com")
        .claim("o", json!({"id": "org_clerk", "rol": "admin"}));
    let principal = verifier.verify(&server.mint(&claims)?).await?;

    assert_eq!(principal.subject(), "user_clerk");
    assert_eq!(principal.organization(), Some("org_clerk"));
    assert_eq!(principal.client_id(), None);

    let wrong_party = claims.clone().claim("azp", "https://attacker.example");
    assert!(matches!(
        verifier.verify(&server.mint(&wrong_party)?).await,
        Err(VerificationError::WrongAuthorizedParty)
    ));
    Ok(())
}

#[tokio::test]
async fn clerk_adapter_accepts_tokens_without_an_authorized_party() -> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    let verifier = ClerkVerifier::from_jwks_uri(
        server.issuer(),
        ["https://app.example.com"],
        jwks_uri(&server),
    )?;
    let claims = JwtClaims::new()
        .subject("native_user")
        .issuer(server.issuer())
        .expires_at(expiry()?);

    assert_eq!(
        verifier.verify(&server.mint(&claims)?).await?.subject(),
        "native_user"
    );
    Ok(())
}

#[tokio::test]
async fn workos_adapter_requires_client_and_maps_organization() -> Result<(), Box<dyn Error>> {
    let server = MockOidcServer::start().await?;
    let verifier =
        WorkOsVerifier::from_jwks_uri(server.issuer(), "client_01ABC", jwks_uri(&server))?;
    let claims = JwtClaims::new()
        .subject("user_workos")
        .issuer(server.issuer())
        .expires_at(expiry()?)
        .claim("client_id", "client_01ABC")
        .claim("org_id", "org_01ABC");
    let principal = verifier.verify(&server.mint(&claims)?).await?;

    assert_eq!(principal.subject(), "user_workos");
    assert_eq!(principal.organization(), Some("org_01ABC"));
    assert_eq!(principal.client_id(), Some("client_01ABC"));

    for claims in [
        JwtClaims::new()
            .subject("missing_client")
            .issuer(server.issuer())
            .expires_at(expiry()?),
        claims.claim("client_id", "client_wrong"),
    ] {
        assert!(matches!(
            verifier.verify(&server.mint(&claims)?).await,
            Err(VerificationError::WrongClientId)
        ));
    }
    Ok(())
}

fn expiry() -> Result<u64, std::time::SystemTimeError> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .saturating_add(Duration::from_secs(300))
        .as_secs())
}
